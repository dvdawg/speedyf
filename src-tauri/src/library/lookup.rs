//! Online metadata lookup (projects.md sec. 4, phase 2).
//!
//! This is the first code in SpeedyF that talks to the network, so the terms
//! are stated here. Nothing leaves the machine unless **Settings -> Look up
//! paper metadata online** is on; every entry point checks that before it
//! builds a request. What is sent is only an identifier - a DOI to Crossref
//! and OpenAlex, an arXiv id to arXiv - plus, for the `search_*_by_title`
//! functions, which the UI offers on request for a paper that has no
//! identifier, the title itself to OpenAlex, Crossref and arXiv. The
//! optional contact address (Settings -> Contact for lookups) goes to
//! Crossref in the User-Agent and to OpenAlex as `mailto`, which is each
//! service's polite-pool convention. No file content, path, or anything else
//! from the library is ever sent. The other services (DataCite, Semantic
//! Scholar, INSPIRE, dblp, Open Library) live under [`super::sources`], each
//! with the same statement at the top of its module.
//!
//! Requests go through `reqwest` with rustls from the Rust side; the
//! webview's CSP is untouched. Every request leaves through a
//! [`HostLimiter`], which queues and spaces requests per host so no service
//! sees a burst and a rate-limit answer pauses the queue rather than failing
//! the lookup. Responses are parsed by pure functions (`parse_*`) that take
//! the body text, so the parsing is tested without a network. Parsed
//! candidates are cached by identifier in `lookup-cache.json`
//! ([`LookupCache`]), and so are misses, for a week, so a rescan does not
//! ask again for what nobody has; the cache is disposable, and a corrupt one
//! is discarded. The chain that decides which service to ask, and what to do
//! with the answers, is [`super::chain`].

use crate::engine::citation::fold_arxiv_doi;
use crate::engine::types::CitationIdDto;
use crate::errors::{AppError, AppResult};
use crate::library::ratelimit::{network_error, service_name, HostLimiter};
use crate::library::types::{Author, LookupCandidateDto, MetaSource, PaperKind};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::header::USER_AGENT;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USER_AGENT_VALUE: &str = "SpeedyF/0.1.0 (local PDF reader)";
const CROSSREF_WORKS: &str = "https://api.crossref.org/works/";
const ARXIV_QUERY: &str = "https://export.arxiv.org/api/query";
const OPENALEX_WORKS: &str = "https://api.openalex.org/works";
/// DataCite's prefix for the DOIs it registers for arXiv; OpenAlex files
/// arXiv papers under these.
const ARXIV_DOI_PREFIX: &str = "10.48550/arxiv.";
/// How many OpenAlex ids one `works?filter=openalex_id:...` request resolves.
const OPENALEX_BATCH: usize = 50;
const TIMEOUT: Duration = Duration::from_secs(15);
/// How many candidates a title search returns at most.
pub(crate) const TITLE_RESULTS: usize = 5;
const DISABLED_MESSAGE: &str =
    "Online lookup is off (turn on Look up paper metadata online in Settings)";
/// The cache file, inside the library's data directory.
pub const CACHE_FILE: &str = "lookup-cache.json";
/// How long a cached miss stands before the identifier is asked for again.
pub const MISS_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a title search's cached candidates stand before the title is
/// searched for again.
pub const LIST_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Everything except the RFC 3986 unreserved characters and `/`, which every
/// DOI carries and Crossref accepts raw. Anything else a DOI may contain
/// (`()<>;:#?` are all seen in the wild) is escaped.
pub(crate) const DOI_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// The settings this module reads. `enabled` is the opt-in; `contact` is
/// the optional address for the polite pools, empty by default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupSettings {
    pub enabled: bool,
    pub contact: String,
    /// Optional Semantic Scholar API key: a better rate limit, nothing else.
    pub s2_key: String,
}

impl LookupSettings {
    pub(crate) fn contact(&self) -> Option<&str> {
        let contact = self.contact.trim();
        (!contact.is_empty()).then_some(contact)
    }

    pub(crate) fn s2_key(&self) -> Option<&str> {
        let key = self.s2_key.trim();
        (!key.is_empty()).then_some(key)
    }
}

/// The one gate every request passes. Kept synchronous so it can be checked
/// before any work - including the status-bar indicator - starts. Public so
/// a command can refuse before consulting the cache: a cache hit would make
/// no request, but the user turned lookups off and expects nothing to run.
pub fn ensure_enabled(settings: &LookupSettings) -> AppResult<()> {
    if settings.enabled {
        Ok(())
    } else {
        Err(AppError::Internal(DISABLED_MESSAGE.to_string()))
    }
}

/// A client for every lookup: 15 s timeout, gzip, and a User-Agent that says
/// what we are. Build one and keep it; `reqwest::Client` is cheap to clone.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .gzip(true)
        .user_agent(USER_AGENT_VALUE)
        .build()
        .unwrap_or_else(|error| {
            log::warn!("lookup: cannot configure the HTTP client ({error}); using defaults");
            reqwest::Client::new()
        })
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

/// Crossref `works/{doi}`. Sends the DOI and nothing else. `Ok(None)` when
/// Crossref has no record (404) or the record has no title.
pub async fn fetch_by_doi(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    doi: &str,
) -> AppResult<Option<LookupCandidateDto>> {
    ensure_enabled(settings)?;
    let doi = normalize_doi(doi);
    if doi.is_empty() {
        return Ok(None);
    }
    let request = crossref_request(settings, client.get(crossref_url(&doi)));
    let Some(body) = fetch_text(client, limiter, request).await? else {
        return Ok(None);
    };
    Ok(parse_crossref_work(&body))
}

/// One arXiv entry: the candidate, and the primary category (`hep-th`,
/// `cs.LG`), which is what tells the chain whether INSPIRE or dblp is worth
/// asking next.
#[derive(Clone, Debug, PartialEq)]
pub struct ArxivRecord {
    pub candidate: LookupCandidateDto,
    pub primary_category: Option<String>,
}

/// arXiv export API `query?id_list={id}`. Sends the arXiv id (sans version,
/// so the newest version answers - it carries the journal reference when
/// there is one) and nothing else. `Ok(None)` when the feed is empty.
pub async fn fetch_by_arxiv(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    arxiv_id: &str,
) -> AppResult<Option<ArxivRecord>> {
    ensure_enabled(settings)?;
    let id = normalize_arxiv_id(arxiv_id);
    if id.is_empty() {
        return Ok(None);
    }
    let request = client
        .get(ARXIV_QUERY)
        .query(&[("id_list", id.as_str()), ("max_results", "1")]);
    let Some(body) = fetch_text(client, limiter, request).await? else {
        return Ok(None);
    };
    Ok(parse_arxiv_entries(&body).into_iter().next())
}

/// OpenAlex `works?search={title}`. Sends the title; the UI offers it only
/// on request. Up to five candidates, in API order.
pub async fn search_by_title(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    title: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    ensure_enabled(settings)?;
    let title = collapse_whitespace(title);
    if title.is_empty() {
        return Ok(Vec::new());
    }
    let per_page = TITLE_RESULTS.to_string();
    let mut query = vec![("search", title.as_str()), ("per-page", per_page.as_str())];
    if let Some(contact) = settings.contact() {
        query.push(("mailto", contact));
    }
    let request = client.get(OPENALEX_WORKS).query(&query);
    let Some(body) = fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_openalex_search(&body);
    candidates.truncate(TITLE_RESULTS);
    Ok(candidates)
}

/// Crossref `works?query.bibliographic={title}&rows=5`. Sends the title (and
/// the contact, in the User-Agent, when one is set). Up to five candidates
/// in Crossref's relevance order.
pub async fn search_crossref_by_title(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    title: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    ensure_enabled(settings)?;
    let title = collapse_whitespace(title);
    if title.is_empty() {
        return Ok(Vec::new());
    }
    let rows = TITLE_RESULTS.to_string();
    let request = client.get(CROSSREF_WORKS.trim_end_matches('/')).query(&[
        ("query.bibliographic", title.as_str()),
        ("rows", rows.as_str()),
    ]);
    let request = crossref_request(settings, request);
    let Some(body) = fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_crossref_search(&body);
    candidates.truncate(TITLE_RESULTS);
    Ok(candidates)
}

/// arXiv export API `query?search_query=ti:"{title}"&max_results=5`. Sends
/// the title's words - punctuation is dropped, since it is query syntax to
/// arXiv's Lucene - and nothing else. Up to five candidates in arXiv's
/// relevance order.
pub async fn search_arxiv_by_title(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    title: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    ensure_enabled(settings)?;
    let words = query_words(title);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let search = format!("ti:\"{words}\"");
    let max_results = TITLE_RESULTS.to_string();
    let request = client.get(ARXIV_QUERY).query(&[
        ("search_query", search.as_str()),
        ("max_results", max_results.as_str()),
    ]);
    let Some(body) = fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_arxiv_atom(&body);
    candidates.truncate(TITLE_RESULTS);
    Ok(candidates)
}

/// An OpenAlex work fetched by identifier: the candidate (`None` when the
/// record has no title) and the OpenAlex ids of the works it cites, for
/// [`resolve_openalex_works`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenAlexRecord {
    pub candidate: Option<LookupCandidateDto>,
    pub referenced_works: Vec<String>,
}

/// OpenAlex `works/https://doi.org/{doi}`, for a DOI or for an arXiv id,
/// which OpenAlex files under the DOI DataCite registers for it
/// (`10.48550/arxiv.{id}`). Sends that DOI (and `mailto` when a contact is
/// set). `Ok(None)` on 404.
pub async fn fetch_openalex_record(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    id: &CitationIdDto,
) -> AppResult<Option<OpenAlexRecord>> {
    ensure_enabled(settings)?;
    let Some(doi) = openalex_doi(id) else {
        return Ok(None);
    };
    let Some(body) = fetch_openalex_work(client, limiter, settings, &doi).await? else {
        return Ok(None);
    };
    Ok(Some(OpenAlexRecord {
        candidate: parse_openalex_work(&body),
        referenced_works: parse_openalex_referenced_works(&body),
    }))
}

/// The DOI OpenAlex files a work under: its own, or for a preprint the
/// arXiv DOI. `None` for an empty identifier.
fn openalex_doi(id: &CitationIdDto) -> Option<String> {
    let doi = match id {
        CitationIdDto::Doi(doi) => normalize_doi(doi),
        CitationIdDto::ArXiv(arxiv) => {
            let arxiv = normalize_arxiv_id(arxiv);
            if arxiv.is_empty() {
                return None;
            }
            arxiv_doi(&arxiv)
        }
    };
    non_empty(doi)
}

/// OpenAlex work ids (`W123`, a work's `referenced_works`) resolved to
/// identifiers in batches of fifty through
/// `works?filter=openalex_id:W1|W2|...&select=doi,ids&per-page=50`. Sends the
/// OpenAlex ids, which name works, not the user. A work without a DOI is
/// left out; one whose DOI is an arXiv DOI comes back as the arXiv id, which
/// is how the library names preprints.
pub async fn resolve_openalex_works(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    works: &[String],
) -> AppResult<Vec<CitationIdDto>> {
    ensure_enabled(settings)?;
    let mut cited = Vec::with_capacity(works.len());
    for batch in works.chunks(OPENALEX_BATCH) {
        let filter = format!("openalex_id:{}", batch.join("|"));
        let per_page = OPENALEX_BATCH.to_string();
        let mut query = vec![
            ("filter", filter.as_str()),
            ("select", "doi,ids"),
            ("per-page", per_page.as_str()),
        ];
        if let Some(contact) = settings.contact() {
            query.push(("mailto", contact));
        }
        let request = client.get(OPENALEX_WORKS).query(&query);
        if let Some(body) = fetch_text(client, limiter, request).await? {
            cited.extend(parse_openalex_dois(&body));
        }
    }
    let mut seen = std::collections::HashSet::new();
    cited.retain(|id| seen.insert(id.clone()));
    Ok(cited)
}

/// `works/https://doi.org/{doi}` as text, `None` on 404.
async fn fetch_openalex_work(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    doi: &str,
) -> AppResult<Option<String>> {
    let url = format!(
        "{OPENALEX_WORKS}/https://doi.org/{}",
        utf8_percent_encode(doi, DOI_SEGMENT)
    );
    let mut request = client.get(url);
    if let Some(contact) = settings.contact() {
        request = request.query(&[("mailto", contact)]);
    }
    fetch_text(client, limiter, request).await
}

fn crossref_url(doi: &str) -> String {
    format!("{CROSSREF_WORKS}{}", utf8_percent_encode(doi, DOI_SEGMENT))
}

/// The DOI DataCite registers for an arXiv id.
fn arxiv_doi(arxiv_id: &str) -> String {
    format!("{ARXIV_DOI_PREFIX}{}", arxiv_id.to_ascii_lowercase())
}

/// Crossref's polite pool: a contact in the User-Agent. Only when one is set.
fn crossref_request(
    settings: &LookupSettings,
    request: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    match crossref_user_agent(settings) {
        Some(user_agent) => request.header(USER_AGENT, user_agent),
        None => request,
    }
}

fn crossref_user_agent(settings: &LookupSettings) -> Option<String> {
    settings
        .contact()
        .map(|contact| format!("SpeedyF/0.1.0 (mailto:{contact})"))
}

/// A title reduced to the words a search engine tokenizes: letters, digits
/// and single spaces. Quotes, colons and brackets are query syntax to arXiv
/// and INSPIRE, and the services match on words anyway.
pub(crate) fn query_words(title: &str) -> String {
    let spaced: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    collapse_whitespace(&spaced)
}

/// Sends a request through the limiter and reads the body. 404 is `None`;
/// any other non-2xx status or transport failure is an error naming the
/// service. This is the helper every source module uses.
pub(crate) async fn fetch_text(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    request: reqwest::RequestBuilder,
) -> AppResult<Option<String>> {
    let response = limiter.send(client, request).await?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let service = service_name(response.url().host_str().unwrap_or_default());
    if !status.is_success() {
        return Err(AppError::Io(format!("{service} answered HTTP {status}")));
    }
    let body = response
        .text()
        .await
        .map_err(|error| network_error(&service, &error))?;
    Ok(Some(body))
}

// ---------------------------------------------------------------------------
// Crossref
// ---------------------------------------------------------------------------

/// Parses a Crossref `works/{doi}` response. Accepts the usual
/// `{"message": {...}}` envelope or a bare work object. `None` when the JSON
/// is not a work or the work has no title - a candidate without a title has
/// nothing to show and would blank the paper's own title if applied.
pub fn parse_crossref_work(json: &str) -> Option<LookupCandidateDto> {
    let root: Value = serde_json::from_str(json).ok()?;
    let work = match root.get("message") {
        Some(message) if message.is_object() => message,
        _ => &root,
    };
    crossref_work(work)
}

/// Parses a Crossref `works?query.bibliographic=` response: one candidate
/// per titled item of `message.items`, in Crossref's order. Not JSON, or no
/// items -> empty.
pub fn parse_crossref_search(json: &str) -> Vec<LookupCandidateDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.pointer("/message/items")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(crossref_work).collect())
        .unwrap_or_default()
}

fn crossref_work(work: &Value) -> Option<LookupCandidateDto> {
    if !work.is_object() {
        return None;
    }
    let title = work
        .get("title")
        .and_then(text_field)
        .map(|raw| crossref_title(&raw))
        .and_then(non_empty)?;
    let authors = work
        .get("author")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(crossref_author).collect())
        .unwrap_or_default();
    let kind = work
        .get("type")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "journal-article" => PaperKind::Article,
            "proceedings-article" => PaperKind::Inproceedings,
            "book" | "monograph" | "edited-book" => PaperKind::Book,
            "dissertation" => PaperKind::Thesis,
            _ => PaperKind::Misc,
        });
    Some(LookupCandidateDto {
        source: MetaSource::Crossref,
        title,
        authors,
        year: crossref_year(work),
        venue: work.get("container-title").and_then(text_field),
        volume: work.get("volume").and_then(text_field),
        issue: work.get("issue").and_then(text_field),
        pages: work.get("page").and_then(text_field),
        publisher: work.get("publisher").and_then(text_field),
        abstract_: work
            .get("abstract")
            .and_then(Value::as_str)
            .map(strip_jats)
            .and_then(non_empty),
        kind,
        doi: work
            .get("DOI")
            .and_then(Value::as_str)
            .map(normalize_doi)
            .and_then(non_empty),
        arxiv: None,
        isbn: None,
    })
}

/// `{family, given}`; an entry with only `name` (a consortium) becomes a
/// family name; an entry with neither is skipped.
fn crossref_author(entry: &Value) -> Option<Author> {
    let given = entry.get("given").and_then(text_field).unwrap_or_default();
    if let Some(family) = entry.get("family").and_then(text_field) {
        return Some(Author { family, given });
    }
    let name = entry.get("name").and_then(text_field)?;
    Some(Author {
        family: name,
        given: String::new(),
    })
}

/// `issued` first; Crossref leaves it as `[[null]]` for some records, so the
/// print, online and deposit dates follow.
fn crossref_year(work: &Value) -> Option<i32> {
    ["issued", "published-print", "published-online", "created"]
        .iter()
        .find_map(|key| {
            work.get(*key)?
                .get("date-parts")?
                .get(0)?
                .get(0)?
                .as_i64()
                .and_then(|year| i32::try_from(year).ok())
        })
}

/// Crossref titles carry inline JATS and MathML now and then
/// (`H<sub>2</sub>O`, `<i>in vivo</i>`, `<mml:math alttext="...">`). The app
/// typesets TeX between `$` signs, so scripts become `$_{2}$` / `$^{2}$`,
/// MathML becomes its `alttext` (as `$...$`) or, without one, its text, and
/// every other tag is dropped. Entities are decoded and whitespace collapsed.
pub(crate) fn crossref_title(raw: &str) -> String {
    if !raw.contains('<') && !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = &rest[start + 1..start + len];
        let after = &rest[start + len + 1..];
        let name = tag_local_name(tag);
        let closing = tag.starts_with('/');
        rest = after;
        if closing || tag.ends_with('/') {
            continue;
        }
        match name {
            "math" => {
                let (inner, tail) = split_element(after, "math");
                rest = tail;
                let alt = attribute(tag, "alttext").map(|alt| decode_entities(&alt));
                match alt.map(|alt| collapse_whitespace(&alt)) {
                    Some(alt) if !alt.is_empty() => {
                        if alt.contains('$') {
                            out.push_str(&alt);
                        } else {
                            out.push('$');
                            out.push_str(&alt);
                            out.push('$');
                        }
                    }
                    _ => out.push_str(&strip_tags(inner)),
                }
            }
            "sub" | "sup" => {
                let (inner, tail) = split_element(after, name);
                rest = tail;
                let text = collapse_whitespace(&decode_entities(&strip_tags(inner)));
                if text.is_empty() {
                    continue;
                }
                if text.contains(['$', '{', '}', '\\']) {
                    out.push_str(&text);
                } else {
                    out.push_str(if name == "sub" { "$_{" } else { "$^{" });
                    out.push_str(&text);
                    out.push_str("}$");
                }
            }
            _ => {}
        }
    }
    out.push_str(rest);
    collapse_whitespace(&decode_entities(&out))
}

/// `jats:sub` / `/mml:math attr="..."` -> `sub` / `math`.
fn tag_local_name(tag: &str) -> &str {
    let name = tag
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("");
    name.rsplit(':').next().unwrap_or(name)
}

/// The content of an element whose start tag was just read, and the text
/// after its end tag. Same-named elements nested inside are counted, so
/// `<sup>a<sup>b</sup></sup>` closes at the outer end tag.
fn split_element<'a>(text: &'a str, local: &str) -> (&'a str, &'a str) {
    let mut depth = 1usize;
    let mut from = 0;
    while let Some(offset) = text[from..].find('<') {
        let at = from + offset;
        let Some(len) = text[at..].find('>') else {
            break;
        };
        let tag = &text[at + 1..at + len];
        if tag_local_name(tag) == local {
            if tag.starts_with('/') {
                depth -= 1;
                if depth == 0 {
                    return (&text[..at], &text[at + len + 1..]);
                }
            } else if !tag.ends_with('/') {
                depth += 1;
            }
        }
        from = at + len + 1;
    }
    (text, "")
}

/// The value of `name="..."` (or single-quoted) in a start tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(offset) = tag[from..].find(name) {
        let at = from + offset;
        from = at + name.len();
        let before_ok = tag[..at].ends_with(char::is_whitespace);
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        if !before_ok {
            continue;
        }
        let body = &rest[1..];
        return body.find(quote).map(|end| body[..end].to_string());
    }
    None
}

/// Crossref abstracts are JATS fragments. Section headings
/// (`<jats:title>Abstract</jats:title>`, `Background`, ...) are dropped with
/// their text, remaining tags are stripped - block elements leave a space,
/// inline ones none - then entities are decoded and whitespace collapsed.
fn strip_jats(raw: &str) -> String {
    collapse_whitespace(&decode_entities(&strip_tags(&drop_title_elements(raw))))
}

fn drop_title_elements(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = find_title_open(rest) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start..];
        match find_title_close(after_open) {
            Some(end) => rest = &after_open[end..],
            // An unterminated heading: nothing after it is trustworthy text.
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// Index of `<jats:title` / `<title` (followed by `>` or an attribute).
fn find_title_open(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = text[from..].find('<') {
        let at = from + offset;
        let tail = &text[at + 1..];
        let local = tail.strip_prefix("jats:").unwrap_or(tail);
        if let Some(after) = local.strip_prefix("title") {
            if after.starts_with('>') || after.starts_with(char::is_whitespace) {
                return Some(at);
            }
        }
        from = at + 1;
    }
    None
}

/// Index just past the first `</jats:title>` / `</title>`.
fn find_title_close(text: &str) -> Option<usize> {
    ["</jats:title>", "</title>"]
        .iter()
        .filter_map(|close| text.find(close).map(|at| at + close.len()))
        .min()
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('>') else {
            // An unterminated tag: drop the remainder rather than show markup.
            return out;
        };
        if is_block_tag(&rest[start + 1..start + len]) {
            out.push(' ');
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Whether the tag (the text between `<` and `>`) separates runs of text.
fn is_block_tag(tag: &str) -> bool {
    let name = tag
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("");
    let local = name.rsplit(':').next().unwrap_or(name);
    matches!(
        local,
        "p" | "sec"
            | "title"
            | "list"
            | "list-item"
            | "def-item"
            | "break"
            | "br"
            | "div"
            | "table-wrap"
            | "tr"
            | "td"
    )
}

/// The XML entities JATS text carries, plus numeric references. Anything
/// unrecognized is left as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let decoded = after
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| decode_entity(&after[1..end]).map(|ch| (ch, end + 1)));
        match decoded {
            Some((ch, len)) => {
                out.push(ch);
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let number = entity.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

// ---------------------------------------------------------------------------
// arXiv
// ---------------------------------------------------------------------------

/// Parses an arXiv export API Atom feed into one candidate per `<entry>`.
/// Entries without a title are skipped, as is the error entry arXiv returns
/// in place of a result for a malformed id (`<id>` under `/api/errors`).
/// A malformed feed yields what was parsed before the fault.
pub fn parse_arxiv_atom(xml: &str) -> Vec<LookupCandidateDto> {
    parse_arxiv_entries(xml)
        .into_iter()
        .map(|record| record.candidate)
        .collect()
}

/// The same, keeping each entry's primary category (`<arxiv:primary_category
/// term="hep-th"/>`, else the first `<category>`).
pub fn parse_arxiv_entries(xml: &str) -> Vec<ArxivRecord> {
    let mut reader = Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut current: Option<ArxivEntry> = None;
    let mut records = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let name = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
                if name == "entry" {
                    current = Some(ArxivEntry::default());
                } else if let Some(entry) = current.as_mut() {
                    entry.note_category(&path, &name, &start);
                }
                path.push(name);
            }
            Ok(Event::Empty(tag)) => {
                if let Some(entry) = current.as_mut() {
                    let name = String::from_utf8_lossy(tag.local_name().as_ref()).into_owned();
                    entry.note_category(&path, &name, &tag);
                }
            }
            Ok(Event::End(_)) => {
                let Some(name) = path.pop() else { break };
                match name.as_str() {
                    "entry" => {
                        if let Some(record) = current.take().and_then(ArxivEntry::into_record) {
                            records.push(record);
                        }
                    }
                    "name" if path.last().map(String::as_str) == Some("author") => {
                        if let Some(entry) = current.as_mut() {
                            entry.finish_author();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(entry) = current.as_mut() {
                    let chunk = text
                        .unescape()
                        .map(|cow| cow.into_owned())
                        .unwrap_or_else(|_| String::from_utf8_lossy(&text).into_owned());
                    entry.absorb(&path, &chunk);
                }
            }
            Ok(Event::CData(cdata)) => {
                if let Some(entry) = current.as_mut() {
                    let chunk = String::from_utf8_lossy(&cdata.into_inner()).into_owned();
                    entry.absorb(&path, &chunk);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    records
}

/// Text gathered from one `<entry>` while the reader walks it.
#[derive(Default)]
struct ArxivEntry {
    id: String,
    title: String,
    summary: String,
    published: String,
    journal_ref: String,
    doi: String,
    authors: Vec<Author>,
    /// The `<name>` of the `<author>` currently open.
    name: String,
    /// `<arxiv:primary_category term="..."/>`, and failing that the first
    /// `<category term="..."/>`.
    primary_category: Option<String>,
    category: Option<String>,
}

impl ArxivEntry {
    /// A `primary_category` or `category` tag directly under the entry:
    /// keeps its `term`.
    fn note_category(&mut self, path: &[String], name: &str, tag: &quick_xml::events::BytesStart) {
        if path.last().map(String::as_str) != Some("entry") {
            return;
        }
        let slot = match name {
            "primary_category" => &mut self.primary_category,
            "category" if self.category.is_none() => &mut self.category,
            _ => return,
        };
        let Ok(Some(term)) = tag.try_get_attribute("term") else {
            return;
        };
        let term = term
            .unescape_value()
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if !term.is_empty() {
            *slot = Some(term);
        }
    }

    /// `path` is the stack of open elements; only what sits directly under
    /// `entry` (or under `entry/author`) is of interest.
    fn absorb(&mut self, path: &[String], text: &str) {
        let Some(entry_at) = path.iter().rposition(|name| name == "entry") else {
            return;
        };
        let tail: Vec<&str> = path[entry_at + 1..].iter().map(String::as_str).collect();
        let target = match tail.as_slice() {
            ["id"] => &mut self.id,
            ["title"] => &mut self.title,
            ["summary"] => &mut self.summary,
            ["published"] => &mut self.published,
            ["journal_ref"] => &mut self.journal_ref,
            ["doi"] => &mut self.doi,
            ["author", "name"] => &mut self.name,
            _ => return,
        };
        target.push_str(text);
    }

    fn finish_author(&mut self) {
        let name = std::mem::take(&mut self.name);
        if let Some(author) = split_name(&name) {
            self.authors.push(author);
        }
    }

    fn into_record(self) -> Option<ArxivRecord> {
        let id = self.id.trim();
        if id.contains("/api/errors") {
            return None;
        }
        let title = non_empty(collapse_whitespace(&self.title))?;
        let year = self
            .published
            .trim()
            .get(..4)
            .and_then(|year| year.parse().ok());
        let candidate = LookupCandidateDto {
            source: MetaSource::Arxiv,
            title,
            authors: self.authors,
            year,
            venue: non_empty(collapse_whitespace(&self.journal_ref)),
            volume: None,
            issue: None,
            pages: None,
            publisher: None,
            abstract_: non_empty(collapse_whitespace(&self.summary)),
            // A preprint may have a journal_ref and a DOI, in which case it
            // is an article; leave the type to the venue rule.
            kind: None,
            doi: non_empty(normalize_doi(&self.doi)),
            arxiv: non_empty(normalize_arxiv_id(id)),
            isbn: None,
        };
        Some(ArxivRecord {
            candidate,
            primary_category: self.primary_category.or(self.category),
        })
    }
}

// ---------------------------------------------------------------------------
// OpenAlex
// ---------------------------------------------------------------------------

/// Parses an OpenAlex `works?search=` response into candidates in API order.
/// Results without a title are skipped. Not JSON, or no `results` -> empty.
pub fn parse_openalex_search(json: &str) -> Vec<LookupCandidateDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.get("results")
        .and_then(Value::as_array)
        .map(|results| results.iter().filter_map(openalex_work).collect())
        .unwrap_or_default()
}

/// Parses an OpenAlex `works/{id}` response - one work, no envelope. `None`
/// when it is not a work or has no title.
pub fn parse_openalex_work(json: &str) -> Option<LookupCandidateDto> {
    let root: Value = serde_json::from_str(json).ok()?;
    openalex_work(&root)
}

/// The `referenced_works` of an OpenAlex work, as bare ids (`W123`), in the
/// work's order. Empty when there are none or the JSON is not a work.
pub fn parse_openalex_referenced_works(json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.get("referenced_works")
        .and_then(Value::as_array)
        .map(|works| {
            works
                .iter()
                .filter_map(Value::as_str)
                .filter_map(openalex_bare_id)
                .collect()
        })
        .unwrap_or_default()
}

/// The identifiers of the works in a `works?filter=openalex_id:...` response:
/// each result's DOI (`doi`, else `ids.doi`), lower-cased, with an arXiv
/// DOI folded to its arXiv id. Results without a DOI are left out.
pub fn parse_openalex_dois(json: &str) -> Vec<CitationIdDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter_map(|work| {
                    let doi = work
                        .get("doi")
                        .and_then(Value::as_str)
                        .or_else(|| work.pointer("/ids/doi").and_then(Value::as_str))?;
                    non_empty(normalize_doi(doi))
                })
                .map(|doi| fold_arxiv_doi(CitationIdDto::Doi(doi)))
                .collect()
        })
        .unwrap_or_default()
}

/// `https://openalex.org/W123` or `W123` -> `W123`.
fn openalex_bare_id(id: &str) -> Option<String> {
    let bare = id.trim().rsplit('/').next().unwrap_or("").trim();
    let valid =
        bare.len() > 1 && bare.starts_with('W') && bare[1..].bytes().all(|b| b.is_ascii_digit());
    valid.then(|| bare.to_string())
}

fn openalex_work(work: &Value) -> Option<LookupCandidateDto> {
    let title = ["title", "display_name"]
        .iter()
        .find_map(|key| work.get(*key).and_then(text_field))?;
    let authors = work
        .get("authorships")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|authorship| {
                    let name = authorship.get("author")?.get("display_name")?.as_str()?;
                    split_name(name)
                })
                .collect()
        })
        .unwrap_or_default();
    let year = work
        .get("publication_year")
        .and_then(Value::as_i64)
        .and_then(|year| i32::try_from(year).ok());
    let source = work.pointer("/primary_location/source");
    let biblio = work.get("biblio");
    let biblio_field = |key: &str| biblio.and_then(|b| b.get(key)).and_then(text_field);
    let pages = match (biblio_field("first_page"), biblio_field("last_page")) {
        (Some(first), Some(last)) if first != last => Some(format!("{first}-{last}")),
        (Some(first), _) => Some(first),
        (None, _) => None,
    };
    let kind = work
        .get("type")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "article" => PaperKind::Article,
            "book" => PaperKind::Book,
            "dissertation" => PaperKind::Thesis,
            _ => PaperKind::Misc,
        });
    Some(LookupCandidateDto {
        source: MetaSource::Openalex,
        title,
        authors,
        year,
        venue: source
            .and_then(|s| s.get("display_name"))
            .and_then(text_field),
        volume: biblio_field("volume"),
        issue: biblio_field("issue"),
        pages,
        publisher: source
            .and_then(|s| s.get("host_organization_name"))
            .and_then(text_field),
        abstract_: work
            .get("abstract_inverted_index")
            .and_then(Value::as_object)
            .map(rebuild_inverted_index)
            .and_then(non_empty),
        kind,
        doi: work
            .get("doi")
            .and_then(Value::as_str)
            .map(normalize_doi)
            .and_then(non_empty),
        arxiv: openalex_arxiv(work),
        isbn: None,
    })
}

/// OpenAlex stores abstracts as `word -> [positions]`; put the words back.
fn rebuild_inverted_index(index: &serde_json::Map<String, Value>) -> String {
    let mut slots: Vec<(usize, &str)> = Vec::new();
    for (word, positions) in index {
        let Some(positions) = positions.as_array() else {
            continue;
        };
        for position in positions {
            if let Some(position) = position.as_u64().and_then(|p| usize::try_from(p).ok()) {
                slots.push((position, word.as_str()));
            }
        }
    }
    slots.sort_unstable();
    let words: Vec<&str> = slots.into_iter().map(|(_, word)| word).collect();
    words.join(" ")
}

/// OpenAlex has no arXiv id field; a work hosted on arXiv carries a location
/// whose `landing_page_url` or `pdf_url` is on `arxiv.org`, and the id is
/// read from that. Otherwise `None`.
fn openalex_arxiv(work: &Value) -> Option<String> {
    let primary = work.get("primary_location").into_iter();
    let others = work
        .get("locations")
        .and_then(Value::as_array)
        .map(|locations| locations.iter())
        .into_iter()
        .flatten();
    primary.chain(others).find_map(|location| {
        ["landing_page_url", "pdf_url"]
            .iter()
            .find_map(|key| arxiv_id_from_url(location.get(*key)?.as_str()?))
    })
}

// ---------------------------------------------------------------------------
// Identifiers and text helpers
// ---------------------------------------------------------------------------

/// Lower-cased, with any `https://doi.org/` or `doi:` prefix removed. DOIs
/// are case-insensitive, so this is also the cache key form.
pub(crate) fn normalize_doi(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let mut doi = lower.as_str();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ] {
        if let Some(rest) = doi.strip_prefix(prefix) {
            doi = rest.trim();
        }
    }
    doi.to_string()
}

/// The bare id without version: `2401.12345v2` -> `2401.12345`,
/// `hep-th/9901001v1` -> `hep-th/9901001`. Accepts `arXiv:` prefixes and
/// `arxiv.org/abs/...` or `/pdf/...` URLs.
pub(crate) fn normalize_arxiv_id(raw: &str) -> String {
    if let Some(id) = arxiv_id_from_url(raw) {
        return id;
    }
    let trimmed = raw.trim();
    let bare = trimmed
        .get(..6)
        .filter(|head| head.eq_ignore_ascii_case("arxiv:"))
        .map_or(trimmed, |_| trimmed[6..].trim());
    strip_arxiv_version(bare).to_string()
}

/// The id in an `arxiv.org/abs/...` or `arxiv.org/pdf/...` URL, without version.
fn arxiv_id_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = ["arxiv.org/abs/", "arxiv.org/pdf/"]
        .iter()
        .find_map(|marker| url.find(marker).map(|at| &url[at + marker.len()..]))?;
    let id = rest
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .trim_end_matches(".pdf");
    non_empty(strip_arxiv_version(id).to_string())
}

/// Drops a trailing `vN`. Only a `v` followed by nothing but digits counts,
/// so `solv-int/9901001` keeps its name.
fn strip_arxiv_version(id: &str) -> &str {
    match id.rfind('v') {
        Some(at) if at > 0 => {
            let suffix = &id[at + 1..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                &id[..at]
            } else {
                id
            }
        }
        _ => id,
    }
}

/// `Given Names Family` -> family is the last token; `Family, Given` is
/// honoured too. A single token is a family name alone.
pub(crate) fn split_name(name: &str) -> Option<Author> {
    if let Some((family, given)) = name.split_once(',') {
        let family = collapse_whitespace(family);
        if !family.is_empty() {
            return Some(Author {
                family,
                given: collapse_whitespace(given),
            });
        }
    }
    let mut tokens: Vec<&str> = name.split_whitespace().collect();
    let family = tokens.pop()?.to_string();
    Some(Author {
        family,
        given: tokens.join(" "),
    })
}

pub(crate) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// A string, a number, or the first element of an array of strings
/// (Crossref wraps titles and container titles that way); collapsed, and
/// `None` when empty or another shape.
pub(crate) fn text_field(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => collapse_whitespace(text),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => return items.first().and_then(text_field),
        _ => return None,
    };
    non_empty(text)
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// What the lookup services answered, persisted as `lookup-cache.json` so a
/// repeated lookup costs nothing and a rescan never re-fetches. Four kinds
/// of entry, told apart by key:
///
/// - one candidate per identifier and service: `doi:<doi>` (Crossref),
///   `arxiv:<id>` (arXiv), `isbn:<isbn>` (Open Library), and for the other
///   services `<service>:doi:<doi>` / `<service>:arxiv:<id>`
///   ([`LookupCache::source_key`]);
/// - the merged candidate list of a title search, `title:<normalized
///   title>`, which stands for [`LIST_TTL`];
/// - the identifiers the network says a paper cites and is cited by,
///   `refs:<id key>` / `cites:<id key>` (projects.md sec. 8);
/// - misses, for any of the first two kinds, stamped with when they were
///   seen and expiring after [`MISS_TTL`]: a DOI nobody knew last week may
///   be registered by now, but not by tonight.
///
/// Disposable: a file that fails to parse is discarded on load.
#[derive(Clone, Debug, Default)]
pub struct LookupCache {
    entries: HashMap<String, LookupCandidateDto>,
    /// Key -> unix second of the lookup that found nothing.
    misses: HashMap<String, u64>,
    lists: HashMap<String, CachedList>,
    links: HashMap<String, Vec<CitationIdDto>>,
    /// `arxiv:<id>` -> the primary category arXiv gave, so a cached answer
    /// decides INSPIRE the way the live one did.
    categories: HashMap<String, String>,
}

/// A title search's merged candidates, stamped so they can expire: the
/// services learn new papers, and a search that found nothing sure enough
/// to apply is worth repeating in a month.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct CachedList {
    at: u64,
    candidates: Vec<LookupCandidateDto>,
}

/// The file: `{"entries": {...}, "misses": {...}, "lists": {...}, "links": {...}}`,
/// every map optional. The file before misses existed was the entries map
/// alone; [`LookupCache::load`] still reads it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheFile {
    #[serde(default)]
    entries: BTreeMap<String, LookupCandidateDto>,
    #[serde(default)]
    misses: BTreeMap<String, u64>,
    #[serde(default)]
    lists: BTreeMap<String, CachedList>,
    #[serde(default)]
    links: BTreeMap<String, Vec<CitationIdDto>>,
    #[serde(default)]
    categories: BTreeMap<String, String>,
}

/// The file as written: the same shape, borrowed, with sorted keys, so a
/// save clones nothing.
#[derive(Serialize)]
struct CacheFileRef<'a> {
    entries: BTreeMap<&'a str, &'a LookupCandidateDto>,
    misses: BTreeMap<&'a str, u64>,
    lists: BTreeMap<&'a str, &'a CachedList>,
    links: BTreeMap<&'a str, &'a Vec<CitationIdDto>>,
    categories: BTreeMap<&'a str, &'a str>,
}

/// The citation edges the network supplied, keyed by the paper's own
/// identifier: what it cites (`refs`) and what cites it (`cites`). Built
/// from the cache by [`LookupCache::network_edges`] for the store's graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkEdges {
    pub refs: HashMap<CitationIdDto, Vec<CitationIdDto>>,
    pub cites: HashMap<CitationIdDto, Vec<CitationIdDto>>,
}

impl LookupCache {
    /// `doi:<lower-cased doi>`
    pub fn doi_key(doi: &str) -> String {
        format!("doi:{}", normalize_doi(doi))
    }

    /// `arxiv:<id without version>`
    pub fn arxiv_key(arxiv_id: &str) -> String {
        format!("arxiv:{}", normalize_arxiv_id(arxiv_id))
    }

    /// `isbn:<digits>`
    pub fn isbn_key(isbn: &str) -> String {
        let digits = crate::library::normalize_isbn(isbn)
            .unwrap_or_else(|| isbn.trim().to_ascii_lowercase());
        format!("isbn:{digits}")
    }

    /// `doi:...` or `arxiv:...` for an identifier.
    pub fn id_key(id: &CitationIdDto) -> String {
        match id {
            CitationIdDto::Doi(doi) => Self::doi_key(doi),
            CitationIdDto::ArXiv(arxiv) => Self::arxiv_key(arxiv),
        }
    }

    /// `<service>:doi:...` / `<service>:arxiv:...`: one service's answer for an
    /// identifier, for the services that are not the identifier's own
    /// registry (`doi:` is Crossref's answer, `arxiv:` arXiv's).
    pub fn source_key(service: &str, id: &CitationIdDto) -> String {
        format!("{service}:{}", Self::id_key(id))
    }

    /// `title:<normalized title>` (see `titlematch::normalize`).
    pub fn title_key(title: &str) -> String {
        format!("title:{}", crate::library::titlematch::normalize(title))
    }

    /// `refs:<id key>`: what the network says this work cites.
    pub fn refs_key(id: &CitationIdDto) -> String {
        format!("refs:{}", Self::id_key(id))
    }

    /// `cites:<id key>`: what the network says cites this work.
    pub fn cites_key(id: &CitationIdDto) -> String {
        format!("cites:{}", Self::id_key(id))
    }

    /// Reads `dir/lookup-cache.json`, in either shape. Missing or corrupt ->
    /// empty. Misses and lists past their time are dropped on the way in.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(CACHE_FILE);
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        let mut cache = match Self::parse(&bytes) {
            Some(cache) => cache,
            None => {
                log::warn!("discarding unreadable lookup cache {}", path.display());
                Self::default()
            }
        };
        cache.prune();
        cache
    }

    /// The envelope first; failing that, the plain map the file used to be.
    fn parse(bytes: &[u8]) -> Option<Self> {
        if let Ok(file) = serde_json::from_slice::<CacheFile>(bytes) {
            return Some(Self {
                entries: file.entries.into_iter().collect(),
                misses: file.misses.into_iter().collect(),
                lists: file.lists.into_iter().collect(),
                links: file.links.into_iter().collect(),
                categories: file.categories.into_iter().collect(),
            });
        }
        let entries: HashMap<String, LookupCandidateDto> = serde_json::from_slice(bytes).ok()?;
        Some(Self {
            entries,
            ..Self::default()
        })
    }

    pub fn get(&self, key: &str) -> Option<&LookupCandidateDto> {
        self.entries.get(key)
    }

    /// A found candidate also clears a miss recorded for the key.
    pub fn insert(&mut self, key: impl Into<String>, candidate: LookupCandidateDto) {
        let key = key.into();
        self.misses.remove(&key);
        self.entries.insert(key, candidate);
    }

    /// A title search's merged candidates, while they stand.
    pub fn get_list(&self, key: &str) -> Option<&[LookupCandidateDto]> {
        self.get_list_at(key, unix_now())
    }

    pub fn get_list_at(&self, key: &str, now: u64) -> Option<&[LookupCandidateDto]> {
        self.lists
            .get(key)
            .filter(|list| !list_expired(list.at, now))
            .map(|list| list.candidates.as_slice())
    }

    /// Keeps what a title search found, as of now. Clears a miss for the
    /// key; an empty list is recorded as a miss instead.
    pub fn insert_list(&mut self, key: impl Into<String>, candidates: Vec<LookupCandidateDto>) {
        self.insert_list_at(key, candidates, unix_now());
    }

    pub fn insert_list_at(
        &mut self,
        key: impl Into<String>,
        candidates: Vec<LookupCandidateDto>,
        at: u64,
    ) {
        let key = key.into();
        if candidates.is_empty() {
            self.lists.remove(&key);
            self.misses.insert(key, at);
            return;
        }
        self.misses.remove(&key);
        self.lists.insert(key, CachedList { at, candidates });
    }

    /// The identifiers under a `refs:` or `cites:` key, if the network has
    /// been asked. An empty list is an answer too: nothing known.
    pub fn get_links(&self, key: &str) -> Option<&[CitationIdDto]> {
        self.links.get(key).map(Vec::as_slice)
    }

    pub fn insert_links(&mut self, key: impl Into<String>, ids: Vec<CitationIdDto>) {
        self.links.insert(key.into(), ids);
    }

    /// The arXiv primary category kept beside an `arxiv:` entry.
    pub fn get_category(&self, key: &str) -> Option<String> {
        self.categories.get(key).cloned()
    }

    pub fn insert_category(&mut self, key: impl Into<String>, category: String) {
        self.categories.insert(key.into(), category);
    }

    /// Every `refs:` and `cites:` list, keyed by the paper's identifier.
    pub fn network_edges(&self) -> NetworkEdges {
        let mut edges = NetworkEdges::default();
        for (key, ids) in &self.links {
            let (map, rest) = if let Some(rest) = key.strip_prefix("refs:") {
                (&mut edges.refs, rest)
            } else if let Some(rest) = key.strip_prefix("cites:") {
                (&mut edges.cites, rest)
            } else {
                continue;
            };
            let id = if let Some(doi) = rest.strip_prefix("doi:") {
                CitationIdDto::Doi(doi.to_string())
            } else if let Some(arxiv) = rest.strip_prefix("arxiv:") {
                CitationIdDto::ArXiv(arxiv.to_string())
            } else {
                continue;
            };
            map.insert(id, ids.clone());
        }
        edges
    }

    /// Records that a lookup of `key` found nothing, as of now.
    pub fn insert_miss(&mut self, key: impl Into<String>) {
        self.insert_miss_at(key, unix_now());
    }

    /// The same with the moment given, for a test or a replay.
    pub fn insert_miss_at(&mut self, key: impl Into<String>, at: u64) {
        self.misses.insert(key.into(), at);
    }

    /// Whether `key` was looked up within the last [`MISS_TTL`] and found
    /// nothing. A cached candidate is never a miss.
    pub fn is_miss(&self, key: &str) -> bool {
        self.is_miss_at(key, unix_now())
    }

    pub fn is_miss_at(&self, key: &str, now: u64) -> bool {
        !self.entries.contains_key(key)
            && self
                .misses
                .get(key)
                .is_some_and(|at| !miss_expired(*at, now))
    }

    /// Drops the misses and title lists past their time.
    pub fn prune(&mut self) {
        self.prune_at(unix_now());
    }

    pub fn prune_at(&mut self, now: u64) {
        self.misses.retain(|_, at| !miss_expired(*at, now));
        self.lists.retain(|_, list| !list_expired(list.at, now));
    }

    /// Candidates held; misses are not counted.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn miss_count(&self) -> usize {
        self.misses.len()
    }

    /// Writes `dir/lookup-cache.json` atomically: a temp file in `dir`, then
    /// a rename over the old one. Keys are written sorted so the file is
    /// stable across saves.
    pub fn save(&self, dir: &Path) -> AppResult<()> {
        fs::create_dir_all(dir)?;
        let file = CacheFileRef {
            entries: self
                .entries
                .iter()
                .map(|(key, candidate)| (key.as_str(), candidate))
                .collect(),
            misses: self
                .misses
                .iter()
                .map(|(key, at)| (key.as_str(), *at))
                .collect(),
            lists: self
                .lists
                .iter()
                .map(|(key, list)| (key.as_str(), list))
                .collect(),
            links: self
                .links
                .iter()
                .map(|(key, ids)| (key.as_str(), ids))
                .collect(),
            categories: self
                .categories
                .iter()
                .map(|(key, category)| (key.as_str(), category.as_str()))
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(|error| {
            AppError::Internal(format!("cannot serialize lookup cache: {error}"))
        })?;
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(dir.join(CACHE_FILE))
            .map_err(|error| AppError::Io(error.to_string()))?;
        Ok(())
    }
}

/// A miss older than [`MISS_TTL`] (or stamped in the future, by a clock
/// that was wrong) no longer stands.
fn miss_expired(at: u64, now: u64) -> bool {
    at > now || now - at >= MISS_TTL.as_secs()
}

/// A title list older than [`LIST_TTL`] (or from the future) no longer
/// stands.
fn list_expired(at: u64, now: u64) -> bool {
    at > now || now - at >= LIST_TTL.as_secs()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests - no network. Fixtures are trimmed from the three APIs' documented
// response shapes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn author(family: &str, given: &str) -> Author {
        Author {
            family: family.to_string(),
            given: given.to_string(),
        }
    }

    // -- settings ---------------------------------------------------------

    #[test]
    fn disabled_settings_refuse_with_the_settings_hint() {
        let off = LookupSettings::default();
        let error = ensure_enabled(&off).expect_err("off by default");
        assert_eq!(
            error.to_string(),
            "Online lookup is off (turn on Look up paper metadata online in Settings)"
        );
        let on = LookupSettings {
            enabled: true,
            ..Default::default()
        };
        assert!(ensure_enabled(&on).is_ok());
    }

    #[test]
    fn every_fetch_refuses_before_building_a_request_when_disabled() {
        // The client never connects on construction, and each fetch checks
        // the setting first, so this runs without a network.
        let off = LookupSettings {
            enabled: false,
            contact: "me@example.org".into(),
            s2_key: String::new(),
        };
        let client = client();
        let limiter = HostLimiter::new();
        fn run<F: std::future::Future>(future: F) -> F::Output {
            tauri::async_runtime::block_on(future)
        }
        assert!(run(fetch_by_doi(&client, &limiter, &off, "10.1000/x")).is_err());
        assert!(run(fetch_by_arxiv(&client, &limiter, &off, "2401.12345")).is_err());
        assert!(run(search_by_title(&client, &limiter, &off, "A title")).is_err());
        assert!(run(search_crossref_by_title(&client, &limiter, &off, "A title")).is_err());
        assert!(run(search_arxiv_by_title(&client, &limiter, &off, "A title")).is_err());
        let id = CitationIdDto::Doi("10.1000/x".into());
        assert!(run(fetch_openalex_record(&client, &limiter, &off, &id)).is_err());
        let arxiv = CitationIdDto::ArXiv("2401.12345".into());
        assert!(run(fetch_openalex_record(&client, &limiter, &off, &arxiv)).is_err());
        assert!(run(resolve_openalex_works(
            &client,
            &limiter,
            &off,
            &["W1".to_string()]
        ))
        .is_err());
        assert_eq!(
            limiter.wait_for("api.crossref.org"),
            Duration::ZERO,
            "nothing was sent"
        );
    }

    #[test]
    fn contact_goes_into_crossref_user_agent_only_when_set() {
        let anonymous = LookupSettings {
            enabled: true,
            contact: "   ".into(),
            s2_key: String::new(),
        };
        assert_eq!(crossref_user_agent(&anonymous), None);
        assert_eq!(anonymous.s2_key(), None);
        let polite = LookupSettings {
            enabled: true,
            contact: " me@example.org ".into(),
            s2_key: " key ".into(),
        };
        assert_eq!(
            crossref_user_agent(&polite).as_deref(),
            Some("SpeedyF/0.1.0 (mailto:me@example.org)")
        );
        assert_eq!(polite.s2_key(), Some("key"));
    }

    #[test]
    fn title_queries_keep_words_only() {
        assert_eq!(
            query_words("Non-equilibrium \"Equality\": Free Energy (Differences)?"),
            "Non equilibrium Equality Free Energy Differences"
        );
        assert_eq!(query_words("  ...  "), "");
        assert_eq!(query_words("Übergänge in C++"), "Übergänge in C");
    }

    #[test]
    fn arxiv_ids_become_datacite_dois_for_openalex() {
        assert_eq!(arxiv_doi("2401.12345"), "10.48550/arxiv.2401.12345");
        assert_eq!(arxiv_doi("hep-th/9901001"), "10.48550/arxiv.hep-th/9901001");
        assert_eq!(
            arxiv_doi("math.GT/0309136"),
            "10.48550/arxiv.math.gt/0309136"
        );
    }

    #[test]
    fn crossref_url_escapes_the_doi_but_keeps_its_slash() {
        assert_eq!(
            crossref_url("10.1103/physrevlett.78.2690"),
            "https://api.crossref.org/works/10.1103/physrevlett.78.2690"
        );
        assert_eq!(
            crossref_url("10.1002/(sici)1097-0258(19980430)17:8<857::aid-sim777>3.0.co;2-e"),
            "https://api.crossref.org/works/10.1002/%28sici%291097-0258%2819980430%2917%3A8%3C857%3A%3Aaid-sim777%3E3.0.co%3B2-e"
        );
        assert_eq!(
            crossref_url("10.1000/a#b?c"),
            "https://api.crossref.org/works/10.1000/a%23b%3Fc"
        );
    }

    // -- identifiers ------------------------------------------------------

    #[test]
    fn doi_normalization() {
        assert_eq!(
            normalize_doi(" https://doi.org/10.1000/ABC "),
            "10.1000/abc"
        );
        assert_eq!(normalize_doi("doi:10.1000/X"), "10.1000/x");
        assert_eq!(normalize_doi("http://dx.doi.org/10.1000/y"), "10.1000/y");
        assert_eq!(normalize_doi("10.1000/z"), "10.1000/z");
        assert_eq!(normalize_doi(""), "");
    }

    #[test]
    fn arxiv_id_normalization_strips_versions_and_prefixes() {
        assert_eq!(normalize_arxiv_id("2401.12345v2"), "2401.12345");
        assert_eq!(normalize_arxiv_id("2401.12345"), "2401.12345");
        assert_eq!(normalize_arxiv_id("hep-th/9901001v1"), "hep-th/9901001");
        assert_eq!(normalize_arxiv_id("solv-int/9901001"), "solv-int/9901001");
        assert_eq!(normalize_arxiv_id("solv-int/9901001v3"), "solv-int/9901001");
        assert_eq!(normalize_arxiv_id("arXiv:2401.12345v1"), "2401.12345");
        assert_eq!(
            normalize_arxiv_id("http://arxiv.org/abs/2401.12345v2"),
            "2401.12345"
        );
        assert_eq!(
            normalize_arxiv_id("https://arxiv.org/pdf/cond-mat/9610209v1.pdf"),
            "cond-mat/9610209"
        );
        assert_eq!(normalize_arxiv_id("math.GT/0309136v2"), "math.GT/0309136");
        assert_eq!(arxiv_id_from_url("https://doi.org/10.1/x"), None);
    }

    #[test]
    fn name_splitting() {
        assert_eq!(
            split_name("M. Cristina Marchetti"),
            Some(author("Marchetti", "M. Cristina"))
        );
        assert_eq!(split_name("Ramaswamy"), Some(author("Ramaswamy", "")));
        assert_eq!(
            split_name("Smith, John A."),
            Some(author("Smith", "John A."))
        );
        assert_eq!(split_name("   "), None);
    }

    // -- Crossref ---------------------------------------------------------

    const CROSSREF_WORK: &str = r#"{
      "status": "ok",
      "message-type": "work",
      "message-version": "1.0.0",
      "message": {
        "DOI": "10.1103/PhysRevLett.78.2690",
        "type": "journal-article",
        "title": ["Nonequilibrium Equality for Free Energy\n   Differences"],
        "container-title": ["Physical Review Letters"],
        "short-container-title": ["Phys. Rev. Lett."],
        "author": [
          {"given": "C.", "family": "Jarzynski", "sequence": "first", "affiliation": []},
          {"name": "ATLAS Collaboration", "sequence": "additional", "affiliation": []},
          {"given": "Orphan", "sequence": "additional", "affiliation": []}
        ],
        "issued": {"date-parts": [[1997, 4, 7]]},
        "published-print": {"date-parts": [[1997, 4, 7]]},
        "created": {"date-parts": [[2002, 7, 27]], "date-time": "2002-07-27T07:32:15Z"},
        "volume": "78",
        "issue": "14",
        "page": "2690-2693",
        "publisher": "American Physical Society (APS)",
        "abstract": "<jats:title>Abstract</jats:title><jats:p>An expression is derived for the <jats:italic>free energy</jats:italic> difference &amp; its bound (&#x03B2; &lt; 1).</jats:p><jats:p>Second paragraph.</jats:p>",
        "URL": "http://dx.doi.org/10.1103/physrevlett.78.2690",
        "reference-count": 11,
        "is-referenced-by-count": 4000
      }
    }"#;

    #[test]
    fn crossref_work_parses_every_field() {
        let work = parse_crossref_work(CROSSREF_WORK).expect("a work");
        assert_eq!(work.source, MetaSource::Crossref);
        assert_eq!(
            work.title,
            "Nonequilibrium Equality for Free Energy Differences"
        );
        assert_eq!(
            work.authors,
            vec![author("Jarzynski", "C."), author("ATLAS Collaboration", "")]
        );
        assert_eq!(work.year, Some(1997));
        assert_eq!(work.venue.as_deref(), Some("Physical Review Letters"));
        assert_eq!(work.volume.as_deref(), Some("78"));
        assert_eq!(work.issue.as_deref(), Some("14"));
        assert_eq!(work.pages.as_deref(), Some("2690-2693"));
        assert_eq!(
            work.publisher.as_deref(),
            Some("American Physical Society (APS)")
        );
        assert_eq!(
            work.abstract_.as_deref(),
            Some("An expression is derived for the free energy difference & its bound (β < 1). Second paragraph.")
        );
        assert_eq!(work.kind, Some(PaperKind::Article));
        assert_eq!(work.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(work.arxiv, None);
    }

    #[test]
    fn crossref_tolerates_missing_fields_and_falls_back_for_the_year() {
        let json = r#"{"message": {
            "DOI": "10.5555/MISSING",
            "type": "proceedings-article",
            "title": ["Something"],
            "issued": {"date-parts": [[null]]},
            "published-online": {"date-parts": [[2020, 1]]}
        }}"#;
        let work = parse_crossref_work(json).expect("a work");
        assert!(work.authors.is_empty());
        assert_eq!(work.year, Some(2020));
        assert_eq!(work.kind, Some(PaperKind::Inproceedings));
        assert_eq!(work.venue, None);
        assert_eq!(work.abstract_, None);
        assert_eq!(work.pages, None);

        let bare = r#"{"title": ["Bare"], "type": "monograph"}"#;
        let work = parse_crossref_work(bare).expect("a bare work");
        assert_eq!(work.year, None);
        assert_eq!(work.doi, None);
        assert_eq!(work.kind, Some(PaperKind::Book));
    }

    #[test]
    fn crossref_type_mapping() {
        let kind = |t: &str| {
            parse_crossref_work(&format!(r#"{{"title": ["T"], "type": "{t}"}}"#))
                .and_then(|w| w.kind)
        };
        assert_eq!(kind("journal-article"), Some(PaperKind::Article));
        assert_eq!(kind("proceedings-article"), Some(PaperKind::Inproceedings));
        assert_eq!(kind("book"), Some(PaperKind::Book));
        assert_eq!(kind("edited-book"), Some(PaperKind::Book));
        assert_eq!(kind("dissertation"), Some(PaperKind::Thesis));
        assert_eq!(kind("posted-content"), Some(PaperKind::Misc));
        assert_eq!(
            parse_crossref_work(r#"{"title": ["T"]}"#).and_then(|w| w.kind),
            None
        );
    }

    #[test]
    fn crossref_search_parses_the_items_in_order() {
        let json = r#"{
          "status": "ok",
          "message-type": "work-list",
          "message": {
            "total-results": 3,
            "items": [
              {"DOI": "10.1103/PhysRevLett.78.2690", "type": "journal-article",
               "title": ["Nonequilibrium Equality for Free Energy Differences"],
               "author": [{"given": "C.", "family": "Jarzynski"}],
               "issued": {"date-parts": [[1997]]}},
              {"DOI": "10.5555/untitled", "type": "journal-article", "title": []},
              {"DOI": "10.5555/second", "type": "proceedings-article", "title": ["Second"]}
            ],
            "items-per-page": 5
          }
        }"#;
        let candidates = parse_crossref_search(json);
        assert_eq!(candidates.len(), 2, "the untitled item is skipped");
        assert_eq!(candidates[0].source, MetaSource::Crossref);
        assert_eq!(
            candidates[0].doi.as_deref(),
            Some("10.1103/physrevlett.78.2690")
        );
        assert_eq!(candidates[0].year, Some(1997));
        assert_eq!(candidates[1].title, "Second");
        assert_eq!(candidates[1].kind, Some(PaperKind::Inproceedings));

        assert!(parse_crossref_search("").is_empty());
        assert!(parse_crossref_search(r#"{"message": {"items": []}}"#).is_empty());
        assert!(
            parse_crossref_search(CROSSREF_WORK).is_empty(),
            "a single work has no items"
        );
    }

    #[test]
    fn crossref_rejects_what_is_not_a_titled_work() {
        assert_eq!(parse_crossref_work("not json"), None);
        assert_eq!(
            parse_crossref_work(r#"{"message": "Resource not found."}"#),
            None
        );
        assert_eq!(
            parse_crossref_work(r#"{"message": {"DOI": "10.1/x", "title": []}}"#),
            None
        );
        assert_eq!(
            parse_crossref_work(r#"{"message": {"DOI": "10.1/x", "title": ["  "]}}"#),
            None
        );
        assert_eq!(parse_crossref_work("[1, 2]"), None);
    }

    #[test]
    fn jats_stripping() {
        assert_eq!(strip_jats("<jats:p>Plain.</jats:p>"), "Plain.");
        assert_eq!(
            strip_jats("<jats:sec><jats:title>Background</jats:title><jats:p>One.</jats:p></jats:sec><jats:sec><jats:title>Methods</jats:title><jats:p>Two &amp; <jats:sup>2</jats:sup>.</jats:p></jats:sec>"),
            "One. Two & 2."
        );
        assert_eq!(
            strip_jats("No tags &lt;here&gt; &#39;quoted&#39;"),
            "No tags <here> 'quoted'"
        );
        assert_eq!(strip_jats("Unterminated <jats:p"), "Unterminated");
        assert_eq!(
            strip_jats("Stray & ampersand &unknown; stays"),
            "Stray & ampersand &unknown; stays"
        );
        assert_eq!(strip_jats("<title>Abstract</title> Text"), "Text");
    }

    // -- arXiv ------------------------------------------------------------

    const ARXIV_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <link href="http://arxiv.org/api/query?search_query%3D%26id_list%3D2401.12345%26start%3D0%26max_results%3D1" rel="self" type="application/atom+xml"/>
  <title type="html">ArXiv Query: search_query=&amp;id_list=2401.12345&amp;start=0&amp;max_results=1</title>
  <id>http://arxiv.org/api/abc123def456</id>
  <updated>2024-03-02T00:00:00-05:00</updated>
  <opensearch:totalResults xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">1</opensearch:totalResults>
  <opensearch:startIndex xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">0</opensearch:startIndex>
  <opensearch:itemsPerPage xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">1</opensearch:itemsPerPage>
  <entry>
    <id>http://arxiv.org/abs/2401.12345v2</id>
    <updated>2024-03-01T10:00:00Z</updated>
    <published>2024-01-22T18:59:59Z</published>
    <title>Active Matter at the
  Mesoscale: A Review</title>
    <summary>  We review the hydrodynamics of active matter,
with emphasis on mesoscale phenomena &amp; defects.
</summary>
    <author>
      <name>M. Cristina Marchetti</name>
      <arxiv:affiliation xmlns:arxiv="http://arxiv.org/schemas/atom">UCSB</arxiv:affiliation>
    </author>
    <author>
      <name>Ramaswamy</name>
    </author>
    <arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.1103/RevModPhys.85.1143</arxiv:doi>
    <link title="doi" href="http://dx.doi.org/10.1103/RevModPhys.85.1143" rel="related"/>
    <arxiv:comment xmlns:arxiv="http://arxiv.org/schemas/atom">42 pages, 12 figures</arxiv:comment>
    <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cond-mat.soft" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cond-mat.soft" scheme="http://arxiv.org/schemas/atom"/>
    <category term="physics.bio-ph" scheme="http://arxiv.org/schemas/atom"/>
    <arxiv:journal_ref xmlns:arxiv="http://arxiv.org/schemas/atom">Rev. Mod. Phys. 85,
      1143 (2013)</arxiv:journal_ref>
    <link href="http://arxiv.org/abs/2401.12345v2" rel="alternate" type="text/html"/>
    <link title="pdf" href="http://arxiv.org/pdf/2401.12345v2" rel="related" type="application/pdf"/>
    <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cond-mat.soft" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cond-mat.soft" scheme="http://arxiv.org/schemas/atom"/>
  </entry>
</feed>
"#;

    #[test]
    fn arxiv_titles_keep_their_tex() {
        let feed = ARXIV_FEED.replace(
            "<title>Active Matter at the\n  Mesoscale: A Review</title>",
            "<title>Scaling of $\\mathcal{O}(N^2)$\n  attention &amp; $x_1$</title>",
        );
        let entries = parse_arxiv_atom(&feed);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].title,
            "Scaling of $\\mathcal{O}(N^2)$ attention & $x_1$"
        );
    }

    #[test]
    fn crossref_titles_turn_markup_into_tex() {
        assert_eq!(crossref_title("Plain title"), "Plain title");
        assert_eq!(
            crossref_title("Splitting H<sub>2</sub>O with <i>in vivo</i> TiO<sub>2</sub>"),
            "Splitting H$_{2}$O with in vivo TiO$_{2}$"
        );
        assert_eq!(
            crossref_title("Ca<sup>2+</sup> waves in <b>bold</b> &amp; <jats:sub>x</jats:sub>"),
            "Ca$^{2+}$ waves in bold & $_{x}$"
        );
        assert_eq!(
            crossref_title(
                r#"Bounds on <mml:math xmlns:mml="http://www.w3.org/1998/Math/MathML" alttext="\epsilon &lt; 1"><mml:mi>&#x3B5;</mml:mi><mml:mo>&lt;</mml:mo><mml:mn>1</mml:mn></mml:math> codes"#
            ),
            "Bounds on $\\epsilon < 1$ codes"
        );
        assert_eq!(
            crossref_title(
                "The <mml:math><mml:mi>k</mml:mi><mml:mo>-</mml:mo><mml:mi>SAT</mml:mi></mml:math> problem"
            ),
            "The k-SAT problem"
        );
        assert_eq!(crossref_title("a<sup>b<sup>c</sup></sup> d"), "a$^{bc}$ d");
        assert_eq!(crossref_title("broken <i"), "broken");
    }

    #[test]
    fn crossref_work_title_goes_through_the_markup_conversion() {
        let json = r#"{"title": ["Growth of GaN<sub>x</sub> on <i>c</i>-plane sapphire"]}"#;
        let work = parse_crossref_work(json).expect("a work");
        assert_eq!(work.title, "Growth of GaN$_{x}$ on c-plane sapphire");
        let empty = r#"{"title": ["<i> </i>"]}"#;
        assert_eq!(parse_crossref_work(empty), None);
    }

    #[test]
    fn arxiv_atom_parses_the_entry_and_not_the_feed_header() {
        let candidates = parse_arxiv_atom(ARXIV_FEED);
        assert_eq!(candidates.len(), 1);
        let entry = &candidates[0];
        assert_eq!(entry.source, MetaSource::Arxiv);
        assert_eq!(entry.title, "Active Matter at the Mesoscale: A Review");
        assert_eq!(
            entry.authors,
            vec![author("Marchetti", "M. Cristina"), author("Ramaswamy", "")]
        );
        assert_eq!(entry.year, Some(2024));
        assert_eq!(entry.arxiv.as_deref(), Some("2401.12345"));
        assert_eq!(entry.doi.as_deref(), Some("10.1103/revmodphys.85.1143"));
        assert_eq!(
            entry.venue.as_deref(),
            Some("Rev. Mod. Phys. 85, 1143 (2013)")
        );
        assert_eq!(
            entry.abstract_.as_deref(),
            Some("We review the hydrodynamics of active matter, with emphasis on mesoscale phenomena & defects.")
        );
        assert_eq!(entry.kind, None);
        assert_eq!(entry.volume, None);
        assert_eq!(entry.pages, None);
        assert_eq!(entry.publisher, None);
    }

    #[test]
    fn arxiv_entries_carry_their_primary_category() {
        let records = parse_arxiv_entries(ARXIV_FEED);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].primary_category.as_deref(),
            Some("cond-mat.soft")
        );
        assert_eq!(records[0].candidate.arxiv.as_deref(), Some("2401.12345"));

        // Without a primary category the first category stands in; a
        // category under another element (an author's) does not count.
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry>
            <id>http://arxiv.org/abs/2401.00001v1</id>
            <title>Categories</title>
            <author><name>A</name><category term="not-this"/></author>
            <category term="cs.LG" scheme="x"></category>
            <category term="stat.ML"/>
        </entry><entry>
            <id>http://arxiv.org/abs/2401.00002v1</id>
            <title>None</title>
        </entry></feed>"#;
        let records = parse_arxiv_entries(xml);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].primary_category.as_deref(), Some("cs.LG"));
        assert_eq!(records[1].primary_category, None);
    }

    #[test]
    fn arxiv_legacy_ids_keep_their_archive_and_lose_their_version() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry>
            <id>http://arxiv.org/abs/hep-th/9901001v3</id>
            <title>Legacy</title>
            <summary><![CDATA[Summary in <CDATA>.]]></summary>
        </entry></feed>"#;
        let candidates = parse_arxiv_atom(xml);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].arxiv.as_deref(), Some("hep-th/9901001"));
        assert_eq!(
            candidates[0].abstract_.as_deref(),
            Some("Summary in <CDATA>.")
        );
        assert!(candidates[0].authors.is_empty());
        assert_eq!(candidates[0].year, None);
        assert_eq!(candidates[0].doi, None);
        assert_eq!(candidates[0].venue, None);
    }

    #[test]
    fn arxiv_empty_feed_error_entry_and_junk_yield_nothing() {
        let empty = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="html">ArXiv Query: search_query=&amp;id_list=9999.99999&amp;start=0&amp;max_results=1</title>
  <id>http://arxiv.org/api/none</id>
  <opensearch:totalResults xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">0</opensearch:totalResults>
</feed>"#;
        assert!(parse_arxiv_atom(empty).is_empty());

        let error = r#"<feed xmlns="http://www.w3.org/2005/Atom"><entry>
            <id>http://arxiv.org/api/errors#incorrect_id_format_for_foo</id>
            <title>Error</title>
            <summary>incorrect id format for foo</summary>
        </entry></feed>"#;
        assert!(parse_arxiv_atom(error).is_empty());

        assert!(parse_arxiv_atom("").is_empty());
        assert!(parse_arxiv_atom("{\"not\": \"xml\"}").is_empty());
        let untitled = r#"<feed><entry><id>http://arxiv.org/abs/2401.00001v1</id></entry></feed>"#;
        assert!(parse_arxiv_atom(untitled).is_empty());
    }

    // -- OpenAlex ---------------------------------------------------------

    const OPENALEX_SEARCH: &str = r#"{
      "meta": {"count": 2, "db_response_time_ms": 12, "page": 1, "per_page": 5},
      "results": [
        {
          "id": "https://openalex.org/W2100837269",
          "doi": "https://doi.org/10.1103/PhysRevLett.78.2690",
          "title": "Nonequilibrium Equality for Free Energy Differences",
          "display_name": "Nonequilibrium Equality for Free Energy Differences",
          "publication_year": 1997,
          "publication_date": "1997-04-07",
          "ids": {
            "openalex": "https://openalex.org/W2100837269",
            "doi": "https://doi.org/10.1103/physrevlett.78.2690",
            "mag": "2100837269"
          },
          "type": "article",
          "primary_location": {
            "is_oa": false,
            "landing_page_url": "https://doi.org/10.1103/physrevlett.78.2690",
            "pdf_url": null,
            "source": {
              "id": "https://openalex.org/S24807848",
              "display_name": "Physical Review Letters",
              "issn_l": "0031-9007",
              "host_organization": "https://openalex.org/P4310320261",
              "host_organization_name": "American Physical Society",
              "type": "journal"
            }
          },
          "locations": [
            {
              "landing_page_url": "https://doi.org/10.1103/physrevlett.78.2690",
              "pdf_url": null,
              "source": {"id": "https://openalex.org/S24807848", "display_name": "Physical Review Letters", "type": "journal"}
            },
            {
              "landing_page_url": "https://arxiv.org/abs/cond-mat/9610209v1",
              "pdf_url": "https://arxiv.org/pdf/cond-mat/9610209v1",
              "source": {"id": "https://openalex.org/S4306400194", "display_name": "arXiv (Cornell University)", "type": "repository"}
            }
          ],
          "authorships": [
            {
              "author_position": "first",
              "author": {"id": "https://openalex.org/A5023", "display_name": "C. Jarzynski", "orcid": null},
              "institutions": [],
              "raw_author_name": "C. Jarzynski"
            }
          ],
          "biblio": {"volume": "78", "issue": "14", "first_page": "2690", "last_page": "2693"},
          "abstract_inverted_index": {"An": [0], "equality": [1], "is": [2, 5], "derived": [3], "that": [4], "exact": [6]}
        },
        {
          "id": "https://openalex.org/W999",
          "doi": null,
          "title": null,
          "display_name": "A Thesis Without Much",
          "publication_year": null,
          "type": "dissertation",
          "primary_location": null,
          "locations": [],
          "authorships": [],
          "biblio": {"volume": null, "issue": null, "first_page": "7", "last_page": null},
          "abstract_inverted_index": null
        },
        {
          "id": "https://openalex.org/W0",
          "title": null,
          "display_name": null
        }
      ]
    }"#;

    #[test]
    fn openalex_search_parses_results_in_order() {
        let candidates = parse_openalex_search(OPENALEX_SEARCH);
        assert_eq!(candidates.len(), 2, "the untitled result is skipped");

        let first = &candidates[0];
        assert_eq!(first.source, MetaSource::Openalex);
        assert_eq!(
            first.title,
            "Nonequilibrium Equality for Free Energy Differences"
        );
        assert_eq!(first.authors, vec![author("Jarzynski", "C.")]);
        assert_eq!(first.year, Some(1997));
        assert_eq!(first.venue.as_deref(), Some("Physical Review Letters"));
        assert_eq!(first.volume.as_deref(), Some("78"));
        assert_eq!(first.issue.as_deref(), Some("14"));
        assert_eq!(first.pages.as_deref(), Some("2690-2693"));
        assert_eq!(
            first.publisher.as_deref(),
            Some("American Physical Society")
        );
        assert_eq!(
            first.abstract_.as_deref(),
            Some("An equality is derived that is exact")
        );
        assert_eq!(first.kind, Some(PaperKind::Article));
        assert_eq!(first.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(first.arxiv.as_deref(), Some("cond-mat/9610209"));

        let second = &candidates[1];
        assert_eq!(second.title, "A Thesis Without Much");
        assert!(second.authors.is_empty());
        assert_eq!(second.year, None);
        assert_eq!(second.venue, None);
        assert_eq!(second.volume, None);
        assert_eq!(second.pages.as_deref(), Some("7"));
        assert_eq!(second.publisher, None);
        assert_eq!(second.abstract_, None);
        assert_eq!(second.kind, Some(PaperKind::Thesis));
        assert_eq!(second.doi, None);
        assert_eq!(second.arxiv, None);
    }

    #[test]
    fn openalex_junk_and_empty_results() {
        assert!(parse_openalex_search("").is_empty());
        assert!(parse_openalex_search(r#"{"meta": {"count": 0}, "results": []}"#).is_empty());
        assert!(parse_openalex_search(r#"{"error": "Invalid query"}"#).is_empty());
    }

    const OPENALEX_WORK: &str = r#"{
      "id": "https://openalex.org/W2100837269",
      "doi": "https://doi.org/10.1103/PhysRevLett.78.2690",
      "title": "Nonequilibrium Equality for Free Energy Differences",
      "publication_year": 1997,
      "type": "article",
      "primary_location": {"source": {"display_name": "Physical Review Letters"}},
      "authorships": [{"author": {"display_name": "C. Jarzynski"}}],
      "biblio": {"volume": "78", "issue": "14", "first_page": "2690", "last_page": "2693"},
      "referenced_works": [
        "https://openalex.org/W1980158185",
        "https://openalex.org/W2012345678",
        "W3000000001",
        "https://openalex.org/A5023",
        ""
      ],
      "referenced_works_count": 3
    }"#;

    #[test]
    fn openalex_single_work_parses_without_an_envelope() {
        let work = parse_openalex_work(OPENALEX_WORK).expect("a work");
        assert_eq!(work.source, MetaSource::Openalex);
        assert_eq!(
            work.title,
            "Nonequilibrium Equality for Free Energy Differences"
        );
        assert_eq!(work.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(work.year, Some(1997));
        assert_eq!(work.venue.as_deref(), Some("Physical Review Letters"));
        assert_eq!(work.pages.as_deref(), Some("2690-2693"));
        assert_eq!(work.authors, vec![author("Jarzynski", "C.")]);
        assert_eq!(parse_openalex_work(r#"{"error": "Not found"}"#), None);
        assert_eq!(
            parse_openalex_work(OPENALEX_SEARCH),
            None,
            "a list is not a work"
        );
    }

    #[test]
    fn openalex_referenced_works_are_bare_work_ids() {
        assert_eq!(
            parse_openalex_referenced_works(OPENALEX_WORK),
            vec!["W1980158185", "W2012345678", "W3000000001"],
            "an author id and an empty string are not works"
        );
        assert!(parse_openalex_referenced_works(r#"{"title": "No refs"}"#).is_empty());
        assert!(parse_openalex_referenced_works("[]").is_empty());
    }

    #[test]
    fn openalex_batch_results_resolve_to_dois_with_arxiv_dois_folded() {
        let json = r#"{
          "meta": {"count": 4, "per_page": 50},
          "results": [
            {"doi": "https://doi.org/10.1103/PhysRevE.60.2721", "ids": {"doi": "https://doi.org/10.1103/PhysRevE.60.2721"}},
            {"doi": null, "ids": {"openalex": "https://openalex.org/W2", "doi": "https://doi.org/10.1000/From.Ids"}},
            {"doi": "https://doi.org/10.48550/arXiv.2401.12345", "ids": {}},
            {"doi": null, "ids": {"openalex": "https://openalex.org/W4", "mag": "4"}}
          ]
        }"#;
        assert_eq!(
            parse_openalex_dois(json),
            vec![
                CitationIdDto::Doi("10.1103/physreve.60.2721".into()),
                CitationIdDto::Doi("10.1000/from.ids".into()),
                CitationIdDto::ArXiv("2401.12345".into()),
            ]
        );
        assert!(parse_openalex_dois(r#"{"results": []}"#).is_empty());
        assert!(parse_openalex_dois("nope").is_empty());
    }

    #[test]
    fn inverted_index_reconstruction() {
        let index = serde_json::json!({
            "world": [1, 4],
            "hello": [0, 3],
            "cruel": [2],
            "skip": "not positions"
        });
        let rebuilt = rebuild_inverted_index(index.as_object().expect("object"));
        assert_eq!(rebuilt, "hello world cruel hello world");
        assert_eq!(rebuild_inverted_index(&serde_json::Map::new()), "");
    }

    // -- cache ------------------------------------------------------------

    fn sample(title: &str) -> LookupCandidateDto {
        LookupCandidateDto {
            source: MetaSource::Crossref,
            title: title.to_string(),
            authors: vec![author("Jarzynski", "C.")],
            year: Some(1997),
            venue: Some("Physical Review Letters".into()),
            volume: Some("78".into()),
            issue: None,
            pages: Some("2690-2693".into()),
            publisher: None,
            abstract_: None,
            kind: Some(PaperKind::Article),
            doi: Some("10.1103/physrevlett.78.2690".into()),
            arxiv: None,
            isbn: None,
        }
    }

    #[test]
    fn cache_keys_normalize_identifiers() {
        assert_eq!(
            LookupCache::doi_key("https://doi.org/10.1103/PhysRevLett.78.2690"),
            "doi:10.1103/physrevlett.78.2690"
        );
        assert_eq!(
            LookupCache::arxiv_key("arXiv:2401.12345v2"),
            "arxiv:2401.12345"
        );
    }

    #[test]
    fn cache_round_trips_through_a_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut cache = LookupCache::load(dir.path());
        assert!(cache.is_empty());

        cache.insert(
            LookupCache::doi_key("10.1103/PhysRevLett.78.2690"),
            sample("Jarzynski"),
        );
        let mut preprint = sample("Preprint");
        preprint.source = MetaSource::Arxiv;
        preprint.arxiv = Some("2401.12345".into());
        cache.insert(LookupCache::arxiv_key("2401.12345v1"), preprint.clone());
        cache.save(dir.path()).expect("save");
        assert!(dir.path().join(CACHE_FILE).is_file());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != CACHE_FILE)
            .collect();
        assert!(leftovers.is_empty(), "no temp file left behind");

        let reloaded = LookupCache::load(dir.path());
        assert_eq!(reloaded.len(), 2);
        assert_eq!(
            reloaded.get("doi:10.1103/physrevlett.78.2690"),
            Some(&sample("Jarzynski"))
        );
        assert_eq!(reloaded.get("arxiv:2401.12345"), Some(&preprint));
        assert_eq!(reloaded.get("doi:10.1/none"), None);

        // Saving again replaces the file in place.
        let mut again = reloaded;
        again.insert("doi:10.1/third", sample("Third"));
        again.save(dir.path()).expect("second save");
        assert_eq!(LookupCache::load(dir.path()).len(), 3);
    }

    #[test]
    fn corrupt_cache_is_discarded_and_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(CACHE_FILE), b"{ not json").expect("write");
        let mut cache = LookupCache::load(dir.path());
        assert!(cache.is_empty());

        // The right shape but the wrong types is corrupt too.
        fs::write(dir.path().join(CACHE_FILE), br#"{"doi:x": {"title": 5}}"#).expect("write");
        assert!(LookupCache::load(dir.path()).is_empty());

        cache.insert("doi:10.1/x", sample("Fresh"));
        cache.save(dir.path()).expect("save over corrupt file");
        assert_eq!(LookupCache::load(dir.path()).len(), 1);
    }

    #[test]
    fn cache_misses_stand_for_a_week_and_a_find_clears_one() {
        let week = MISS_TTL.as_secs();
        let now = 1_700_000_000;
        let mut cache = LookupCache::default();
        assert!(!cache.is_miss_at("doi:10.1/x", now));

        cache.insert_miss_at("doi:10.1/x", now);
        assert!(cache.is_miss_at("doi:10.1/x", now));
        assert!(cache.is_miss_at("doi:10.1/x", now + week - 1));
        assert!(!cache.is_miss_at("doi:10.1/x", now + week), "expired");
        assert!(
            !cache.is_miss_at("doi:10.1/x", now - 1),
            "a stamp from the future is not trusted"
        );
        assert_eq!(cache.miss_count(), 1);
        assert!(cache.is_empty(), "a miss is not an entry");

        cache.insert("doi:10.1/x", sample("Found after all"));
        assert!(!cache.is_miss_at("doi:10.1/x", now));
        assert_eq!(cache.miss_count(), 0);

        cache.insert_miss_at("doi:10.1/old", now - week);
        cache.insert_miss_at("doi:10.1/new", now - 1);
        cache.prune_at(now);
        assert_eq!(cache.miss_count(), 1);
        assert!(cache.is_miss_at("doi:10.1/new", now));

        // The live clock: a miss just recorded is one now.
        cache.insert_miss("arxiv:2401.00001");
        assert!(cache.is_miss("arxiv:2401.00001"));
        assert!(!cache.is_miss("arxiv:2401.00002"));
    }

    #[test]
    fn cache_misses_round_trip_and_an_old_plain_map_file_still_loads() {
        let dir = tempfile::tempdir().expect("temp dir");
        let now = unix_now();
        let mut cache = LookupCache::default();
        cache.insert("doi:10.1/x", sample("Kept"));
        cache.insert_miss_at("doi:10.1/missing", now);
        cache.insert_miss_at("doi:10.1/stale", now - MISS_TTL.as_secs() - 1);
        cache.save(dir.path()).expect("save");

        let text = fs::read_to_string(dir.path().join(CACHE_FILE)).expect("read");
        assert!(
            text.starts_with("{\n  \"entries\": {"),
            "the envelope: {text}"
        );
        assert!(text.contains("\"misses\": {"));

        let reloaded = LookupCache::load(dir.path());
        assert_eq!(reloaded.get("doi:10.1/x"), Some(&sample("Kept")));
        assert!(reloaded.is_miss("doi:10.1/missing"));
        assert!(!reloaded.is_miss("doi:10.1/stale"));
        assert_eq!(
            reloaded.miss_count(),
            1,
            "the stale miss was pruned on load"
        );

        // The file as written before misses existed: the entries map alone.
        let plain =
            serde_json::to_vec_pretty(&BTreeMap::from([("doi:10.1/plain", sample("Plain"))]))
                .expect("plain map");
        fs::write(dir.path().join(CACHE_FILE), plain).expect("write");
        let old = LookupCache::load(dir.path());
        assert_eq!(old.len(), 1);
        assert_eq!(old.get("doi:10.1/plain"), Some(&sample("Plain")));
        assert_eq!(old.miss_count(), 0);

        // An empty envelope is an empty cache, not a corrupt one.
        fs::write(dir.path().join(CACHE_FILE), b"{}").expect("write");
        assert!(LookupCache::load(dir.path()).is_empty());
    }

    #[test]
    fn cache_keys_for_services_titles_isbns_and_edges() {
        let doi = CitationIdDto::Doi("10.1103/PhysRevLett.78.2690".into());
        let arxiv = CitationIdDto::ArXiv("2401.12345v2".into());
        assert_eq!(LookupCache::id_key(&doi), "doi:10.1103/physrevlett.78.2690");
        assert_eq!(LookupCache::id_key(&arxiv), "arxiv:2401.12345");
        assert_eq!(
            LookupCache::source_key("s2", &doi),
            "s2:doi:10.1103/physrevlett.78.2690"
        );
        assert_eq!(
            LookupCache::source_key("inspire", &arxiv),
            "inspire:arxiv:2401.12345"
        );
        assert_eq!(
            LookupCache::title_key("  Non-Equilibrium  Equality: for Free Energy "),
            "title:nonequilibrium equality for free energy"
        );
        assert_eq!(
            LookupCache::isbn_key("978-0-306-40615-7"),
            "isbn:9780306406157"
        );
        assert_eq!(LookupCache::isbn_key(" junk "), "isbn:junk");
        assert_eq!(
            LookupCache::refs_key(&doi),
            "refs:doi:10.1103/physrevlett.78.2690"
        );
        assert_eq!(LookupCache::cites_key(&arxiv), "cites:arxiv:2401.12345");
    }

    #[test]
    fn title_lists_and_edges_round_trip_and_lists_expire_after_a_month() {
        let dir = tempfile::tempdir().expect("temp dir");
        let now = unix_now();
        let month = LIST_TTL.as_secs();
        let mut cache = LookupCache::default();
        let key = LookupCache::title_key("Jarzynski Equality");

        // An empty search is a miss under the title key.
        cache.insert_list_at(key.clone(), Vec::new(), now);
        assert!(cache.is_miss_at(&key, now));
        assert_eq!(cache.get_list_at(&key, now), None);

        // A found list clears the miss and stands for a month.
        cache.insert_list_at(key.clone(), vec![sample("One"), sample("Two")], now);
        assert!(!cache.is_miss_at(&key, now));
        assert_eq!(cache.get_list_at(&key, now).map(<[_]>::len), Some(2));
        assert_eq!(
            cache.get_list_at(&key, now + month - 1).map(<[_]>::len),
            Some(2)
        );
        assert_eq!(cache.get_list_at(&key, now + month), None, "expired");
        assert_eq!(cache.get_list_at(&key, now - 1), None, "from the future");
        assert!(cache.is_empty(), "a list is not an entry");

        let doi = CitationIdDto::Doi("10.1/a".into());
        let refs = vec![
            CitationIdDto::Doi("10.1/b".into()),
            CitationIdDto::ArXiv("2401.00001".into()),
        ];
        assert_eq!(cache.get_links(&LookupCache::refs_key(&doi)), None);
        cache.insert_links(LookupCache::refs_key(&doi), refs.clone());
        cache.insert_links(LookupCache::cites_key(&doi), Vec::new());
        cache.insert_links("junk:doi:10.1/x", refs.clone());
        cache.insert_links("refs:isbn:1", refs.clone());
        assert_eq!(cache.get_links("refs:doi:10.1/a"), Some(refs.as_slice()));
        assert_eq!(
            cache.get_links("cites:doi:10.1/a"),
            Some(&[][..]),
            "asked, none"
        );
        cache.save(dir.path()).expect("save");

        cache.insert_category("arxiv:2401.1", "hep-th".into());
        cache.save(dir.path()).expect("save");
        let reloaded = LookupCache::load(dir.path());
        assert_eq!(reloaded.get_list(&key).map(<[_]>::len), Some(2));
        assert_eq!(reloaded.get_links("refs:doi:10.1/a"), Some(refs.as_slice()));
        assert_eq!(
            reloaded.get_category("arxiv:2401.1").as_deref(),
            Some("hep-th")
        );
        assert_eq!(reloaded.get_category("arxiv:none"), None);
        let edges = reloaded.network_edges();
        assert_eq!(edges.refs.get(&doi), Some(&refs));
        assert_eq!(edges.cites.get(&doi), Some(&Vec::new()));
        assert_eq!(edges.refs.len(), 1, "keys of other shapes are left out");
        assert_eq!(edges.cites.len(), 1);

        // A stale list is pruned on load.
        let mut stale = LookupCache::default();
        stale.insert_list_at("title:old", vec![sample("Old")], now - month - 1);
        stale.insert_list_at("title:new", vec![sample("New")], now);
        stale.save(dir.path()).expect("save");
        let reloaded = LookupCache::load(dir.path());
        assert_eq!(reloaded.get_list("title:old"), None);
        assert_eq!(reloaded.get_list("title:new").map(<[_]>::len), Some(1));
    }

    #[test]
    fn cache_save_creates_a_missing_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("library").join("data");
        assert!(LookupCache::load(&nested).is_empty());
        let mut cache = LookupCache::default();
        cache.insert("doi:10.1/x", sample("Nested"));
        cache.save(&nested).expect("save into a new directory");
        assert_eq!(LookupCache::load(&nested).len(), 1);
    }
}
