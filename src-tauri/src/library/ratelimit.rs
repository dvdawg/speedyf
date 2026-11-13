//! Per-host pacing for everything the lookup chain sends (projects.md §4).
//!
//! Every lookup service publishes a rate limit, and a project-wide lookup
//! would blow through any of them if it fired its requests at once. So
//! requests are queued per host: one in flight per host, each started no
//! sooner than that host's minimum interval after the last
//! ([`minimum_interval`]), and when a host answers 429 or 503 the queue
//! pauses — for what its `Retry-After` says, or without one for a backoff
//! that starts at 2 s and doubles to a 120 s cap until the next success
//! resets it. Hosts never wait on each other: a slow arXiv does not hold up
//! Crossref.
//!
//! Nothing here chooses what to send, and nothing is sent that a caller did
//! not build; [`HostLimiter::send`] is the one way a request leaves. It
//! retries once, after the pause, on 429, 503, any other 5xx, or a timeout,
//! and never on another 4xx — a 404 is an answer, not a fault.
//!
//! The clock is injected ([`Clock`]) so the tests wait on a fake one rather
//! than sleep.

use crate::errors::{AppError, AppResult};
use parking_lot::Mutex;
use reqwest::header::RETRY_AFTER;
use reqwest::StatusCode;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Spacing for a host the table below has no row for.
const DEFAULT_INTERVAL: Duration = Duration::from_millis(1000);
/// The first pause after a 429/503 without a `Retry-After`; doubles each
/// time the host says so again.
const BACKOFF_START: Duration = Duration::from_secs(2);
/// The longest pause, from the backoff or from a `Retry-After`. A service
/// that wants more than two minutes is one to leave alone for now rather
/// than hold a lookup open for.
const BACKOFF_CAP: Duration = Duration::from_secs(120);
/// A retry after a server fault or timeout starts no sooner than this,
/// whatever the host's interval: a server that just fell over wants a
/// breath, not another request 100 ms later.
const RETRY_GRACE: Duration = Duration::from_secs(2);
/// The Semantic Scholar key header; its presence picks the keyed interval.
const S2_KEY_HEADER: &str = "x-api-key";

/// The least time between two requests to `host`, from each service's
/// published or customary limit. `keyed` is whether the request carries an
/// API key, which only Semantic Scholar honours.
///
/// | host                     | interval                 |
/// | ------------------------ | ------------------------ |
/// | api.crossref.org         | 100 ms                   |
/// | api.openalex.org         | 100 ms                   |
/// | api.semanticscholar.org  | 1 s with a key, 3 s without |
/// | export.arxiv.org         | 3 s (their terms ask for one request per 3 s) |
/// | inspirehep.net           | 500 ms                   |
/// | dblp.org                 | 1 s                      |
/// | api.datacite.org         | 100 ms                   |
/// | openlibrary.org          | 1 s                      |
/// | anything else            | 1 s                      |
pub fn minimum_interval(host: &str, keyed: bool) -> Duration {
    let millis = match host.to_ascii_lowercase().as_str() {
        "api.crossref.org" => 100,
        "api.openalex.org" => 100,
        "api.semanticscholar.org" => {
            if keyed {
                1000
            } else {
                3000
            }
        }
        "export.arxiv.org" => 3000,
        "inspirehep.net" => 500,
        "dblp.org" => 1000,
        "api.datacite.org" => 100,
        "openlibrary.org" => 1000,
        _ => return DEFAULT_INTERVAL,
    };
    Duration::from_millis(millis)
}

/// The name a status line or error uses for a host: "Crossref" rather than
/// "api.crossref.org". An unknown host is named as is.
pub fn service_name(host: &str) -> String {
    match host.to_ascii_lowercase().as_str() {
        "api.crossref.org" => "Crossref",
        "api.openalex.org" => "OpenAlex",
        "api.semanticscholar.org" => "Semantic Scholar",
        "export.arxiv.org" => "arXiv",
        "inspirehep.net" => "INSPIRE",
        "dblp.org" => "dblp",
        "api.datacite.org" => "DataCite",
        "openlibrary.org" => "Open Library",
        other => other,
    }
    .to_string()
}

/// The pause after the `strikes`-th consecutive 429/503 that came without a
/// `Retry-After`: 2 s, 4 s, 8 s, … capped at 120 s.
pub fn backoff(strikes: u32) -> Duration {
    let doublings = strikes.saturating_sub(1).min(6);
    (BACKOFF_START * (1 << doublings)).min(BACKOFF_CAP)
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Where the limiter reads the time and how it waits. The real one is
/// tokio's; the tests install one that jumps forward instead of sleeping.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// `Instant::now` and `tokio::time::sleep_until`.
struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep_until(tokio::time::Instant::from_std(
            deadline,
        )))
    }
}

// ---------------------------------------------------------------------------
// The limiter
// ---------------------------------------------------------------------------

/// One host's pacing state. The slot is a tokio mutex so a request waiting
/// for its turn parks the task rather than a thread.
#[derive(Default)]
struct HostEntry {
    /// Held by the request in flight; the next request waits on it.
    slot: Arc<tokio::sync::Mutex<()>>,
    /// The earliest instant the next request may start. `None` is now.
    next_allowed: Option<Instant>,
    /// 429/503 answers since the last success; sizes the backoff.
    strikes: u32,
}

/// Paces requests per host. One per process, shared by every lookup; cheap
/// to hold behind an `Arc` or in the engine's shared state.
pub struct HostLimiter {
    clock: Arc<dyn Clock>,
    hosts: Mutex<HashMap<String, HostEntry>>,
}

/// The right to have one request in flight to a host. Dropping it frees the
/// slot; the spacing to the next request is the host's own and is not
/// affected by how long the permit was held.
#[must_use = "the host's slot is held until the permit drops"]
pub struct Permit {
    host: String,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl Permit {
    #[cfg_attr(not(test), allow(dead_code))] // the tests read it
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// What one attempt of a paced operation came to, for [`HostLimiter::run`].
pub enum Attempt<T> {
    /// An answer to hand back, whatever it says.
    Done(T),
    /// A failure worth one more try after a pause: a rate limit, a server
    /// fault, a timeout. The attempt has already told the limiter about any
    /// status it saw.
    Again(AppError),
    /// A failure a retry would only repeat.
    Fail(AppError),
}

impl Default for HostLimiter {
    fn default() -> Self {
        Self::with_clock(Arc::new(TokioClock))
    }
}

impl HostLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            hosts: Mutex::new(HashMap::new()),
        }
    }

    /// Waits for the host's slot and then for its next allowed instant, and
    /// takes both. `keyed` is whether the request carries an API key (see
    /// [`minimum_interval`]). Hosts are compared case-insensitively.
    pub async fn acquire(&self, host: &str, keyed: bool) -> Permit {
        let host = host.to_ascii_lowercase();
        let slot = self.with_entry(&host, |entry| entry.slot.clone());
        let guard = slot.lock_owned().await;
        self.pace(&host, keyed).await;
        Permit {
            host,
            _guard: guard,
        }
    }

    /// Records what a host answered. A 2xx clears its strikes. A 429 or 503
    /// holds the host for `retry_after` (the header, parsed by the caller)
    /// or, without one, for the backoff; either is capped at two minutes
    /// and never shortens a hold already in place. Other statuses change
    /// nothing.
    pub fn note_response(&self, host: &str, status: u16, retry_after: Option<Duration>) {
        let now = self.clock.now();
        self.with_entry(&host.to_ascii_lowercase(), |entry| match status {
            200..=299 => entry.strikes = 0,
            429 | 503 => {
                entry.strikes = entry.strikes.saturating_add(1);
                let pause = retry_after
                    .unwrap_or_else(|| backoff(entry.strikes))
                    .min(BACKOFF_CAP);
                entry.hold_until(now + pause);
            }
            _ => {}
        });
    }

    /// How long a request to `host` would wait for its turn, not counting a
    /// request already in flight. Zero when the host is free. For a status
    /// line that wants to say "waiting for Crossref".
    pub fn wait_for(&self, host: &str) -> Duration {
        let now = self.clock.now();
        let hosts = self.hosts.lock();
        hosts
            .get(&host.to_ascii_lowercase())
            .and_then(|entry| entry.next_allowed)
            .map_or(Duration::ZERO, |at| at.saturating_duration_since(now))
    }

    /// The one way a request leaves: waits for the host's turn, sends, tells
    /// the limiter what came back, and on a 429, 503, other 5xx or timeout
    /// retries once after the pause. Any other answer — 2xx, 3xx, 404, a
    /// 403 — comes back as the response for the caller to read; a second
    /// retryable failure, or a transport failure other than a timeout, is an
    /// error naming the service.
    pub async fn send(
        &self,
        client: &reqwest::Client,
        request: reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        let request = request
            .build()
            .map_err(|error| AppError::Io(format!("cannot build request: {error}")))?;
        let host = request
            .url()
            .host_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let keyed = request.headers().contains_key(S2_KEY_HEADER);
        let service = service_name(&host);
        let (host, service, request) = (host.as_str(), service.as_str(), &request);
        self.run(host, keyed, || async move {
            let Some(attempt) = request.try_clone() else {
                return Attempt::Fail(AppError::Internal(format!(
                    "{service}: a streaming request cannot be paced"
                )));
            };
            match client.execute(attempt).await {
                Ok(response) => {
                    let status = response.status();
                    self.note_response(host, status.as_u16(), retry_after_header(&response));
                    if is_retryable(status) {
                        Attempt::Again(AppError::Io(format!("{service} answered HTTP {status}")))
                    } else {
                        Attempt::Done(response)
                    }
                }
                Err(error) if error.is_timeout() => Attempt::Again(network_error(service, &error)),
                Err(error) => Attempt::Fail(network_error(service, &error)),
            }
        })
        .await
    }

    /// Runs `attempt` under the host's permit: once, and once more after the
    /// pause when the first came back [`Attempt::Again`]. The second
    /// `Again` is the error returned. [`send`](Self::send) is this with an
    /// HTTP request as the attempt; it is separate so the retry rule is
    /// tested without a network.
    pub async fn run<T, F, Fut>(&self, host: &str, keyed: bool, mut attempt: F) -> AppResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Attempt<T>>,
    {
        let host = host.to_ascii_lowercase();
        let _permit = self.acquire(&host, keyed).await;
        let mut tries = 0;
        loop {
            tries += 1;
            match attempt().await {
                Attempt::Done(value) => return Ok(value),
                Attempt::Fail(error) => return Err(error),
                Attempt::Again(error) => {
                    if tries >= 2 {
                        return Err(error);
                    }
                    self.hold(&host, RETRY_GRACE);
                    self.pace(&host, keyed).await;
                }
            }
        }
    }

    /// Waits until the host's next allowed instant and claims the interval
    /// after it. Called with the host's slot held, so nothing else moves the
    /// instant meanwhile; the loop is for the day that changes.
    async fn pace(&self, host: &str, keyed: bool) {
        loop {
            let now = self.clock.now();
            let deadline = self.with_entry(host, |entry| match entry.next_allowed {
                Some(at) if at > now => Some(at),
                _ => {
                    entry.next_allowed = Some(now + minimum_interval(host, keyed));
                    None
                }
            });
            match deadline {
                Some(at) => self.clock.sleep_until(at).await,
                None => return,
            }
        }
    }

    /// Pushes the host's next allowed instant out to at least `now + pause`.
    fn hold(&self, host: &str, pause: Duration) {
        let until = self.clock.now() + pause;
        self.with_entry(host, |entry| entry.hold_until(until));
    }

    fn with_entry<R>(&self, host: &str, with: impl FnOnce(&mut HostEntry) -> R) -> R {
        let mut hosts = self.hosts.lock();
        with(hosts.entry(host.to_string()).or_default())
    }
}

impl HostEntry {
    fn hold_until(&mut self, until: Instant) {
        self.next_allowed = Some(self.next_allowed.map_or(until, |at| at.max(until)));
    }
}

/// 429, 503 and every other 5xx. Never another 4xx.
fn is_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

pub(crate) fn network_error(service: &str, error: &reqwest::Error) -> AppError {
    let what = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "is unreachable"
    } else {
        "request failed"
    };
    AppError::Io(format!("{service} {what}: {error}"))
}

// ---------------------------------------------------------------------------
// Retry-After
// ---------------------------------------------------------------------------

fn retry_after_header(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    parse_retry_after(value, SystemTime::now())
}

/// A `Retry-After` value as a wait from `now`: delta-seconds (`120`) or an
/// HTTP-date (`Wed, 21 Oct 2015 07:28:00 GMT`), the latter saturating at
/// zero when already past. `None` for anything else.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = parse_http_date(value)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(at.saturating_sub(now)))
}

/// Unix seconds for an HTTP-date in any of RFC 9110's three forms:
/// IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`), RFC 850
/// (`Sunday, 06-Nov-94 08:49:37 GMT`) and asctime
/// (`Sun Nov  6 08:49:37 1994`). The weekday is not checked.
fn parse_http_date(text: &str) -> Option<u64> {
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == '-')
        .filter(|token| !token.is_empty())
        .collect();
    let month_at = tokens
        .iter()
        .position(|token| month_number(token).is_some())?;
    let month = month_number(tokens[month_at])?;
    let (day, year) = match month_at {
        // `Sun 06 Nov 1994 …` and `Sunday 06 Nov 94 …`
        2 => (tokens.get(1)?, tokens.get(3)?),
        // `Sun Nov 6 08:49:37 1994`
        1 => (tokens.get(2)?, tokens.last()?),
        _ => return None,
    };
    let day: u32 = day.parse().ok()?;
    let mut year: i64 = year.parse().ok()?;
    if year < 100 {
        // RFC 850's two-digit year, read as the spec suggests: the nearest.
        year += if year < 70 { 2000 } else { 1900 };
    }
    let time = tokens.iter().find(|token| token.contains(':'))?;
    let mut parts = time.split(':').map(|part| part.parse::<u64>().ok());
    let (hour, minute, second) = (parts.next()??, parts.next()??, parts.next()??);
    // A server-controlled number: a year that would overflow the day
    // arithmetic is not a date, whatever it is.
    if !(1970..=9999).contains(&year)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let days = u64::try_from(days).ok()?;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn month_number(token: &str) -> Option<u32> {
    let month = match token.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    Some(month)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

// ---------------------------------------------------------------------------
// Tests — no network, no sleeping. The clock is a counter the limiter's own
// waits advance.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::task::{Context, Poll, Wake, Waker};

    /// Time that moves only when something waits on it.
    struct FakeClock {
        now: Mutex<Instant>,
        slept: Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(Instant::now()),
                slept: Mutex::new(Vec::new()),
            })
        }

        fn advance(&self, by: Duration) {
            *self.now.lock() += by;
        }

        fn slept(&self) -> Vec<Duration> {
            self.slept.lock().clone()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock()
        }

        fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            let mut now = self.now.lock();
            if deadline > *now {
                self.slept.lock().push(deadline - *now);
                *now = deadline;
            }
            Box::pin(std::future::ready(()))
        }
    }

    fn fresh() -> (Arc<FakeClock>, HostLimiter) {
        let clock = FakeClock::new();
        let limiter = HostLimiter::with_clock(clock.clone());
        (clock, limiter)
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    /// Polls once, with a waker that does nothing: the answer is whether the
    /// future is blocked right now.
    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        struct Noop;
        impl Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        let waker = Waker::from(Arc::new(Noop));
        future.poll(&mut Context::from_waker(&waker))
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;
    const SECS: fn(u64) -> Duration = Duration::from_secs;

    // -- the table --------------------------------------------------------

    #[test]
    fn the_interval_table() {
        assert_eq!(minimum_interval("api.crossref.org", false), MS(100));
        assert_eq!(minimum_interval("api.openalex.org", false), MS(100));
        assert_eq!(minimum_interval("api.semanticscholar.org", true), MS(1000));
        assert_eq!(minimum_interval("api.semanticscholar.org", false), MS(3000));
        assert_eq!(minimum_interval("export.arxiv.org", false), MS(3000));
        assert_eq!(minimum_interval("inspirehep.net", false), MS(500));
        assert_eq!(minimum_interval("dblp.org", false), MS(1000));
        assert_eq!(minimum_interval("api.datacite.org", false), MS(100));
        assert_eq!(minimum_interval("openlibrary.org", false), MS(1000));
        assert_eq!(minimum_interval("example.org", false), MS(1000));
        assert_eq!(
            minimum_interval("API.Crossref.ORG", true),
            MS(100),
            "case-insensitive"
        );
    }

    #[test]
    fn service_names() {
        assert_eq!(service_name("api.crossref.org"), "Crossref");
        assert_eq!(service_name("export.arxiv.org"), "arXiv");
        assert_eq!(service_name("inspirehep.net"), "INSPIRE");
        assert_eq!(service_name("mirror.example.org"), "mirror.example.org");
    }

    #[test]
    fn backoff_doubles_from_two_seconds_to_the_cap() {
        assert_eq!(backoff(0), SECS(2));
        assert_eq!(backoff(1), SECS(2));
        assert_eq!(backoff(2), SECS(4));
        assert_eq!(backoff(3), SECS(8));
        assert_eq!(backoff(6), SECS(64));
        assert_eq!(backoff(7), SECS(120));
        assert_eq!(backoff(40), SECS(120));
    }

    // -- spacing ----------------------------------------------------------

    #[test]
    fn two_requests_to_one_host_are_spaced_by_its_interval() {
        let (clock, limiter) = fresh();
        let start = clock.now();
        let first = block_on(limiter.acquire("api.crossref.org", false));
        assert_eq!(clock.now(), start, "the first request goes at once");
        assert_eq!(limiter.wait_for("api.crossref.org"), MS(100));
        drop(first);

        let _second = block_on(limiter.acquire("api.crossref.org", false));
        assert_eq!(clock.now() - start, MS(100));
        assert_eq!(clock.slept(), vec![MS(100)]);
    }

    #[test]
    fn a_long_request_does_not_shorten_the_spacing_after_it() {
        // The interval is measured from one request's start to the next;
        // a request that itself took longer than the interval frees the
        // host at once.
        let (clock, limiter) = fresh();
        let first = block_on(limiter.acquire("export.arxiv.org", false));
        clock.advance(SECS(5));
        drop(first);
        let before = clock.now();
        let _second = block_on(limiter.acquire("export.arxiv.org", false));
        assert_eq!(clock.now(), before);
        assert!(clock.slept().is_empty());
    }

    #[test]
    fn the_keyed_interval_applies_to_semantic_scholar_only() {
        let (clock, limiter) = fresh();
        drop(block_on(limiter.acquire("api.semanticscholar.org", true)));
        drop(block_on(limiter.acquire("api.semanticscholar.org", true)));
        assert_eq!(clock.slept(), vec![SECS(1)]);

        let (clock, limiter) = fresh();
        drop(block_on(limiter.acquire("api.semanticscholar.org", false)));
        drop(block_on(limiter.acquire("api.semanticscholar.org", false)));
        assert_eq!(clock.slept(), vec![SECS(3)]);

        let (clock, limiter) = fresh();
        drop(block_on(limiter.acquire("dblp.org", true)));
        drop(block_on(limiter.acquire("dblp.org", true)));
        assert_eq!(clock.slept(), vec![SECS(1)]);
    }

    #[test]
    fn hosts_do_not_wait_on_each_other() {
        let (clock, limiter) = fresh();
        let _arxiv = block_on(limiter.acquire("export.arxiv.org", false));
        // arXiv's slot is held and its next instant is 3 s out; Crossref
        // neither waits for the slot nor sleeps.
        let before = clock.now();
        let crossref = std::pin::pin!(limiter.acquire("api.crossref.org", false));
        assert!(matches!(poll_once(crossref), Poll::Ready(_)));
        assert_eq!(clock.now(), before);
        assert!(clock.slept().is_empty());
    }

    #[test]
    fn one_request_in_flight_per_host() {
        let (clock, limiter) = fresh();
        let first = block_on(limiter.acquire("dblp.org", false));
        let mut second = std::pin::pin!(limiter.acquire("dblp.org", false));
        assert!(
            poll_once(second.as_mut()).is_pending(),
            "the slot is held by the first request"
        );
        assert!(
            clock.slept().is_empty(),
            "waiting for the slot is not a sleep"
        );
        drop(first);
        let permit = match poll_once(second.as_mut()) {
            Poll::Ready(permit) => permit,
            Poll::Pending => panic!("the slot was freed"),
        };
        assert_eq!(permit.host(), "dblp.org");
        assert_eq!(clock.slept(), vec![SECS(1)], "then the interval applies");
    }

    #[test]
    fn hosts_are_compared_case_insensitively() {
        let (clock, limiter) = fresh();
        drop(block_on(limiter.acquire("API.CrossRef.org", false)));
        drop(block_on(limiter.acquire("api.crossref.org", false)));
        assert_eq!(clock.slept(), vec![MS(100)]);
        assert_eq!(limiter.wait_for("Api.Crossref.Org"), MS(100));
    }

    // -- 429 / 503 --------------------------------------------------------

    #[test]
    fn a_429_with_retry_after_holds_the_host_for_that_long() {
        let (clock, limiter) = fresh();
        let permit = block_on(limiter.acquire("api.openalex.org", false));
        limiter.note_response("api.openalex.org", 429, Some(SECS(7)));
        drop(permit);
        assert_eq!(limiter.wait_for("api.openalex.org"), SECS(7));
        let before = clock.now();
        drop(block_on(limiter.acquire("api.openalex.org", false)));
        assert_eq!(clock.now() - before, SECS(7));
        assert_eq!(
            limiter.wait_for("api.crossref.org"),
            Duration::ZERO,
            "another host is untouched"
        );
    }

    #[test]
    fn a_503_without_retry_after_backs_off_doubling_and_a_success_resets_it() {
        let (clock, limiter) = fresh();
        let host = "inspirehep.net";
        let expected = [2, 4, 8, 16, 32, 64, 120, 120];
        for seconds in expected {
            limiter.note_response(host, 503, None);
            assert_eq!(limiter.wait_for(host), SECS(seconds));
            clock.advance(SECS(seconds));
        }
        limiter.note_response(host, 200, None);
        limiter.note_response(host, 429, None);
        assert_eq!(limiter.wait_for(host), SECS(2), "back to the start");
    }

    #[test]
    fn a_hold_never_shortens_and_other_statuses_change_nothing() {
        let (_clock, limiter) = fresh();
        let host = "api.datacite.org";
        limiter.note_response(host, 429, Some(SECS(30)));
        limiter.note_response(host, 429, Some(SECS(5)));
        assert_eq!(limiter.wait_for(host), SECS(30));
        limiter.note_response(host, 404, None);
        limiter.note_response(host, 500, None);
        assert_eq!(limiter.wait_for(host), SECS(30));
        limiter.note_response(host, 429, Some(SECS(100_000)));
        assert_eq!(limiter.wait_for(host), SECS(120), "capped");
    }

    #[test]
    fn retry_after_parses_seconds_and_http_dates() {
        let now = UNIX_EPOCH + SECS(784_111_777 - 60);
        assert_eq!(parse_retry_after("120", now), Some(SECS(120)));
        assert_eq!(parse_retry_after(" 0 ", now), Some(SECS(0)));
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now),
            Some(SECS(60))
        );
        assert_eq!(
            parse_retry_after("Sunday, 06-Nov-94 08:49:37 GMT", now),
            Some(SECS(60))
        );
        assert_eq!(
            parse_retry_after("Sun Nov  6 08:49:37 1994", now),
            Some(SECS(60))
        );
        let later = UNIX_EPOCH + SECS(784_111_777 + 60);
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", later),
            Some(SECS(0)),
            "a date already past is no wait"
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("", now), None);
        assert_eq!(
            parse_retry_after("Sun, 32 Nov 1994 08:49:37 GMT", now),
            None
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
    }

    #[test]
    fn an_absurd_year_in_retry_after_is_no_date_and_no_panic() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for text in [
            "Sun, 06 Nov 99999999999999 08:49:37 GMT",
            "Sun Nov  6 08:49:37 99999999999999",
            "Sunday, 06-Nov-99999999999999 08:49:37 GMT",
            "Sun, 06 Nov 10000 08:49:37 GMT",
            "Sun, 06 Nov 1969 08:49:37 GMT",
        ] {
            assert_eq!(parse_retry_after(text, now), None, "{text}");
        }
        assert_eq!(
            parse_retry_after("Fri, 31 Dec 9999 23:59:59 GMT", now).map(|d| d.as_secs()),
            Some(253_402_300_799 - 1_700_000_000),
            "the last year allowed still parses"
        );
    }

    // -- retry once -------------------------------------------------------

    #[test]
    fn run_retries_once_after_the_pause_then_gives_up() {
        let (clock, limiter) = fresh();
        let calls = Cell::new(0);
        let start = clock.now();
        let result: AppResult<()> = block_on(limiter.run("api.crossref.org", false, || {
            calls.set(calls.get() + 1);
            // What `send` does on a 429: tell the limiter, then ask again.
            limiter.note_response("api.crossref.org", 429, None);
            async { Attempt::Again(AppError::Io("Crossref answered HTTP 429".into())) }
        }));
        assert_eq!(calls.get(), 2);
        let error = result.expect_err("the second failure is the error");
        assert!(
            matches!(&error, AppError::Io(message) if message == "Crossref answered HTTP 429"),
            "{error}"
        );
        // The first strike's backoff (2 s) was waited before the retry.
        assert_eq!(clock.now() - start, SECS(2));
        assert_eq!(clock.slept(), vec![SECS(2)]);
    }

    #[test]
    fn run_hands_back_the_second_attempts_answer() {
        let (clock, limiter) = fresh();
        let calls = Cell::new(0);
        let result = block_on(limiter.run("example.org", false, || {
            calls.set(calls.get() + 1);
            let n = calls.get();
            async move {
                if n == 1 {
                    Attempt::Again(AppError::Io("example.org timed out".into()))
                } else {
                    Attempt::Done(n)
                }
            }
        }));
        assert_eq!(result.expect("second try"), 2);
        // A timeout tells the limiter nothing; the retry waits the grace.
        assert_eq!(clock.slept(), vec![RETRY_GRACE]);
    }

    #[test]
    fn run_never_retries_a_plain_failure() {
        let (clock, limiter) = fresh();
        let calls = Cell::new(0);
        let result: AppResult<()> = block_on(limiter.run("example.org", false, || {
            calls.set(calls.get() + 1);
            async { Attempt::Fail(AppError::Io("example.org is unreachable".into())) }
        }));
        assert_eq!(calls.get(), 1);
        assert!(result.is_err());
        assert!(clock.slept().is_empty());
    }

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable(StatusCode::NOT_FOUND));
        assert!(!is_retryable(StatusCode::FORBIDDEN));
        assert!(!is_retryable(StatusCode::BAD_REQUEST));
        assert!(!is_retryable(StatusCode::OK));
    }
}
