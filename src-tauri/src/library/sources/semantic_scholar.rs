//! Semantic Scholar's Academic Graph API (projects.md sec. 4). It knows most
//! papers in computer science and the sciences by DOI or arXiv id, and -
//! what no other source here offers - what each paper cites and what cites
//! it. What leaves the machine: for a paper, its DOI or arXiv id
//! (`paper/DOI:...`, `paper/ARXIV:...`); for its references and citations, the
//! same id; for a title search, the title. The optional key (Settings ->
//! Semantic Scholar API key) goes in the `x-api-key` header and buys a
//! better rate limit, nothing else; without it the limiter spaces requests
//! three seconds apart. No contact is sent.

use crate::engine::citation::fold_arxiv_doi;
use crate::engine::types::CitationIdDto;
use crate::errors::AppResult;
use crate::library::lookup::{self, LookupSettings};
use crate::library::ratelimit::HostLimiter;
use crate::library::types::{LookupCandidateDto, MetaSource, PaperKind};
use percent_encoding::utf8_percent_encode;
use serde_json::Value;

const S2_PAPER: &str = "https://api.semanticscholar.org/graph/v1/paper/";
const S2_SEARCH: &str = "https://api.semanticscholar.org/graph/v1/paper/search";
const PAPER_FIELDS: &str =
    "title,authors,year,venue,journal,externalIds,abstract,publicationTypes,referenceCount";
const LINK_FIELDS: &str = "externalIds";
/// The most references or citations one request returns; the API's cap.
const LINK_LIMIT: &str = "1000";
const KEY_HEADER: &str = "x-api-key";

/// `paper/{DOI:...|ARXIV:...}?fields=...`. Sends the identifier. `Ok(None)` on
/// 404 or a record without a title.
pub async fn fetch_by_id(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    id: &CitationIdDto,
) -> AppResult<Option<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let Some(path) = paper_path(id) else {
        return Ok(None);
    };
    let request =
        request(client, settings, format!("{S2_PAPER}{path}")).query(&[("fields", PAPER_FIELDS)]);
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(None);
    };
    Ok(parse_paper(&body))
}

/// `paper/{id}/references?fields=externalIds&limit=1000`: what the paper
/// cites, as DOIs (lower-cased, an arXiv DOI folded to its id) or arXiv
/// ids (version stripped). Sends the identifier. Empty on 404.
pub async fn fetch_references(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    id: &CitationIdDto,
) -> AppResult<Vec<CitationIdDto>> {
    fetch_links(client, limiter, settings, id, "references").await
}

/// `paper/{id}/citations?fields=externalIds&limit=1000`: what cites the
/// paper, in the same form. Sends the identifier. Empty on 404.
pub async fn fetch_citations(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    id: &CitationIdDto,
) -> AppResult<Vec<CitationIdDto>> {
    fetch_links(client, limiter, settings, id, "citations").await
}

async fn fetch_links(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    id: &CitationIdDto,
    direction: &str,
) -> AppResult<Vec<CitationIdDto>> {
    lookup::ensure_enabled(settings)?;
    let Some(path) = paper_path(id) else {
        return Ok(Vec::new());
    };
    let request = request(client, settings, format!("{S2_PAPER}{path}/{direction}"))
        .query(&[("fields", LINK_FIELDS), ("limit", LINK_LIMIT)]);
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    Ok(parse_links(&body))
}

/// `paper/search?query={title}&limit=5&fields=...`. Sends the title. Up to
/// five candidates in Semantic Scholar's relevance order.
pub async fn search_by_title(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    title: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let title = lookup::collapse_whitespace(title);
    if title.is_empty() {
        return Ok(Vec::new());
    }
    let limit = lookup::TITLE_RESULTS.to_string();
    let request = request(client, settings, S2_SEARCH.to_string()).query(&[
        ("query", title.as_str()),
        ("limit", limit.as_str()),
        ("fields", PAPER_FIELDS),
    ]);
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_search(&body);
    candidates.truncate(lookup::TITLE_RESULTS);
    Ok(candidates)
}

/// `DOI:{doi}` or `ARXIV:{id}`, normalized; `None` for an empty id.
fn paper_path(id: &CitationIdDto) -> Option<String> {
    match id {
        CitationIdDto::Doi(doi) => {
            let doi = lookup::normalize_doi(doi);
            (!doi.is_empty())
                .then(|| format!("DOI:{}", utf8_percent_encode(&doi, lookup::DOI_SEGMENT)))
        }
        CitationIdDto::ArXiv(arxiv) => {
            let id = lookup::normalize_arxiv_id(arxiv);
            (!id.is_empty()).then(|| format!("ARXIV:{id}"))
        }
    }
}

/// A GET with the key header when a key is set; the limiter reads the
/// header to pick the keyed interval.
fn request(
    client: &reqwest::Client,
    settings: &LookupSettings,
    url: String,
) -> reqwest::RequestBuilder {
    let request = client.get(url);
    match settings.s2_key() {
        Some(key) => request.header(KEY_HEADER, key),
        None => request,
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses a `paper/{id}` response: one paper object. `None` when it is not
/// one or has no title.
pub fn parse_paper(json: &str) -> Option<LookupCandidateDto> {
    let root: Value = serde_json::from_str(json).ok()?;
    paper(&root)
}

/// Parses a `paper/search` response: one candidate per titled paper in
/// `data`, in API order. Not JSON, or no `data` -> empty.
pub fn parse_search(json: &str) -> Vec<LookupCandidateDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.get("data")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(paper).collect())
        .unwrap_or_default()
}

/// Parses a `references` or `citations` response: for each linked paper
/// (`citedPaper` or `citingPaper`) its DOI, else its arXiv id; papers with
/// neither are left out, and duplicates collapsed.
pub fn parse_links(json: &str) -> Vec<CitationIdDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut ids: Vec<CitationIdDto> = root
        .get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    let linked = entry
                        .get("citedPaper")
                        .or_else(|| entry.get("citingPaper"))?;
                    external_id(linked.get("externalIds")?)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

fn paper(paper: &Value) -> Option<LookupCandidateDto> {
    if !paper.is_object() {
        return None;
    }
    let title = paper.get("title").and_then(lookup::text_field)?;
    let authors = paper
        .get("authors")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|author| lookup::split_name(author.get("name")?.as_str()?))
                .collect()
        })
        .unwrap_or_default();
    let year = paper
        .get("year")
        .and_then(Value::as_i64)
        .and_then(|year| i32::try_from(year).ok());
    let journal = paper.get("journal").filter(|j| j.is_object());
    let journal_field = |key: &str| {
        journal
            .and_then(|j| j.get(key))
            .and_then(lookup::text_field)
    };
    let venue = journal_field("name").or_else(|| paper.get("venue").and_then(lookup::text_field));
    let kind = paper
        .get("publicationTypes")
        .and_then(Value::as_array)
        .and_then(|types| {
            types
                .iter()
                .filter_map(Value::as_str)
                .find_map(|kind| match kind {
                    "JournalArticle" => Some(PaperKind::Article),
                    "Conference" => Some(PaperKind::Inproceedings),
                    "Book" => Some(PaperKind::Book),
                    _ => None,
                })
        });
    let external = paper.get("externalIds");
    let doi = external
        .and_then(|ids| ids.get("DOI"))
        .and_then(Value::as_str)
        .map(lookup::normalize_doi)
        .and_then(lookup::non_empty);
    let arxiv = external
        .and_then(|ids| ids.get("ArXiv"))
        .and_then(Value::as_str)
        .map(lookup::normalize_arxiv_id)
        .and_then(lookup::non_empty);
    Some(LookupCandidateDto {
        source: MetaSource::SemanticScholar,
        title,
        authors,
        year,
        venue,
        volume: journal_field("volume"),
        issue: None,
        pages: journal_field("pages"),
        publisher: None,
        abstract_: paper.get("abstract").and_then(lookup::text_field),
        kind,
        doi,
        arxiv,
        isbn: None,
    })
}

/// The DOI, else the arXiv id, of an `externalIds` object.
fn external_id(ids: &Value) -> Option<CitationIdDto> {
    if let Some(doi) = ids
        .get("DOI")
        .and_then(Value::as_str)
        .map(lookup::normalize_doi)
        .and_then(lookup::non_empty)
    {
        return Some(fold_arxiv_doi(CitationIdDto::Doi(doi)));
    }
    ids.get("ArXiv")
        .and_then(Value::as_str)
        .map(lookup::normalize_arxiv_id)
        .and_then(lookup::non_empty)
        .map(CitationIdDto::ArXiv)
}

// ---------------------------------------------------------------------------
// Tests - no network. Fixtures are trimmed from the Academic Graph API's
// documented response shapes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::Author;

    fn author(family: &str, given: &str) -> Author {
        Author {
            family: family.to_string(),
            given: given.to_string(),
        }
    }

    const PAPER: &str = r#"{
      "paperId": "649def34f8be52c8b66281af98ae884c09aef38b",
      "externalIds": {"ArXiv": "1706.03762v5", "DBLP": "conf/nips/VaswaniSPUJGKP17", "MAG": "2963403868", "DOI": "10.5555/3295222.3295349", "CorpusId": 13756489},
      "title": "Attention is All you\n Need",
      "abstract": "The dominant sequence transduction models are based on\n complex recurrent or convolutional neural networks.",
      "venue": "Neural Information Processing Systems",
      "year": 2017,
      "referenceCount": 41,
      "journal": {"name": "ArXiv", "volume": "abs/1706.03762", "pages": "5998-6008"},
      "publicationTypes": ["Review", "JournalArticle", "Conference"],
      "authors": [
        {"authorId": "40348417", "name": "Ashish Vaswani"},
        {"authorId": "1846258", "name": "Noam M. Shazeer"},
        {"authorId": null, "name": "   "}
      ]
    }"#;

    #[test]
    fn paper_parses_every_field() {
        let paper = parse_paper(PAPER).expect("a paper");
        assert_eq!(paper.source, MetaSource::SemanticScholar);
        assert_eq!(paper.title, "Attention is All you Need");
        assert_eq!(
            paper.authors,
            vec![author("Vaswani", "Ashish"), author("Shazeer", "Noam M.")]
        );
        assert_eq!(paper.year, Some(2017));
        assert_eq!(
            paper.venue.as_deref(),
            Some("ArXiv"),
            "journal.name before venue"
        );
        assert_eq!(paper.volume.as_deref(), Some("abs/1706.03762"));
        assert_eq!(paper.pages.as_deref(), Some("5998-6008"));
        assert_eq!(paper.issue, None);
        assert_eq!(paper.publisher, None);
        assert_eq!(
            paper.abstract_.as_deref(),
            Some("The dominant sequence transduction models are based on complex recurrent or convolutional neural networks.")
        );
        assert_eq!(
            paper.kind,
            Some(PaperKind::Article),
            "the first recognized type"
        );
        assert_eq!(paper.doi.as_deref(), Some("10.5555/3295222.3295349"));
        assert_eq!(
            paper.arxiv.as_deref(),
            Some("1706.03762"),
            "version stripped"
        );
    }

    #[test]
    fn paper_tolerates_missing_fields() {
        let json = r#"{"paperId": "abc", "title": "Bare", "journal": null, "venue": "NeurIPS",
                       "publicationTypes": ["Review"], "externalIds": {"CorpusId": 1}, "authors": []}"#;
        let paper = parse_paper(json).expect("a paper");
        assert_eq!(paper.venue.as_deref(), Some("NeurIPS"));
        assert_eq!(paper.kind, None, "no recognized type");
        assert_eq!(paper.doi, None);
        assert_eq!(paper.arxiv, None);
        assert!(paper.authors.is_empty());
        assert_eq!(paper.year, None);

        let conference = r#"{"title": "C", "publicationTypes": ["Conference"]}"#;
        assert_eq!(
            parse_paper(conference).and_then(|p| p.kind),
            Some(PaperKind::Inproceedings)
        );
        let book = r#"{"title": "B", "publicationTypes": ["Book"]}"#;
        assert_eq!(
            parse_paper(book).and_then(|p| p.kind),
            Some(PaperKind::Book)
        );
    }

    #[test]
    fn paper_rejects_what_is_not_a_titled_paper() {
        assert_eq!(parse_paper("not json"), None);
        assert_eq!(
            parse_paper(r#"{"error": "Paper with id DOI:10.1/x not found"}"#),
            None
        );
        assert_eq!(parse_paper(r#"{"paperId": "x", "title": null}"#), None);
        assert_eq!(parse_paper("[]"), None);
    }

    #[test]
    fn search_parses_data_in_order() {
        let json = r#"{"total": 3, "offset": 0, "next": 3, "data": [
            {"paperId": "1", "title": "First", "year": 2001},
            {"paperId": "2", "title": null},
            {"paperId": "3", "title": "Third", "externalIds": {"DOI": "10.1/THIRD"}}
        ]}"#;
        let found = parse_search(json);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "First");
        assert_eq!(found[0].year, Some(2001));
        assert_eq!(found[1].doi.as_deref(), Some("10.1/third"));
        assert!(parse_search(r#"{"total": 0, "offset": 0, "data": []}"#).is_empty());
        assert!(parse_search(r#"{"error": "bad query"}"#).is_empty());
        assert!(parse_search("").is_empty());
    }

    #[test]
    fn references_and_citations_become_identifiers() {
        let references = r#"{"offset": 0, "data": [
            {"contexts": [], "citedPaper": {"paperId": "a", "externalIds": {"DOI": "10.1103/PhysRevE.60.2721", "ArXiv": "cond-mat/9901352v4"}}},
            {"citedPaper": {"paperId": "b", "externalIds": {"ArXiv": "2401.12345v2"}}},
            {"citedPaper": {"paperId": "c", "externalIds": {"DOI": "10.48550/arXiv.1706.03762"}}},
            {"citedPaper": {"paperId": "d", "externalIds": {"MAG": "123"}}},
            {"citedPaper": {"paperId": "e", "externalIds": null}},
            {"citedPaper": null},
            {"citedPaper": {"paperId": "a-again", "externalIds": {"DOI": "10.1103/physreve.60.2721"}}}
        ]}"#;
        assert_eq!(
            parse_links(references),
            vec![
                CitationIdDto::Doi("10.1103/physreve.60.2721".into()),
                CitationIdDto::ArXiv("2401.12345".into()),
                CitationIdDto::ArXiv("1706.03762".into()),
            ],
            "DOI before arXiv id; arXiv DOIs folded; no id, no entry; duplicates dropped"
        );
        let citations = r#"{"offset": 0, "data": [
            {"citingPaper": {"paperId": "x", "externalIds": {"DOI": "10.1000/Citing"}}}
        ]}"#;
        assert_eq!(
            parse_links(citations),
            vec![CitationIdDto::Doi("10.1000/citing".into())]
        );
        assert!(parse_links(r#"{"offset": 0, "data": []}"#).is_empty());
        assert!(parse_links(r#"{"error": "not found"}"#).is_empty());
        assert!(parse_links("").is_empty());
    }

    #[test]
    fn paper_paths_are_prefixed_and_normalized() {
        assert_eq!(
            paper_path(&CitationIdDto::Doi(
                "https://doi.org/10.1103/PhysRevLett.78.2690".into()
            ))
            .as_deref(),
            Some("DOI:10.1103/physrevlett.78.2690")
        );
        assert_eq!(
            paper_path(&CitationIdDto::Doi("10.1000/a#b".into())).as_deref(),
            Some("DOI:10.1000/a%23b")
        );
        assert_eq!(
            paper_path(&CitationIdDto::ArXiv("arXiv:2401.12345v2".into())).as_deref(),
            Some("ARXIV:2401.12345")
        );
        assert_eq!(
            paper_path(&CitationIdDto::ArXiv("hep-th/9901001v1".into())).as_deref(),
            Some("ARXIV:hep-th/9901001")
        );
        assert_eq!(paper_path(&CitationIdDto::Doi("  ".into())), None);
        assert_eq!(paper_path(&CitationIdDto::ArXiv("".into())), None);
    }

    #[test]
    fn the_key_header_is_sent_only_when_a_key_is_set() {
        let client = lookup::client();
        let without = LookupSettings {
            enabled: true,
            ..Default::default()
        };
        let built = request(&client, &without, S2_SEARCH.to_string())
            .build()
            .expect("request");
        assert!(!built.headers().contains_key(KEY_HEADER));

        let with = LookupSettings {
            enabled: true,
            contact: String::new(),
            s2_key: " secret ".into(),
        };
        let built = request(&client, &with, S2_SEARCH.to_string())
            .build()
            .expect("request");
        assert_eq!(
            built
                .headers()
                .get(KEY_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("secret")
        );
        assert_eq!(built.url().host_str(), Some("api.semanticscholar.org"));
    }

    #[test]
    fn every_fetch_refuses_when_lookup_is_off() {
        let off = LookupSettings::default();
        let client = lookup::client();
        let limiter = HostLimiter::new();
        fn run<F: std::future::Future>(future: F) -> F::Output {
            tauri::async_runtime::block_on(future)
        }
        let id = CitationIdDto::Doi("10.1/x".into());
        assert!(run(fetch_by_id(&client, &limiter, &off, &id)).is_err());
        assert!(run(fetch_references(&client, &limiter, &off, &id)).is_err());
        assert!(run(fetch_citations(&client, &limiter, &off, &id)).is_err());
        assert!(run(search_by_title(&client, &limiter, &off, "A title")).is_err());
    }
}
