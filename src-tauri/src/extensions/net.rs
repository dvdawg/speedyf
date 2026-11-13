//! `net.fetch`: the one way an extension reaches the internet.
//!
//! SpeedyF itself makes no request without a setting the user turned on, and
//! an extension's worker cannot connect anywhere at all — its CSP is
//! `connect-src ipc:`. An extension with the `network` permission asks the
//! core instead, and the core:
//!
//! - accepts `https://` only, on the default port, with no credentials in
//!   the URL;
//! - checks the host against the extension's own manifest list (exact, or
//!   a subdomain of a `*.` entry), and checks every redirect the same way;
//! - sends through the shared per-host [`HostLimiter`], so an extension
//!   cannot burst a service the library lookups also use (an unknown host
//!   gets one request a second);
//! - stops reading at a size cap (5 MB unless asked for less or more, never
//!   past 20 MB), and gives up after 30 seconds.
//!
//! Enabling an extension that declares `network` is the user's consent to
//! this; the Extensions panel lists the hosts on its card, and the status bar
//! says so while a request is in flight.

use super::host_allowed;
use crate::errors::{AppError, AppResult};
use crate::library::ratelimit::HostLimiter;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// What a fetch returns when the caller does not say.
pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The most any fetch may return, whatever it asks for.
pub const HARD_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// The whole request, connect to last byte.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Redirects followed before giving up, each one checked against the list.
const MAX_REDIRECTS: usize = 5;

/// How the body comes back: a string, or bytes as plain numbers (the same
/// convention `render.page` uses, since an ArrayBuffer posted to a worker
/// is transferred rather than copied).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Text,
    Bytes,
}

impl BodyKind {
    pub fn parse(kind: &str) -> AppResult<BodyKind> {
        match kind {
            "text" => Ok(BodyKind::Text),
            "bytes" => Ok(BodyKind::Bytes),
            other => Err(AppError::Malformed(format!(
                "net.fetch: \"as\" must be \"text\" or \"bytes\", not \"{other}\""
            ))),
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum FetchBody {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: FetchBody,
}

/// The cap a request asked for, clamped to what is allowed. Zero and
/// absent both mean the default.
pub fn effective_cap(requested: Option<u64>) -> u64 {
    match requested {
        None | Some(0) => DEFAULT_MAX_BYTES,
        Some(bytes) => bytes.min(HARD_MAX_BYTES),
    }
}

/// Whether a URL's scheme and port are ones `net.fetch` will use. `https`
/// on its default port only; plain `http` to 127.0.0.1 exists in test
/// builds alone, so the tests can run a server without a certificate.
fn scheme_allowed(url: &reqwest::Url) -> bool {
    if cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1") {
        return true;
    }
    url.scheme() == "https" && url.port().is_none()
}

/// Parse and police a URL against an extension's host list. The error says
/// what to fix, naming the host and what is allowed.
pub fn check_url(raw: &str, allowed: &[String]) -> AppResult<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|e| AppError::Malformed(format!("net.fetch: \"{raw}\" is not a URL ({e})")))?;
    if !scheme_allowed(&url) {
        return Err(AppError::Unsupported(format!(
            "net.fetch: only https:// URLs on the default port can be fetched, not \"{raw}\""
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Unsupported(
            "net.fetch: a URL cannot carry a user name or password".into(),
        ));
    }
    let host = url.host_str().unwrap_or_default();
    if !host_allowed(allowed, host) {
        return Err(AppError::Unsupported(format!(
            "net.fetch: \"{host}\" is not in this extension's network list (allowed: {})",
            allowed.join(", ")
        )));
    }
    Ok(url)
}

/// A client whose redirects stay inside the list. Built per extension call
/// because the policy closes over that extension's hosts.
fn client(allowed: Arc<Vec<String>>, user_agent: &str) -> AppResult<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error(format!("more than {MAX_REDIRECTS} redirects"));
        }
        let url = attempt.url().clone();
        let host = url.host_str().unwrap_or_default().to_string();
        if !scheme_allowed(&url) {
            return attempt.error(format!("redirected to a non-https URL ({url})"));
        }
        if !host_allowed(&allowed, &host) {
            return attempt.error(format!(
                "redirected to \"{host}\", which is not in this extension's network list"
            ));
        }
        attempt.follow()
    });
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .gzip(true)
        .user_agent(user_agent)
        .redirect(policy)
        .build()
        .map_err(|e| AppError::Internal(format!("net.fetch: cannot build a client ({e})")))
}

/// A reqwest error with its causes, which is where a redirect refusal's
/// reason lives.
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let line = cause.to_string();
        if !text.contains(&line) {
            text.push_str(": ");
            text.push_str(&line);
        }
        source = cause.source();
    }
    text
}

/// Fetch `raw` for an extension whose manifest allows `allowed`, through the
/// shared limiter. `extension` names it in the User-Agent, so a service can
/// tell an extension's traffic from SpeedyF's own lookups.
pub async fn fetch(
    limiter: &HostLimiter,
    extension: &str,
    allowed: &[String],
    raw: &str,
    kind: BodyKind,
    max_bytes: Option<u64>,
) -> AppResult<FetchResponse> {
    let url = check_url(raw, allowed)?;
    let cap = effective_cap(max_bytes);
    let user_agent = format!(
        "SpeedyF/{} (extension {extension})",
        env!("CARGO_PKG_VERSION")
    );
    let client = client(Arc::new(allowed.to_vec()), &user_agent)?;
    let request = client.get(url.clone());
    let mut response = limiter.send(&client, request).await.map_err(|e| match e {
        AppError::Io(message) => AppError::Io(format!("net.fetch: {message}")),
        other => other,
    })?;

    if response.content_length().is_some_and(|length| length > cap) {
        return Err(AppError::Unsupported(format!(
            "net.fetch: the response is larger than {cap} bytes"
        )));
    }
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut body: Vec<u8> = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|e| {
            AppError::Io(format!(
                "net.fetch: reading {} failed: {}",
                url.host_str().unwrap_or_default(),
                describe(&e)
            ))
        })?;
        let Some(chunk) = chunk else { break };
        if body.len() as u64 + chunk.len() as u64 > cap {
            // Dropping the response closes the connection: nothing past the
            // cap is read.
            return Err(AppError::Unsupported(format!(
                "net.fetch: the response is larger than {cap} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    let body = match kind {
        BodyKind::Text => FetchBody::Text(String::from_utf8_lossy(&body).into_owned()),
        BodyKind::Bytes => FetchBody::Bytes(body),
    };
    Ok(FetchResponse {
        status,
        content_type,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| h.to_string()).collect()
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    /// A one-thread HTTP server on 127.0.0.1 answering each path from
    /// `routes` (path, full response bytes), for `count` connections.
    fn serve(routes: Vec<(&'static str, Vec<u8>)>, count: usize) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(count) {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let response = routes
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_else(|| {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    });
                let _ = stream.write_all(&response);
            }
        });
        port
    }

    fn ok(body: &[u8], content_type: &str) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn redirect(to: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn only_https_on_the_default_port_is_accepted() {
        let allowed = hosts(&["export.arxiv.org"]);
        assert!(check_url("https://export.arxiv.org/abs/1", &allowed).is_ok());
        for refused in [
            "http://export.arxiv.org/abs/1",
            "ftp://export.arxiv.org/x",
            "file:///etc/passwd",
            "https://export.arxiv.org:8443/x",
            "https://user:pw@export.arxiv.org/x",
            "not a url",
        ] {
            assert!(
                check_url(refused, &allowed).is_err(),
                "{refused} was accepted"
            );
        }
    }

    #[test]
    fn hosts_must_be_on_the_list() {
        let allowed = hosts(&["arxiv.org", "*.ebi.ac.uk"]);
        assert!(check_url("https://arxiv.org/e-print/1", &allowed).is_ok());
        assert!(check_url("https://ARXIV.org/e-print/1", &allowed).is_ok());
        assert!(check_url("https://www.ebi.ac.uk/europepmc", &allowed).is_ok());
        let error = check_url("https://evil.example/", &allowed)
            .unwrap_err()
            .to_string();
        assert!(error.contains("\"evil.example\" is not in"), "{error}");
        assert!(error.contains("arxiv.org, *.ebi.ac.uk"), "{error}");
        // The wildcard means subdomains, not the suffix or a lookalike.
        assert!(check_url("https://ebi.ac.uk/", &allowed).is_err());
        assert!(check_url("https://notebi.ac.uk/", &allowed).is_err());
        assert!(check_url("https://export.arxiv.org/", &allowed).is_err());
    }

    #[test]
    fn the_cap_defaults_and_clamps() {
        assert_eq!(effective_cap(None), DEFAULT_MAX_BYTES);
        assert_eq!(effective_cap(Some(0)), DEFAULT_MAX_BYTES);
        assert_eq!(effective_cap(Some(10)), 10);
        assert_eq!(effective_cap(Some(u64::MAX)), HARD_MAX_BYTES);
    }

    #[test]
    fn fetches_text_and_bytes_from_an_allowed_host() {
        let port = serve(
            vec![
                ("/text", ok(b"hello \xce\xb1", "text/plain; charset=utf-8")),
                ("/bytes", ok(&[0, 1, 2, 255], "application/octet-stream")),
            ],
            2,
        );
        let limiter = HostLimiter::new();
        let allowed = hosts(&["127.0.0.1"]);
        let text = block_on(fetch(
            &limiter,
            "dev.speedyf.test",
            &allowed,
            &format!("http://127.0.0.1:{port}/text"),
            BodyKind::Text,
            None,
        ))
        .unwrap();
        assert_eq!(text.status, 200);
        assert_eq!(
            text.content_type.as_deref(),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(text.body, FetchBody::Text("hello \u{3b1}".into()));

        let bytes = block_on(fetch(
            &limiter,
            "dev.speedyf.test",
            &allowed,
            &format!("http://127.0.0.1:{port}/bytes"),
            BodyKind::Bytes,
            None,
        ))
        .unwrap();
        assert_eq!(bytes.body, FetchBody::Bytes(vec![0, 1, 2, 255]));
        // Bytes travel as plain numbers, the protocol's convention.
        assert_eq!(
            serde_json::to_value(&bytes).unwrap()["body"],
            serde_json::json!([0, 1, 2, 255])
        );
    }

    #[test]
    fn a_host_off_the_list_is_refused_before_anything_is_sent() {
        let limiter = HostLimiter::new();
        let error = block_on(fetch(
            &limiter,
            "dev.speedyf.test",
            &hosts(&["arxiv.org"]),
            "http://127.0.0.1:9/x",
            BodyKind::Text,
            None,
        ))
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("not in this extension's network list"),
            "{error}"
        );
    }

    #[test]
    fn a_response_past_the_cap_is_refused() {
        let big = vec![b'x'; 4096];
        let port = serve(vec![("/big", ok(&big, "text/plain"))], 1);
        let error = block_on(fetch(
            &HostLimiter::new(),
            "dev.speedyf.test",
            &hosts(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{port}/big"),
            BodyKind::Text,
            Some(1000),
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("larger than 1000 bytes"), "{error}");
    }

    #[test]
    fn a_streamed_response_past_the_cap_is_cut_off() {
        // No Content-Length: the cap has to be enforced while reading.
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n".to_vec();
        response.extend(vec![b'y'; 8192]);
        let port = serve(vec![("/stream", response)], 1);
        let error = block_on(fetch(
            &HostLimiter::new(),
            "dev.speedyf.test",
            &hosts(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{port}/stream"),
            BodyKind::Bytes,
            Some(100),
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("larger than 100 bytes"), "{error}");
    }

    #[test]
    fn a_redirect_off_the_list_is_refused() {
        let port = serve(vec![("/away", redirect("http://localhost:1/elsewhere"))], 1);
        let error = block_on(fetch(
            &HostLimiter::new(),
            "dev.speedyf.test",
            &hosts(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{port}/away"),
            BodyKind::Text,
            None,
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("redirect"), "{error}");
    }

    #[test]
    fn a_redirect_within_the_list_is_followed() {
        // Two servers on the same allowed host: /hop redirects to /land.
        let port = serve(vec![("/land", ok(b"landed", "text/plain"))], 1);
        let hop = serve(
            vec![("/hop", redirect(&format!("http://127.0.0.1:{port}/land")))],
            1,
        );
        let response = block_on(fetch(
            &HostLimiter::new(),
            "dev.speedyf.test",
            &hosts(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{hop}/hop"),
            BodyKind::Text,
            None,
        ))
        .unwrap();
        assert_eq!(response.body, FetchBody::Text("landed".into()));
    }

    #[test]
    fn a_bad_kind_is_named() {
        assert_eq!(BodyKind::parse("text").unwrap(), BodyKind::Text);
        assert!(BodyKind::parse("json")
            .unwrap_err()
            .to_string()
            .contains("\"text\" or \"bytes\""));
    }
}
