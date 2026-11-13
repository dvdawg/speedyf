//! The lookup chain (projects.md sec. 4): which services a paper is asked
//! about, in what order, and what becomes of the answers.
//!
//! Nothing here is asked that cannot help. A DOI goes to Crossref, and to
//! DataCite only when Crossref has no record (Zenodo and the other
//! non-Crossref DOIs live there); Semantic Scholar is asked for the abstract
//! and the citation edges; INSPIRE only when the paper looks like physics;
//! OpenAlex only when venue, pages or volume is still missing after all
//! that. An arXiv id without a DOI goes to arXiv first - its record may
//! carry the journal DOI, in which case the chain continues down the DOI
//! branch - then INSPIRE when physics, Semantic Scholar, and OpenAlex only
//! when the venue is still missing. An ISBN goes to Open Library. The plan
//! is data ([`plan_identifier_sources`]), so the choice is tested without a
//! network; [`Chain::lookup_by_identifiers`] follows it one request at a
//! time, each through the [`HostLimiter`].
//!
//! A title search asks OpenAlex, Crossref, arXiv and Semantic Scholar at
//! once (different hosts never wait on each other), dblp when the paper
//! looks like computer science and INSPIRE when it looks like physics,
//! merges the lists, and judges them by the user's rule: an exact match, or
//! a near match with the same year, is applied; anything less is parked on
//! the paper for the user ([`apply_title_candidates`]). A title too short
//! to be sure of ("Introduction", a file stem) is never applied unasked.
//!
//! The citation edges (projects.md sec. 8) are fetched afterwards, in the
//! background ([`Chain::fetch_edges`]): Semantic Scholar's references for
//! every paper it answered for, its citations only when an API key is set
//! (without one every request costs three seconds, and citations are the
//! half the sidecars cannot supply anyway - one call per paper, not two),
//! and OpenAlex's `referenced_works` when it answered. A paper's edges are
//! fetched once: a `refs:`/`cites:` key already in the cache, or a job
//! already in flight from an earlier batch, is not queued again.
//!
//! Every answer, including a miss, goes into the [`LookupCache`] under its
//! own key, so a re-run costs nothing until the miss expires; the cache is
//! written at most every [`CACHE_SAVE_INTERVAL`] while a chain runs and
//! once more when its phase ends ([`Chain::flush`]), serialized outside
//! the lock. A host whose hold has grown past [`HOST_GIVE_UP`] is left
//! alone for the rest of the chain's life - a batch does not wait two
//! minutes per paper on a service that keeps refusing - and a paper that
//! met such a host is not stamped as looked up, so the next batch asks
//! again. Turning lookup off stops every chain and the edge task at their
//! next request: the settings are read live, never from a copy.

use crate::engine::types::CitationIdDto;
use crate::errors::{AppError, AppResult};
use crate::library::lookup::{self, LookupCache, LookupSettings};
use crate::library::papers::{LookupPlan, LookupTarget};
use crate::library::ratelimit::{service_name, HostLimiter};
use crate::library::sources::{datacite, dblp, inspire, openlibrary, semantic_scholar};
use crate::library::titlematch::{self, MatchQuality};
use crate::library::types::{LookupBatchDto, LookupCandidateDto};
use futures_util::future;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A host whose hold (from its 429s) has grown past this is skipped for the
/// rest of a run, and the log says so.
pub const HOST_GIVE_UP: Duration = Duration::from_secs(60);
/// The most candidates parked on a paper.
pub const PARK_LIMIT: usize = 5;
/// The least time between two saves of the cache while a chain runs; the
/// end of a phase saves regardless.
pub const CACHE_SAVE_INTERVAL: Duration = Duration::from_secs(5);
/// A title with fewer normalized words than this, or fewer characters than
/// [`SURE_TITLE_CHARS`], is never applied without asking: "Introduction"
/// or a file stem matches some lookup to the letter, and applying it would
/// write a stranger's DOI onto the paper as its identity.
pub const SURE_TITLE_WORDS: usize = 3;
pub const SURE_TITLE_CHARS: usize = 15;

/// The arXiv categories (by prefix) that make a paper physics, for INSPIRE.
const PHYSICS_CATEGORIES: [&str; 8] = [
    "hep-", "nucl-", "cond-mat", "quant-ph", "math-ph", "astro-ph", "gr-qc", "physics",
];
/// Words in a venue or title that make a paper physics (compared
/// case-insensitively).
const PHYSICS_WORDS: [&str; 5] = ["phys", "nucl", "rev. mod", "j. stat", "lett."];
/// Words in a venue or title that make a paper computer science, for dblp.
const CS_WORDS: [&str; 6] = [
    "proceedings",
    "conference",
    "computing",
    "learning",
    "neural",
    "algorithm",
];

/// A service the chain can ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Crossref,
    DataCite,
    OpenAlex,
    SemanticScholar,
    Inspire,
    Arxiv,
    OpenLibrary,
    Dblp,
}

impl Source {
    /// The host its requests go to, which is what the limiter paces.
    pub fn host(self) -> &'static str {
        match self {
            Source::Crossref => "api.crossref.org",
            Source::DataCite => "api.datacite.org",
            Source::OpenAlex => "api.openalex.org",
            Source::SemanticScholar => "api.semanticscholar.org",
            Source::Inspire => "inspirehep.net",
            Source::Arxiv => "export.arxiv.org",
            Source::OpenLibrary => "openlibrary.org",
            Source::Dblp => "dblp.org",
        }
    }

    /// The cache's name for the service in a `<service>:doi:...` key.
    fn cache_name(self) -> &'static str {
        match self {
            Source::Crossref => "crossref",
            Source::DataCite => "datacite",
            Source::OpenAlex => "openalex",
            Source::SemanticScholar => "s2",
            Source::Inspire => "inspire",
            Source::Arxiv => "arxiv",
            Source::OpenLibrary => "openlibrary",
            Source::Dblp => "dblp",
        }
    }

    fn name(self) -> String {
        service_name(self.host())
    }
}

/// Every service, for the tests that hold every host.
#[cfg_attr(not(test), allow(dead_code))] // read by the tests
pub const ALL_SOURCES: [Source; 8] = [
    Source::Crossref,
    Source::DataCite,
    Source::OpenAlex,
    Source::SemanticScholar,
    Source::Inspire,
    Source::Arxiv,
    Source::OpenLibrary,
    Source::Dblp,
];

/// When a planned step is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// Only when the step before it found nothing.
    OnMiss,
    /// Only when venue, pages or volume is still missing after the steps
    /// before it.
    IfBibliographyMissing,
    /// Only when the venue is still missing after the steps before it.
    IfVenueMissing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub source: Source,
    pub when: When,
}

const fn step(source: Source, when: When) -> Step {
    Step { source, when }
}

/// Which chain a batch runs for a paper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainKind {
    /// By DOI, arXiv id or ISBN; applied as it comes.
    Identifier,
    /// By title; applied only on a sure match, else parked.
    Title,
}

// ---------------------------------------------------------------------------
// Source selection
// ---------------------------------------------------------------------------

/// The archive of an old-style arXiv id: `hep-th/9901001` -> `hep-th`,
/// `math.GT/0309136` -> `math.gt`. `None` for a new-style id, which says
/// nothing about its subject.
pub fn legacy_arxiv_archive(id: &str) -> Option<String> {
    let id = id.trim();
    let id = id
        .strip_prefix("arXiv:")
        .or_else(|| id.strip_prefix("arxiv:"))
        .unwrap_or(id);
    let (archive, rest) = id.split_once('/')?;
    let plausible = !archive.is_empty()
        && archive
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.')
        && rest.chars().next().is_some_and(|c| c.is_ascii_digit());
    plausible.then(|| archive.to_ascii_lowercase())
}

/// Whether a paper looks like physics, which is when INSPIRE is worth
/// asking: its arXiv category (given, or read off an old-style id) is one
/// of hep-*, nucl-*, cond-mat*, quant-ph, math-ph, astro-ph*, gr-qc or
/// physics.*, or its venue or title mentions "Phys", "Nucl", "Rev. Mod",
/// "J. Stat" or "Lett.".
pub fn looks_like_physics(
    arxiv_id: Option<&str>,
    category: Option<&str>,
    venue: Option<&str>,
    title: Option<&str>,
) -> bool {
    let category = category
        .map(str::to_ascii_lowercase)
        .or_else(|| arxiv_id.and_then(legacy_arxiv_archive));
    if category
        .as_deref()
        .is_some_and(|c| PHYSICS_CATEGORIES.iter().any(|p| c.starts_with(p)))
    {
        return true;
    }
    mentions_any(venue, &PHYSICS_WORDS) || mentions_any(title, &PHYSICS_WORDS)
}

/// Whether a paper looks like computer science, which is when dblp is worth
/// asking: an arXiv category of cs.* (or the old `cs/` archive), or a venue
/// or title mentioning "Proceedings", "Conference", "Computing",
/// "Learning", "Neural" or "Algorithm".
pub fn looks_like_cs(
    arxiv_id: Option<&str>,
    category: Option<&str>,
    venue: Option<&str>,
    title: Option<&str>,
) -> bool {
    let category = category
        .map(str::to_ascii_lowercase)
        .or_else(|| arxiv_id.and_then(legacy_arxiv_archive));
    if category
        .as_deref()
        .is_some_and(|c| c == "cs" || c.starts_with("cs."))
    {
        return true;
    }
    mentions_any(venue, &CS_WORDS) || mentions_any(title, &CS_WORDS)
}

fn mentions_any(text: Option<&str>, words: &[&str]) -> bool {
    let Some(text) = text else {
        return false;
    };
    let text = text.to_lowercase();
    words.iter().any(|word| text.contains(word))
}

/// The category arXiv gave this run, else the one the file's own stamp
/// carried: the record knows its subject before any service is asked.
fn category_of<'a>(target: &'a LookupTarget, category: Option<&'a str>) -> Option<&'a str> {
    category.or(target.arxiv_category.as_deref())
}

fn target_is_physics(target: &LookupTarget, category: Option<&str>) -> bool {
    looks_like_physics(
        target.arxiv.as_deref(),
        category_of(target, category),
        target.venue.as_deref(),
        target.title.as_deref(),
    )
}

fn target_is_cs(target: &LookupTarget, category: Option<&str>) -> bool {
    looks_like_cs(
        target.arxiv.as_deref(),
        category_of(target, category),
        target.venue.as_deref(),
        target.title.as_deref(),
    )
}

/// The services an identifier lookup of this paper asks, in order, from
/// what is known before anything is fetched. A DOI: Crossref, DataCite on
/// a miss, Semantic Scholar, INSPIRE when physics, OpenAlex when the
/// bibliography is still incomplete. An arXiv id alone: arXiv, INSPIRE when
/// physics, Semantic Scholar, OpenAlex when the venue is still missing -
/// and should arXiv supply a journal DOI, the DOI steps follow, less the
/// services already asked. An ISBN alone: Open Library. Nothing: nothing.
pub fn plan_identifier_sources(target: &LookupTarget) -> Vec<Step> {
    let physics = target_is_physics(target, None);
    if target.doi.is_some() {
        return doi_steps(physics);
    }
    if target.arxiv.is_some() {
        let mut steps = vec![step(Source::Arxiv, When::Always)];
        steps.extend(arxiv_only_steps(physics));
        return steps;
    }
    if target.isbn.is_some() {
        return vec![step(Source::OpenLibrary, When::Always)];
    }
    Vec::new()
}

fn doi_steps(physics: bool) -> Vec<Step> {
    let mut steps = vec![
        step(Source::Crossref, When::Always),
        step(Source::DataCite, When::OnMiss),
        step(Source::SemanticScholar, When::Always),
    ];
    if physics {
        steps.push(step(Source::Inspire, When::Always));
    }
    steps.push(step(Source::OpenAlex, When::IfBibliographyMissing));
    steps
}

/// The steps after arXiv itself, for a paper arXiv gave no DOI for.
fn arxiv_only_steps(physics: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if physics {
        steps.push(step(Source::Inspire, When::Always));
    }
    steps.push(step(Source::SemanticScholar, When::Always));
    steps.push(step(Source::OpenAlex, When::IfVenueMissing));
    steps
}

// ---------------------------------------------------------------------------
// Title judgement
// ---------------------------------------------------------------------------

/// What a title search comes to for a paper.
#[derive(Clone, Debug, PartialEq)]
// Built once per search and consumed at once; the gap between a candidate
// and `Nothing` costs nothing that matters.
#[allow(clippy::large_enum_variant)]
pub enum TitleOutcome {
    /// A sure match: applied without asking.
    Apply(LookupCandidateDto),
    /// Something, but nothing sure enough: parked for the user, at most
    /// [`PARK_LIMIT`].
    Park(Vec<LookupCandidateDto>),
    /// Nothing found, or nothing the user has not already declined.
    Nothing,
}

/// The candidates less those the user declined before (`declined`, by
/// `titlematch::identity`): the question has been asked.
pub fn without_declined(
    candidates: Vec<LookupCandidateDto>,
    declined: &[String],
) -> Vec<LookupCandidateDto> {
    candidates
        .into_iter()
        .filter(|candidate| !declined.contains(&titlematch::identity(candidate)))
        .collect()
}

/// Whether a title is long enough to apply a match on without asking: at
/// least [`SURE_TITLE_WORDS`] words and [`SURE_TITLE_CHARS`] characters
/// once normalized.
pub fn title_is_sure_enough(title: &str) -> bool {
    let normalized = titlematch::normalize(title);
    normalized
        .split(' ')
        .filter(|word| !word.is_empty())
        .count()
        >= SURE_TITLE_WORDS
        && normalized.chars().count() >= SURE_TITLE_CHARS
}

/// The user's rule applied to a merged candidate list: the first exact
/// match is applied; failing that the first match that still
/// [`MatchQuality::applies`] - near, with the same year (the list is in
/// source rank order, so first is best); failing that the list is parked,
/// likeliest title first. A paper whose title is too short to be sure of
/// ([`title_is_sure_enough`]) has everything parked. Declined candidates
/// are dropped first.
pub fn apply_title_candidates(
    title: &str,
    year: Option<i32>,
    merged: Vec<LookupCandidateDto>,
    declined: &[String],
) -> TitleOutcome {
    let mut candidates = without_declined(merged, declined);
    if candidates.is_empty() {
        return TitleOutcome::Nothing;
    }
    if title_is_sure_enough(title) {
        let grade = |candidate: &LookupCandidateDto| {
            titlematch::judge(title, year, &candidate.title, candidate.year)
        };
        let sure = candidates
            .iter()
            .position(|c| grade(c) == MatchQuality::Exact)
            .or_else(|| candidates.iter().position(|c| grade(c).applies()));
        if let Some(at) = sure {
            return TitleOutcome::Apply(candidates.swap_remove(at));
        }
    }
    // Stable, so equally alike titles keep their source rank order.
    candidates.sort_by(|a, b| {
        titlematch::similarity(title, &b.title)
            .partial_cmp(&titlematch::similarity(title, &a.title))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(PARK_LIMIT);
    TitleOutcome::Park(candidates)
}

// ---------------------------------------------------------------------------
// Running the chain
// ---------------------------------------------------------------------------

/// What asking one service came to.
#[derive(Debug)]
// One per request, read once; the gap between a candidate and a miss
// costs nothing that matters.
#[allow(clippy::large_enum_variant)]
enum Consulted {
    Found(LookupCandidateDto),
    /// No record - from the cache or live; cached either way.
    Missing,
    /// Not asked: the identifier it needs is unknown, or the step's
    /// condition did not hold.
    Skipped,
    /// Not asked: its host is given up on, or lookup was turned off. The
    /// outcome is then incomplete, and the paper not stamped as looked up.
    GivenUp,
    /// The request failed, after the limiter's retry.
    Failed(AppError),
}

/// What the paper has, for the conditional steps; grows with each answer.
#[derive(Clone, Copy, Debug)]
struct Known {
    venue: bool,
    pages: bool,
    volume: bool,
}

impl Known {
    fn of(target: &LookupTarget) -> Self {
        Self {
            venue: target.venue.is_some(),
            pages: target.has_pages,
            volume: target.has_volume,
        }
    }

    fn absorb(&mut self, candidate: &LookupCandidateDto) {
        self.venue |= candidate.venue.is_some();
        self.pages |= candidate.pages.is_some();
        self.volume |= candidate.volume.is_some();
    }

    fn applies(&self, when: When, previous: &Consulted) -> bool {
        match when {
            When::Always => true,
            When::OnMiss => matches!(previous, Consulted::Missing),
            When::IfBibliographyMissing => !(self.venue && self.pages && self.volume),
            When::IfVenueMissing => !self.venue,
        }
    }
}

/// The citation edges to fetch for a paper once its lookup is done: from
/// Semantic Scholar when it answered, and OpenAlex when it did. Fetched in
/// the background ([`Chain::fetch_edges`]), once per identifier: a job is
/// claimed in [`ChainShared::edges_in_flight`] when queued and released
/// when done, and a `refs:`/`cites:` key already in the cache is not
/// fetched again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeJob {
    pub id: CitationIdDto,
    pub semantic_scholar: bool,
    pub openalex: bool,
}

/// What an identifier chain came to for a paper.
#[derive(Debug, Default)]
pub struct ChainOutcome {
    /// Every service's candidate, in the order asked; each is applied at
    /// its own source's rank.
    pub candidates: Vec<LookupCandidateDto>,
    /// The requests that failed, as messages; what the other services gave
    /// is still in `candidates`.
    pub errors: Vec<String>,
    /// Every planned service was asked (or answered from the cache): none
    /// was skipped for a host given up on or for lookup turned off. Only
    /// a complete, error-free chain stamps the paper as looked up.
    pub complete: bool,
    pub edges: Option<EdgeJob>,
}

/// What a title search came to, before judgement.
#[derive(Debug, Default)]
pub struct TitleSearch {
    /// Merged across the services, in source rank order.
    pub candidates: Vec<LookupCandidateDto>,
    pub errors: Vec<String>,
    /// Every service was asked; see [`ChainOutcome::complete`].
    pub complete: bool,
}

type Search<'a> = Pin<Box<dyn Future<Output = AppResult<Vec<LookupCandidateDto>>> + Send + 'a>>;

/// What every chain shares with the engine: the client, the limiter, the
/// settings as they are now (read live, so turning lookup off stops a
/// running chain), the cache and where it is saved, and the edge jobs in
/// flight across batches.
#[derive(Clone)]
pub struct ChainShared {
    pub client: reqwest::Client,
    pub limiter: Arc<HostLimiter>,
    pub settings: Arc<Mutex<LookupSettings>>,
    pub cache: Arc<Mutex<LookupCache>>,
    pub cache_dir: PathBuf,
    pub edges_in_flight: Arc<Mutex<HashSet<CitationIdDto>>>,
}

/// One lookup or batch, owning what it needs so it can run on in the
/// background after the command that started it has answered. The hosts it
/// gives up on are its own.
pub struct Chain {
    shared: ChainShared,
    given_up: Mutex<HashSet<&'static str>>,
    /// OpenAlex's `referenced_works` from this chain's own fetches, so the
    /// edge phase does not fetch the record again.
    openalex_works: Mutex<HashMap<CitationIdDto, Vec<String>>>,
    dirty: AtomicBool,
    last_saved: Mutex<Instant>,
    saves: AtomicU32,
}

impl Chain {
    pub fn new(shared: ChainShared) -> Self {
        Self {
            shared,
            given_up: Mutex::new(HashSet::new()),
            openalex_works: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(false),
            // The first answer is saved at once; the rest are throttled.
            last_saved: Mutex::new(Instant::now() - CACHE_SAVE_INTERVAL),
            saves: AtomicU32::new(0),
        }
    }

    fn settings(&self) -> LookupSettings {
        self.shared.settings.lock().clone()
    }

    /// Whether lookup is still on, as of now.
    pub fn enabled(&self) -> bool {
        self.shared.settings.lock().enabled
    }

    /// How many times the cache has been written by this chain.
    #[cfg_attr(not(test), allow(dead_code))] // read by the tests
    pub fn saves(&self) -> u32 {
        self.saves.load(Ordering::SeqCst)
    }

    /// The identifier chain for one paper: the plan, followed one request
    /// at a time, re-planned when arXiv supplies a DOI or a category. Never
    /// fails as a whole - a service that fails is logged and counted in
    /// `errors`, and the rest are still asked.
    pub async fn lookup_by_identifiers(&self, target: &LookupTarget) -> ChainOutcome {
        let mut outcome = ChainOutcome {
            complete: true,
            ..ChainOutcome::default()
        };
        let mut known = Known::of(target);
        let mut doi = target.doi.as_deref().map(lookup::normalize_doi);
        let arxiv = target.arxiv.as_deref().map(lookup::normalize_arxiv_id);
        let mut category: Option<String> = None;
        let mut asked: HashSet<Source> = HashSet::new();
        let mut steps = plan_identifier_sources(target);
        let mut previous = Consulted::Missing;
        let mut at = 0;
        let (mut s2_answered, mut openalex_answered) = (false, false);
        while at < steps.len() {
            let Step { source, when } = steps[at];
            at += 1;
            if asked.contains(&source) {
                continue;
            }
            if !known.applies(when, &previous) {
                previous = Consulted::Skipped;
                continue;
            }
            asked.insert(source);
            let result = self
                .ask(
                    source,
                    doi.as_deref(),
                    arxiv.as_deref(),
                    target.isbn.as_deref(),
                    &mut category,
                )
                .await;
            match &result {
                Consulted::Found(candidate) => {
                    known.absorb(candidate);
                    outcome.candidates.push(candidate.clone());
                    s2_answered |= source == Source::SemanticScholar;
                    openalex_answered |= source == Source::OpenAlex;
                }
                Consulted::Failed(error) => outcome.errors.push(error.to_string()),
                Consulted::GivenUp => outcome.complete = false,
                Consulted::Missing | Consulted::Skipped => {}
            }
            if source == Source::Arxiv {
                // arXiv may have said more about the paper than the record
                // did: a journal DOI sends the chain down the DOI branch, and
                // the category decides INSPIRE. Replan; `asked` keeps a
                // service from being asked twice.
                if doi.is_none() {
                    doi = match &result {
                        Consulted::Found(candidate) => candidate
                            .doi
                            .as_deref()
                            .map(lookup::normalize_doi)
                            .filter(|d| !d.is_empty() && !is_arxiv_doi(d)),
                        _ => None,
                    };
                }
                let physics = target_is_physics(target, category.as_deref());
                steps = if doi.is_some() {
                    doi_steps(physics)
                } else {
                    arxiv_only_steps(physics)
                };
                at = 0;
            }
            previous = result;
        }
        if s2_answered || openalex_answered {
            let id = match (&doi, &arxiv) {
                (Some(doi), _) => Some(CitationIdDto::Doi(doi.clone())),
                (None, Some(arxiv)) => Some(CitationIdDto::ArXiv(arxiv.clone())),
                (None, None) => None,
            };
            if let Some(id) = id {
                outcome.edges = self.claim_edges(id, s2_answered, openalex_answered);
            }
        }
        outcome
    }

    /// An edge job for the paper, unless its edges are cached already or a
    /// job for it is in flight from an earlier lookup. Claimed here; the
    /// claim is released when the job is done or dropped.
    fn claim_edges(
        &self,
        id: CitationIdDto,
        semantic_scholar: bool,
        openalex: bool,
    ) -> Option<EdgeJob> {
        let wanted = {
            let cache = self.shared.cache.lock();
            let want_cites = semantic_scholar && self.settings().s2_key().is_some();
            cache.get_links(&LookupCache::refs_key(&id)).is_none()
                || (want_cites && cache.get_links(&LookupCache::cites_key(&id)).is_none())
        };
        if !wanted {
            return None;
        }
        let mut in_flight = self.shared.edges_in_flight.lock();
        if !in_flight.insert(id.clone()) {
            return None;
        }
        Some(EdgeJob {
            id,
            semantic_scholar,
            openalex,
        })
    }

    /// One service, by whichever identifier it takes: the DOI when one is
    /// known, else the arXiv id. `category` is filled from an arXiv answer.
    async fn ask(
        &self,
        source: Source,
        doi: Option<&str>,
        arxiv: Option<&str>,
        isbn: Option<&str>,
        category: &mut Option<String>,
    ) -> Consulted {
        let id = match (doi, arxiv) {
            (Some(doi), _) => Some(CitationIdDto::Doi(doi.to_string())),
            (None, Some(arxiv)) => Some(CitationIdDto::ArXiv(arxiv.to_string())),
            (None, None) => None,
        };
        let settings = self.settings();
        let (client, limiter, settings) = (&self.shared.client, &*self.shared.limiter, &settings);
        match source {
            Source::Crossref => {
                let Some(doi) = doi else {
                    return Consulted::Skipped;
                };
                self.consult(LookupCache::doi_key(doi), source, || {
                    lookup::fetch_by_doi(client, limiter, settings, doi)
                })
                .await
            }
            Source::DataCite => {
                let (Some(doi), Some(id)) = (doi, &id) else {
                    return Consulted::Skipped;
                };
                self.consult(
                    LookupCache::source_key(source.cache_name(), id),
                    source,
                    || datacite::fetch_by_doi(client, limiter, settings, doi),
                )
                .await
            }
            Source::OpenAlex => {
                let Some(id) = id else {
                    return Consulted::Skipped;
                };
                let key = LookupCache::source_key(source.cache_name(), &id);
                if let Some(cached) = self.cached(&key) {
                    return cached;
                }
                if !self.host_open(source) {
                    return Consulted::GivenUp;
                }
                match lookup::fetch_openalex_record(client, limiter, settings, &id).await {
                    Ok(Some(record)) => {
                        // Kept for the edge phase, which would otherwise
                        // fetch the same record again for these.
                        if !record.referenced_works.is_empty() {
                            self.openalex_works
                                .lock()
                                .insert(id, record.referenced_works);
                        }
                        match record.candidate {
                            Some(candidate) => {
                                self.remember(key, Some(candidate.clone()));
                                Consulted::Found(candidate)
                            }
                            None => {
                                self.remember(key, None);
                                Consulted::Missing
                            }
                        }
                    }
                    Ok(None) => {
                        self.remember(key, None);
                        Consulted::Missing
                    }
                    Err(error) => self.failed(source, error),
                }
            }
            Source::SemanticScholar => {
                let Some(id) = &id else {
                    return Consulted::Skipped;
                };
                self.consult(
                    LookupCache::source_key(source.cache_name(), id),
                    source,
                    || semantic_scholar::fetch_by_id(client, limiter, settings, id),
                )
                .await
            }
            Source::Inspire => {
                let Some(id) = &id else {
                    return Consulted::Skipped;
                };
                self.consult(
                    LookupCache::source_key(source.cache_name(), id),
                    source,
                    || async move {
                        match id {
                            CitationIdDto::Doi(doi) => {
                                inspire::fetch_by_doi(client, limiter, settings, doi).await
                            }
                            CitationIdDto::ArXiv(arxiv) => {
                                inspire::fetch_by_arxiv(client, limiter, settings, arxiv).await
                            }
                        }
                    },
                )
                .await
            }
            Source::Arxiv => {
                let Some(arxiv) = arxiv else {
                    return Consulted::Skipped;
                };
                let key = LookupCache::arxiv_key(arxiv);
                if let Some(cached) = self.cached(&key) {
                    *category = self.shared.cache.lock().get_category(&key);
                    return cached;
                }
                if !self.host_open(source) {
                    return Consulted::GivenUp;
                }
                match lookup::fetch_by_arxiv(client, limiter, settings, arxiv).await {
                    Ok(Some(record)) => {
                        *category = record.primary_category.clone();
                        {
                            let mut cache = self.shared.cache.lock();
                            cache.insert(key.clone(), record.candidate.clone());
                            if let Some(category) = record.primary_category {
                                cache.insert_category(key, category);
                            }
                        }
                        self.changed();
                        Consulted::Found(record.candidate)
                    }
                    Ok(None) => {
                        self.remember(key, None);
                        Consulted::Missing
                    }
                    Err(error) => self.failed(source, error),
                }
            }
            Source::OpenLibrary => {
                let Some(isbn) = isbn else {
                    return Consulted::Skipped;
                };
                self.consult(LookupCache::isbn_key(isbn), source, || {
                    openlibrary::fetch_by_isbn(client, limiter, settings, isbn)
                })
                .await
            }
            // dblp has no lookup by identifier.
            Source::Dblp => Consulted::Skipped,
        }
    }

    /// The cache first - a hit or a standing miss costs nothing - then the
    /// fetch, unless the host is given up on; the answer is cached either
    /// way.
    async fn consult<F, Fut>(&self, key: String, source: Source, fetch: F) -> Consulted
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<Option<LookupCandidateDto>>>,
    {
        if let Some(cached) = self.cached(&key) {
            return cached;
        }
        if !self.host_open(source) {
            return Consulted::GivenUp;
        }
        match fetch().await {
            Ok(Some(candidate)) => {
                self.remember(key, Some(candidate.clone()));
                Consulted::Found(candidate)
            }
            Ok(None) => {
                self.remember(key, None);
                Consulted::Missing
            }
            Err(error) => self.failed(source, error),
        }
    }

    fn failed(&self, source: Source, error: AppError) -> Consulted {
        log::warn!("lookup: {} failed: {error}", source.name());
        Consulted::Failed(error)
    }

    fn cached(&self, key: &str) -> Option<Consulted> {
        let cache = self.shared.cache.lock();
        if let Some(candidate) = cache.get(key) {
            Some(Consulted::Found(candidate.clone()))
        } else if cache.is_miss(key) {
            Some(Consulted::Missing)
        } else {
            None
        }
    }

    /// Keeps an answer; the disk follows (`changed`).
    fn remember(&self, key: String, candidate: Option<LookupCandidateDto>) {
        {
            let mut cache = self.shared.cache.lock();
            match candidate {
                Some(candidate) => cache.insert(key, candidate),
                None => cache.insert_miss(key),
            }
        }
        self.changed();
    }

    fn remember_list(&self, key: String, candidates: Vec<LookupCandidateDto>) {
        self.shared.cache.lock().insert_list(key, candidates);
        self.changed();
    }

    fn remember_links(&self, key: String, ids: Vec<CitationIdDto>) {
        self.shared.cache.lock().insert_links(key, ids);
        self.changed();
    }

    /// The cache changed: saved now if the last save is
    /// [`CACHE_SAVE_INTERVAL`] ago, else left for the next change or the
    /// phase's [`flush`](Self::flush).
    fn changed(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        let due = self.last_saved.lock().elapsed() >= CACHE_SAVE_INTERVAL;
        if due {
            self.save_now();
        }
    }

    /// Writes the cache if anything is unsaved. Called when a phase ends: a
    /// lookup, a batch, the edge task.
    pub fn flush(&self) {
        if self.dirty.load(Ordering::SeqCst) {
            self.save_now();
        }
    }

    /// A snapshot under the lock, serialized and written outside it. The
    /// cache is disposable, so failing to save it is logged and nothing
    /// more.
    fn save_now(&self) {
        let snapshot = {
            let cache = self.shared.cache.lock();
            if !self.dirty.swap(false, Ordering::SeqCst) {
                return;
            }
            cache.clone()
        };
        *self.last_saved.lock() = Instant::now();
        self.saves.fetch_add(1, Ordering::SeqCst);
        if let Err(error) = snapshot.save(&self.shared.cache_dir) {
            log::warn!("cannot save the lookup cache: {error}");
        }
    }

    /// Whether a service is worth asking: lookup is still on, and its host
    /// is neither given up on nor held for longer than [`HOST_GIVE_UP`]. A
    /// host that crosses the line is given up on for the rest of this
    /// chain's life, once, in the log.
    fn host_open(&self, source: Source) -> bool {
        if !self.enabled() {
            return false;
        }
        let host = source.host();
        let mut given_up = self.given_up.lock();
        if given_up.contains(host) {
            return false;
        }
        let wait = self.shared.limiter.wait_for(host);
        if wait > HOST_GIVE_UP {
            log::warn!(
                "lookup: {} keeps refusing (held for {} s); skipping it for the rest of this run",
                source.name(),
                wait.as_secs()
            );
            given_up.insert(host);
            return false;
        }
        true
    }

    // -- title search ------------------------------------------------------

    /// The title chain: every service that can search by title, asked at
    /// once, merged. From the cache when the title was searched for within
    /// [`lookup::LIST_TTL`]. A list that came back without one of the
    /// services (a failure, a host given up on) is not cached, so the next
    /// search asks it again.
    pub async fn search_by_title_everywhere(&self, target: &LookupTarget) -> TitleSearch {
        let title = target
            .title
            .as_deref()
            .map(lookup::collapse_whitespace)
            .unwrap_or_default();
        if title.is_empty() {
            return TitleSearch {
                complete: true,
                ..TitleSearch::default()
            };
        }
        let key = LookupCache::title_key(&title);
        {
            let cache = self.shared.cache.lock();
            if let Some(list) = cache.get_list(&key) {
                return TitleSearch {
                    candidates: list.to_vec(),
                    errors: Vec::new(),
                    complete: true,
                };
            }
            if cache.is_miss(&key) {
                return TitleSearch {
                    complete: true,
                    ..TitleSearch::default()
                };
            }
        }
        let mut sources = vec![
            Source::OpenAlex,
            Source::Crossref,
            Source::Arxiv,
            Source::SemanticScholar,
        ];
        if target_is_cs(target, None) {
            sources.push(Source::Dblp);
        }
        if target_is_physics(target, None) {
            sources.push(Source::Inspire);
        }
        let open: Vec<Source> = sources
            .iter()
            .copied()
            .filter(|source| self.host_open(*source))
            .collect();
        let complete = open.len() == sources.len();
        let settings = self.settings();
        let searches: Vec<Search<'_>> = open
            .iter()
            .map(|source| self.search(*source, &title, &settings))
            .collect();
        let results = future::join_all(searches).await;
        let mut lists = Vec::with_capacity(results.len());
        let mut errors = Vec::new();
        for (source, result) in open.into_iter().zip(results) {
            match result {
                Ok(list) => lists.push(list),
                Err(error) => {
                    log::warn!("lookup: {} title search failed: {error}", source.name());
                    errors.push(error.to_string());
                }
            }
        }
        let candidates = titlematch::merge_candidates(lists);
        if complete && errors.is_empty() {
            self.remember_list(key, candidates.clone());
        }
        TitleSearch {
            candidates,
            errors,
            complete,
        }
    }

    fn search<'a>(
        &'a self,
        source: Source,
        title: &'a str,
        settings: &'a LookupSettings,
    ) -> Search<'a> {
        let (client, limiter) = (&self.shared.client, &*self.shared.limiter);
        match source {
            Source::OpenAlex => Box::pin(lookup::search_by_title(client, limiter, settings, title)),
            Source::Crossref => Box::pin(lookup::search_crossref_by_title(
                client, limiter, settings, title,
            )),
            Source::Arxiv => Box::pin(lookup::search_arxiv_by_title(
                client, limiter, settings, title,
            )),
            Source::SemanticScholar => Box::pin(semantic_scholar::search_by_title(
                client, limiter, settings, title,
            )),
            Source::Dblp => Box::pin(dblp::search_by_title(client, limiter, settings, title)),
            Source::Inspire => Box::pin(inspire::search_by_title(client, limiter, settings, title)),
            Source::DataCite | Source::OpenLibrary => Box::pin(async { Ok(Vec::new()) }),
        }
    }

    // -- citation edges ----------------------------------------------------

    /// Fetches the edges for each job and stores them under `refs:` and
    /// `cites:`; `refs` is the union of Semantic Scholar's references and
    /// OpenAlex's. A key already present is left alone. A list is stored
    /// only when every request behind it succeeded, so a failure is asked
    /// again by the next lookup rather than remembered as nothing. Stops
    /// at the first job after lookup is turned off. Every job's claim is
    /// released, and the cache saved, before this returns.
    pub async fn fetch_edges(&self, jobs: Vec<EdgeJob>) {
        for job in jobs {
            if self.enabled() {
                self.fetch_edge(&job).await;
            }
            self.shared.edges_in_flight.lock().remove(&job.id);
        }
        self.flush();
    }

    async fn fetch_edge(&self, job: &EdgeJob) {
        let settings = self.settings();
        let (client, limiter, settings) = (&self.shared.client, &*self.shared.limiter, &settings);
        let refs_key = LookupCache::refs_key(&job.id);
        let cites_key = LookupCache::cites_key(&job.id);
        let (want_refs, want_cites) = {
            let cache = self.shared.cache.lock();
            (
                cache.get_links(&refs_key).is_none(),
                job.semantic_scholar
                    && settings.s2_key().is_some()
                    && cache.get_links(&cites_key).is_none(),
            )
        };
        let mut refs: Vec<CitationIdDto> = Vec::new();
        let mut refs_complete = job.semantic_scholar || job.openalex;
        if job.semantic_scholar && want_refs {
            match self
                .guarded(Source::SemanticScholar, || {
                    semantic_scholar::fetch_references(client, limiter, settings, &job.id)
                })
                .await
            {
                Some(Ok(ids)) => refs.extend(ids),
                _ => refs_complete = false,
            }
        }
        if job.openalex && want_refs {
            let known_works = self.openalex_works.lock().remove(&job.id);
            match self
                .guarded(Source::OpenAlex, || async {
                    let works = match known_works {
                        Some(works) => works,
                        None => lookup::fetch_openalex_record(client, limiter, settings, &job.id)
                            .await?
                            .map(|record| record.referenced_works)
                            .unwrap_or_default(),
                    };
                    if works.is_empty() {
                        return Ok(Vec::new());
                    }
                    lookup::resolve_openalex_works(client, limiter, settings, &works).await
                })
                .await
            {
                Some(Ok(ids)) => refs.extend(ids),
                _ => refs_complete = false,
            }
        }
        if want_refs && refs_complete {
            let mut seen = HashSet::new();
            refs.retain(|id| seen.insert(id.clone()));
            self.remember_links(refs_key, refs);
        }
        if want_cites {
            if let Some(Ok(ids)) = self
                .guarded(Source::SemanticScholar, || {
                    semantic_scholar::fetch_citations(client, limiter, settings, &job.id)
                })
                .await
            {
                self.remember_links(cites_key, ids);
            }
        }
    }

    /// Runs a request unless the host is given up on; a failure is logged.
    async fn guarded<T, F, Fut>(&self, source: Source, fetch: F) -> Option<AppResult<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        if !self.host_open(source) {
            return None;
        }
        let result = fetch().await;
        if let Err(error) = &result {
            log::warn!("lookup: {} citation edges failed: {error}", source.name());
        }
        Some(result)
    }
}

/// `10.48550/arxiv....`: the DOI DataCite registers for a preprint, which
/// names no journal and sends the chain nowhere new.
fn is_arxiv_doi(doi: &str) -> bool {
    doi.to_ascii_lowercase().starts_with("10.48550/arxiv.")
}

// ---------------------------------------------------------------------------
// Driving a batch
// ---------------------------------------------------------------------------

/// What one paper's turn in a batch came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Applied,
    Parked,
    Failed,
    Nothing,
}

/// What drives a batch: the turns themselves, whether lookup is still on
/// between them, and where to report progress. The command implements it
/// over the engine; the tests over nothing.
pub trait BatchTurns {
    fn enabled(&self) -> bool;
    fn turn(&mut self, kind: ChainKind, target: LookupTarget) -> impl Future<Output = Turn> + Send;
    fn progress(&self, done: u32, total: u32);
}

/// Runs a batch plan: one turn per target, progress after each. When
/// lookup is turned off between turns the rest are counted as skipped and
/// the batch ends there, so the setting stops a run within one paper.
pub async fn drive_batch<T: BatchTurns + Send>(plan: LookupPlan, host: &mut T) -> LookupBatchDto {
    let mut batch = LookupBatchDto {
        skipped: plan.skipped,
        ..LookupBatchDto::default()
    };
    let total = plan.targets.len().min(u32::MAX as usize) as u32;
    if total == 0 {
        return batch;
    }
    host.progress(0, total);
    let mut targets = plan.targets.into_iter();
    let mut done = 0;
    while let Some((kind, target)) = targets.next() {
        if !host.enabled() {
            let left = 1 + targets.len().min(u32::MAX as usize) as u32;
            log::info!("lookup turned off; {left} papers of the batch left alone");
            batch.skipped += left;
            break;
        }
        batch.attempted += 1;
        match host.turn(kind, target).await {
            Turn::Applied => batch.applied += 1,
            Turn::Parked => batch.parked += 1,
            Turn::Failed => batch.failed += 1,
            Turn::Nothing => {}
        }
        done += 1;
        host.progress(done, total);
    }
    batch
}

// ---------------------------------------------------------------------------
// Tests - no network: the plan, the helpers and the judgement are data, and
// the chain is driven against a cache or hosts given up on.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{Author, MetaSource, PaperKind};
    use std::sync::atomic::AtomicU32;

    fn target(doi: Option<&str>, arxiv: Option<&str>, isbn: Option<&str>) -> LookupTarget {
        LookupTarget {
            paper_id: "p".into(),
            doi: doi.map(str::to_owned),
            arxiv: arxiv.map(str::to_owned),
            isbn: isbn.map(str::to_owned),
            ..LookupTarget::default()
        }
    }

    fn sources(steps: &[Step]) -> Vec<(Source, When)> {
        steps.iter().map(|s| (s.source, s.when)).collect()
    }

    fn candidate(source: MetaSource, title: &str, year: Option<i32>) -> LookupCandidateDto {
        LookupCandidateDto {
            source,
            title: title.to_owned(),
            authors: vec![Author {
                family: "Author".into(),
                given: "A.".into(),
            }],
            year,
            venue: None,
            volume: None,
            issue: None,
            pages: None,
            publisher: None,
            abstract_: None,
            kind: None,
            doi: None,
            arxiv: None,
            isbn: None,
        }
    }

    /// A chain over a temp dir, lookup on, no key, nothing cached.
    fn chain_in(dir: &std::path::Path) -> Chain {
        Chain::new(ChainShared {
            client: lookup::client(),
            limiter: Arc::new(HostLimiter::new()),
            settings: Arc::new(Mutex::new(LookupSettings {
                enabled: true,
                contact: String::new(),
                s2_key: String::new(),
            })),
            cache: Arc::new(Mutex::new(LookupCache::default())),
            cache_dir: dir.to_path_buf(),
            edges_in_flight: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    /// Every host held for two minutes: nothing can be sent.
    fn give_up_on_every_host(chain: &Chain) {
        for source in ALL_SOURCES {
            chain
                .shared
                .limiter
                .note_response(source.host(), 429, Some(Duration::from_secs(120)));
        }
    }

    #[test]
    fn legacy_arxiv_ids_name_their_archive() {
        assert_eq!(
            legacy_arxiv_archive("hep-th/9901001").as_deref(),
            Some("hep-th")
        );
        assert_eq!(
            legacy_arxiv_archive("arXiv:math.GT/0309136v2").as_deref(),
            Some("math.gt")
        );
        assert_eq!(legacy_arxiv_archive("cs/0001001").as_deref(), Some("cs"));
        assert_eq!(legacy_arxiv_archive("2401.12345"), None);
        assert_eq!(
            legacy_arxiv_archive("10.1000/x"),
            None,
            "a DOI is not an archive"
        );
        assert_eq!(legacy_arxiv_archive("/0001001"), None);
    }

    #[test]
    fn physics_is_told_by_category_archive_venue_or_title() {
        assert!(looks_like_physics(None, Some("hep-th"), None, None));
        assert!(looks_like_physics(
            None,
            Some("cond-mat.stat-mech"),
            None,
            None
        ));
        assert!(looks_like_physics(None, Some("physics.optics"), None, None));
        assert!(looks_like_physics(None, Some("astro-ph.CO"), None, None));
        assert!(looks_like_physics(
            Some("nucl-th/9901001"),
            None,
            None,
            None
        ));
        assert!(looks_like_physics(
            None,
            None,
            Some("Phys. Rev. Lett."),
            None
        ));
        assert!(looks_like_physics(None, None, Some("Nucl. Phys. B"), None));
        assert!(looks_like_physics(
            None,
            None,
            Some("Rev. Mod. Phys."),
            None
        ));
        assert!(looks_like_physics(None, None, Some("J. Stat. Mech."), None));
        assert!(looks_like_physics(
            None,
            None,
            Some("Europhys. Lett."),
            None
        ));
        assert!(looks_like_physics(
            None,
            None,
            None,
            Some("Nonequilibrium physics of active matter")
        ));
        assert!(!looks_like_physics(None, Some("cs.LG"), None, None));
        assert!(!looks_like_physics(Some("2401.12345"), None, None, None));
        assert!(!looks_like_physics(
            None,
            None,
            Some("NeurIPS"),
            Some("Attention is all you need")
        ));
        assert!(!looks_like_physics(None, None, None, None));
        // A known category outranks an old-style archive and the words.
        assert!(!looks_like_physics(
            Some("hep-th/9901001"),
            Some("cs.AI"),
            None,
            None
        ));
    }

    #[test]
    fn cs_is_told_by_category_archive_venue_or_title() {
        assert!(looks_like_cs(None, Some("cs.LG"), None, None));
        assert!(looks_like_cs(None, Some("cs"), None, None));
        assert!(looks_like_cs(Some("cs/0001001"), None, None, None));
        for venue in [
            "Proceedings of ICML",
            "International Conference on Learning Representations",
            "Journal of Computing",
            "Neural Computation",
            "Algorithmica",
        ] {
            assert!(looks_like_cs(None, None, Some(venue), None), "{venue}");
        }
        assert!(looks_like_cs(
            None,
            None,
            None,
            Some("A fast algorithm for sparse matrices")
        ));
        assert!(!looks_like_cs(None, Some("hep-th"), None, None));
        assert!(!looks_like_cs(None, Some("cstat"), None, None));
        assert!(!looks_like_cs(
            None,
            None,
            Some("Phys. Rev. E"),
            Some("Active matter")
        ));
        assert!(!looks_like_cs(None, None, None, None));
    }

    #[test]
    fn a_doi_asks_crossref_then_datacite_on_a_miss_then_s2_then_openalex_if_gaps() {
        let plan = plan_identifier_sources(&target(Some("10.1/x"), None, None));
        assert_eq!(
            sources(&plan),
            vec![
                (Source::Crossref, When::Always),
                (Source::DataCite, When::OnMiss),
                (Source::SemanticScholar, When::Always),
                (Source::OpenAlex, When::IfBibliographyMissing),
            ]
        );
        // With an arXiv id too, the DOI branch is taken and arXiv not asked.
        let both = plan_identifier_sources(&target(Some("10.1/x"), Some("2401.1"), None));
        assert_eq!(sources(&both), sources(&plan));
    }

    #[test]
    fn a_physics_doi_also_asks_inspire() {
        let mut paper = target(Some("10.1103/physrevlett.78.2690"), None, None);
        paper.venue = Some("Phys. Rev. Lett.".into());
        let plan = plan_identifier_sources(&paper);
        assert_eq!(
            sources(&plan),
            vec![
                (Source::Crossref, When::Always),
                (Source::DataCite, When::OnMiss),
                (Source::SemanticScholar, When::Always),
                (Source::Inspire, When::Always),
                (Source::OpenAlex, When::IfBibliographyMissing),
            ]
        );
        // An old-style hep id says physics on its own.
        let legacy = target(Some("10.1/x"), Some("hep-th/9901001"), None);
        assert!(
            sources(&plan_identifier_sources(&legacy)).contains(&(Source::Inspire, When::Always))
        );
        // A new-style id says nothing, but the stamp's category the scan
        // kept on the record does - and what arXiv says this run wins.
        let mut stamped = target(Some("10.1/x"), Some("2401.12345"), None);
        assert!(!target_is_physics(&stamped, None));
        stamped.arxiv_category = Some("hep-th".into());
        assert!(target_is_physics(&stamped, None));
        assert!(!target_is_cs(&stamped, None));
        assert!(
            sources(&plan_identifier_sources(&stamped)).contains(&(Source::Inspire, When::Always))
        );
        assert!(!target_is_physics(&stamped, Some("cs.LG")));
        assert!(target_is_cs(&stamped, Some("cs.LG")));
        stamped.arxiv_category = Some("cs.CL".into());
        assert!(target_is_cs(&stamped, None));
        assert!(!target_is_physics(&stamped, None));
    }

    #[test]
    fn an_arxiv_id_alone_asks_arxiv_first_then_s2_then_openalex_if_no_venue() {
        let plan = plan_identifier_sources(&target(None, Some("2401.12345"), None));
        assert_eq!(
            sources(&plan),
            vec![
                (Source::Arxiv, When::Always),
                (Source::SemanticScholar, When::Always),
                (Source::OpenAlex, When::IfVenueMissing),
            ]
        );
        let physics = plan_identifier_sources(&target(None, Some("quant-ph/0001001"), None));
        assert_eq!(
            sources(&physics),
            vec![
                (Source::Arxiv, When::Always),
                (Source::Inspire, When::Always),
                (Source::SemanticScholar, When::Always),
                (Source::OpenAlex, When::IfVenueMissing),
            ]
        );
        // What follows once arXiv has supplied a journal DOI: the DOI steps.
        assert_eq!(
            sources(&doi_steps(false)),
            sources(&plan_identifier_sources(&target(
                Some("10.1/x"),
                None,
                None
            )))
        );
    }

    #[test]
    fn an_isbn_alone_asks_open_library_and_nothing_asks_nothing() {
        let plan = plan_identifier_sources(&target(None, None, Some("9780306406157")));
        assert_eq!(sources(&plan), vec![(Source::OpenLibrary, When::Always)]);
        assert!(plan_identifier_sources(&target(None, None, None)).is_empty());
        // dblp is never an identifier source.
        assert!(
            !plan_identifier_sources(&target(Some("10.1/x"), Some("cs/0001001"), None))
                .iter()
                .any(|s| s.source == Source::Dblp)
        );
    }

    #[test]
    fn conditional_steps_read_what_is_known_so_far() {
        let mut known = Known::of(&target(Some("10.1/x"), None, None));
        assert!(known.applies(When::Always, &Consulted::Skipped));
        assert!(known.applies(When::OnMiss, &Consulted::Missing));
        assert!(!known.applies(When::OnMiss, &Consulted::Skipped));
        assert!(!known.applies(When::OnMiss, &Consulted::GivenUp));
        assert!(!known.applies(
            When::OnMiss,
            &Consulted::Found(candidate(MetaSource::Crossref, "x", None))
        ));
        assert!(known.applies(When::IfVenueMissing, &Consulted::Missing));
        assert!(known.applies(When::IfBibliographyMissing, &Consulted::Missing));

        let mut full = candidate(MetaSource::Crossref, "x", None);
        full.venue = Some("Venue".into());
        known.absorb(&full);
        assert!(!known.applies(When::IfVenueMissing, &Consulted::Missing));
        assert!(
            known.applies(When::IfBibliographyMissing, &Consulted::Missing),
            "pages and volume still missing"
        );
        full.pages = Some("1-2".into());
        full.volume = Some("3".into());
        known.absorb(&full);
        assert!(!known.applies(When::IfBibliographyMissing, &Consulted::Missing));

        // What the paper already has counts too.
        let mut paper = target(Some("10.1/x"), None, None);
        paper.venue = Some("Venue".into());
        paper.has_pages = true;
        paper.has_volume = true;
        assert!(!Known::of(&paper).applies(When::IfBibliographyMissing, &Consulted::Missing));
    }

    #[test]
    fn every_source_has_a_host_the_limiter_knows() {
        use crate::library::ratelimit::minimum_interval;
        for source in ALL_SOURCES {
            assert_ne!(service_name(source.host()), source.host(), "{source:?}");
            assert!(minimum_interval(source.host(), false) > Duration::ZERO);
        }
    }

    #[test]
    fn an_exact_title_is_applied_whatever_the_year() {
        let merged = vec![
            candidate(MetaSource::Crossref, "Something Else Entirely", Some(2020)),
            candidate(
                MetaSource::Openalex,
                "Nonequilibrium Equality for Free-Energy Differences",
                None,
            ),
        ];
        let outcome = apply_title_candidates(
            "Nonequilibrium equality for free energy differences",
            Some(1997),
            merged,
            &[],
        );
        match outcome {
            TitleOutcome::Apply(c) => assert_eq!(c.source, MetaSource::Openalex),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_near_title_is_applied_only_with_the_same_year() {
        // Six words shared of seven: 0.857, just over the threshold.
        let paper = "Nonequilibrium equality for free energy differences";
        let near = "Nonequilibrium equality for free energy differences revisited";
        let same_year = vec![candidate(MetaSource::Crossref, near, Some(1997))];
        assert!(matches!(
            apply_title_candidates(paper, Some(1997), same_year, &[]),
            TitleOutcome::Apply(_)
        ));
        let other_year = vec![candidate(MetaSource::Crossref, near, Some(1998))];
        assert!(matches!(
            apply_title_candidates(paper, Some(1997), other_year, &[]),
            TitleOutcome::Park(ref parked) if parked.len() == 1
        ));
        let no_year = vec![candidate(MetaSource::Crossref, near, Some(1997))];
        assert!(matches!(
            apply_title_candidates(paper, None, no_year, &[]),
            TitleOutcome::Park(_)
        ));
    }

    #[test]
    fn the_first_sure_match_wins_and_exact_beats_near() {
        let paper = "Active matter at the mesoscale a review";
        let merged = vec![
            candidate(
                MetaSource::Crossref,
                "Active matter at the mesoscale a reviews",
                Some(2013),
            ),
            candidate(
                MetaSource::Openalex,
                "Active Matter at the Mesoscale: A Review",
                Some(2013),
            ),
        ];
        match apply_title_candidates(paper, Some(2013), merged, &[]) {
            TitleOutcome::Apply(c) => assert_eq!(c.source, MetaSource::Openalex, "exact first"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn short_titles_are_parked_even_on_an_exact_match() {
        assert!(!title_is_sure_enough("Introduction"));
        assert!(!title_is_sure_enough("notes"));
        assert!(!title_is_sure_enough("Deep Learning"));
        assert!(
            !title_is_sure_enough("A B C D"),
            "three words but too short"
        );
        assert!(title_is_sure_enough("Deep Learning Now"));
        assert!(title_is_sure_enough(
            "Nonequilibrium equality for free energy differences"
        ));
        for short in ["Introduction", "notes", "Deep learning"] {
            let exact = vec![candidate(MetaSource::Crossref, short, Some(2020))];
            match apply_title_candidates(short, Some(2020), exact, &[]) {
                TitleOutcome::Park(parked) => assert_eq!(parked.len(), 1, "{short}"),
                other => panic!("{short}: {other:?}"),
            }
        }
        // Long enough: the same exact match is applied.
        let exact = vec![candidate(
            MetaSource::Crossref,
            "Deep Learning Now",
            Some(2020),
        )];
        assert!(matches!(
            apply_title_candidates("Deep learning now", Some(2020), exact, &[]),
            TitleOutcome::Apply(_)
        ));
    }

    #[test]
    fn weak_matches_are_parked_capped_at_five_and_declined_ones_dropped() {
        let merged: Vec<LookupCandidateDto> = (0..7)
            .map(|i| {
                candidate(
                    MetaSource::Openalex,
                    &format!("Unrelated paper number {i}"),
                    Some(2000 + i),
                )
            })
            .collect();
        match apply_title_candidates("The paper I have", Some(2020), merged.clone(), &[]) {
            TitleOutcome::Park(parked) => {
                assert_eq!(parked.len(), PARK_LIMIT);
                assert_eq!(
                    parked[0].title, "Unrelated paper number 0",
                    "ties keep rank order"
                );
            }
            other => panic!("{other:?}"),
        }
        // The likeliest title is parked first, whatever its source's rank.
        let mixed = vec![
            candidate(MetaSource::Crossref, "Something else again", Some(2020)),
            candidate(
                MetaSource::Openalex,
                "The paper I have, revisited and extended",
                None,
            ),
        ];
        match apply_title_candidates("The paper I have", Some(2020), mixed, &[]) {
            TitleOutcome::Park(parked) => {
                assert_eq!(parked[0].source, MetaSource::Openalex);
                assert_eq!(parked.len(), 2);
            }
            other => panic!("{other:?}"),
        }
        let declined: Vec<String> = merged.iter().map(titlematch::identity).collect();
        assert_eq!(
            apply_title_candidates("The paper I have", Some(2020), merged.clone(), &declined),
            TitleOutcome::Nothing,
            "every one was declined before"
        );
        assert!(without_declined(merged.clone(), &declined).is_empty());
        assert_eq!(
            without_declined(merged.clone(), &declined[..2]).len(),
            5,
            "the manual search drops the declined ones the same way"
        );
        // Even an exact match the user declined is not applied again.
        let exact = vec![candidate(
            MetaSource::Crossref,
            "The paper I have today",
            Some(2020),
        )];
        let declined = vec![titlematch::identity(&exact[0])];
        assert_eq!(
            apply_title_candidates("The paper I have today", Some(2020), exact, &declined),
            TitleOutcome::Nothing
        );
        assert_eq!(
            apply_title_candidates("The paper I have", None, Vec::new(), &[]),
            TitleOutcome::Nothing
        );
    }

    #[test]
    fn arxiv_dois_are_not_journal_dois() {
        assert!(is_arxiv_doi("10.48550/arXiv.2401.12345"));
        assert!(!is_arxiv_doi("10.1103/physrevlett.78.2690"));
    }

    #[test]
    fn cache_saves_are_throttled_and_the_flush_writes_the_rest() {
        let dir = tempfile::tempdir().expect("temp dir");
        let chain = chain_in(dir.path());
        assert_eq!(chain.saves(), 0);
        for i in 0..50 {
            chain.remember(
                format!("doi:10.1/{i}"),
                Some(candidate(MetaSource::Crossref, "T", None)),
            );
            chain.remember(format!("s2:doi:10.1/{i}"), None);
        }
        assert_eq!(chain.saves(), 1, "the first answer at once, the rest wait");
        assert!(dir.path().join(lookup::CACHE_FILE).is_file());
        chain.flush();
        assert_eq!(chain.saves(), 2);
        chain.flush();
        assert_eq!(chain.saves(), 2, "nothing unsaved");
        let reloaded = LookupCache::load(dir.path());
        assert_eq!(reloaded.len(), 50);
        assert_eq!(reloaded.miss_count(), 50);
    }

    #[test]
    fn hosts_given_up_on_make_the_outcome_incomplete_without_a_request() {
        let dir = tempfile::tempdir().expect("temp dir");
        let chain = chain_in(dir.path());
        give_up_on_every_host(&chain);
        let paper = target(Some("10.1/x"), None, None);
        let outcome = block_on(chain.lookup_by_identifiers(&paper));
        assert!(outcome.candidates.is_empty());
        assert!(outcome.errors.is_empty(), "nothing was sent to fail");
        assert!(!outcome.complete, "not stamped as looked up");
        assert_eq!(outcome.edges, None);

        let mut titled = target(None, None, None);
        titled.title = Some("A title long enough to search".into());
        let search = block_on(chain.search_by_title_everywhere(&titled));
        assert!(search.candidates.is_empty());
        assert!(search.errors.is_empty());
        assert!(!search.complete);
        assert!(
            chain.shared.cache.lock().is_empty() && chain.saves() == 0,
            "an incomplete search is not cached"
        );

        // Nothing to ask with is complete, trivially.
        assert!(block_on(chain.lookup_by_identifiers(&target(None, None, None))).complete);
    }

    #[test]
    fn turning_lookup_off_stops_the_chain_before_any_request() {
        let dir = tempfile::tempdir().expect("temp dir");
        let chain = chain_in(dir.path());
        chain.shared.settings.lock().enabled = false;
        let outcome = block_on(chain.lookup_by_identifiers(&target(Some("10.1/x"), None, None)));
        assert!(outcome.candidates.is_empty());
        assert!(
            outcome.errors.is_empty(),
            "a request would have failed the enabled check with an error"
        );
        assert!(!outcome.complete);
        assert!(!chain.enabled());
        let mut titled = target(None, None, None);
        titled.title = Some("A title long enough to search".into());
        let search = block_on(chain.search_by_title_everywhere(&titled));
        assert!(search.candidates.is_empty() && search.errors.is_empty() && !search.complete);
    }

    #[test]
    fn a_chain_served_from_the_cache_queues_edges_once_and_the_category_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let chain = chain_in(dir.path());
        give_up_on_every_host(&chain);
        let doi = CitationIdDto::Doi("10.1/x".into());
        let mut crossref = candidate(MetaSource::Crossref, "Found", Some(2020));
        crossref.venue = Some("V".into());
        crossref.pages = Some("1-2".into());
        crossref.volume = Some("3".into());
        crossref.doi = Some("10.1/x".into());
        let s2 = candidate(MetaSource::SemanticScholar, "Found", Some(2020));
        {
            let mut cache = chain.shared.cache.lock();
            cache.insert(LookupCache::doi_key("10.1/x"), crossref);
            cache.insert(LookupCache::source_key("s2", &doi), s2);
        }
        let paper = target(Some("10.1/x"), None, None);
        let outcome = block_on(chain.lookup_by_identifiers(&paper));
        assert_eq!(outcome.candidates.len(), 2);
        assert!(
            outcome.complete,
            "every step answered from the cache or did not apply"
        );
        assert_eq!(
            outcome.edges,
            Some(EdgeJob {
                id: doi.clone(),
                semantic_scholar: true,
                openalex: false,
            })
        );
        // Claimed: a second chain does not queue it again.
        let again = block_on(chain.lookup_by_identifiers(&paper));
        assert_eq!(again.edges, None);
        // The edge task releases the claim even when lookup is off.
        chain.shared.settings.lock().enabled = false;
        block_on(chain.fetch_edges(outcome.edges.into_iter().collect()));
        assert!(chain.shared.edges_in_flight.lock().is_empty());
        chain.shared.settings.lock().enabled = true;
        assert!(block_on(chain.lookup_by_identifiers(&paper))
            .edges
            .is_some());
        // Once the references are cached, no job is queued without a key.
        chain.shared.edges_in_flight.lock().clear();
        chain
            .shared
            .cache
            .lock()
            .insert_links(LookupCache::refs_key(&doi), Vec::new());
        assert_eq!(block_on(chain.lookup_by_identifiers(&paper)).edges, None);
        // With a key the citations are wanted too.
        chain.shared.settings.lock().s2_key = "key".into();
        assert!(block_on(chain.lookup_by_identifiers(&paper))
            .edges
            .is_some());

        // A cached arXiv answer carries its category: the physics decision
        // is the same on the second run as on the first.
        let arxiv = candidate(MetaSource::Arxiv, "Preprint", Some(2024));
        {
            let mut cache = chain.shared.cache.lock();
            cache.insert(LookupCache::arxiv_key("2401.1"), arxiv);
            cache.insert_category(LookupCache::arxiv_key("2401.1"), "hep-th".into());
        }
        let mut category = None;
        let found = block_on(chain.ask(Source::Arxiv, None, Some("2401.1"), None, &mut category));
        assert!(matches!(found, Consulted::Found(_)));
        assert_eq!(category.as_deref(), Some("hep-th"));
        let _ = PaperKind::Article;
    }

    /// A batch host that counts, and turns lookup off after `allow` turns.
    struct Counting {
        allow: u32,
        turns: AtomicU32,
        progress: Mutex<Vec<(u32, u32)>>,
    }

    impl BatchTurns for Counting {
        fn enabled(&self) -> bool {
            self.turns.load(Ordering::SeqCst) < self.allow
        }

        fn turn(
            &mut self,
            _kind: ChainKind,
            _target: LookupTarget,
        ) -> impl Future<Output = Turn> + Send {
            self.turns.fetch_add(1, Ordering::SeqCst);
            async { Turn::Applied }
        }

        fn progress(&self, done: u32, total: u32) {
            self.progress.lock().push((done, total));
        }
    }

    #[test]
    fn a_batch_stops_when_lookup_is_turned_off_between_turns() {
        let plan = LookupPlan {
            targets: (0..3)
                .map(|i| {
                    (
                        ChainKind::Identifier,
                        target(Some(&format!("10.1/{i}")), None, None),
                    )
                })
                .collect(),
            skipped: 2,
        };
        let mut host = Counting {
            allow: 1,
            turns: AtomicU32::new(0),
            progress: Mutex::new(Vec::new()),
        };
        let batch = block_on(drive_batch(plan.clone(), &mut host));
        assert_eq!(batch.attempted, 1);
        assert_eq!(batch.applied, 1);
        assert_eq!(batch.skipped, 2 + 2, "the two left plus the plan's own");
        assert_eq!(*host.progress.lock(), vec![(0, 3), (1, 3)]);

        let mut host = Counting {
            allow: 10,
            turns: AtomicU32::new(0),
            progress: Mutex::new(Vec::new()),
        };
        let batch = block_on(drive_batch(plan, &mut host));
        assert_eq!((batch.attempted, batch.applied, batch.skipped), (3, 3, 2));
        assert_eq!(
            host.progress.lock().len(),
            4,
            "one report per step plus the start"
        );
        let empty = block_on(drive_batch(LookupPlan::default(), &mut host));
        assert_eq!(empty, LookupBatchDto::default());
    }
}
