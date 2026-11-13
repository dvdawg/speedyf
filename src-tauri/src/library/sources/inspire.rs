//! INSPIRE-HEP `api/literature` (projects.md sec. 4). INSPIRE curates
//! high-energy physics by hand and has the best record for a paper it
//! knows: journal, volume and pages, with the arXiv id and DOI together.
//! What leaves the machine is the query - an arXiv id (`q=arxiv:...`), a DOI
//! (`q=doi:...`), or for a title search the title's words (`q=t "..."`) - and
//! the list of fields wanted. No key, no contact.

use crate::errors::AppResult;
use crate::library::lookup::{self, LookupSettings};
use crate::library::ratelimit::HostLimiter;
use crate::library::types::{LookupCandidateDto, MetaSource, PaperKind};
use serde_json::Value;

const INSPIRE_LITERATURE: &str = "https://inspirehep.net/api/literature";
/// Only what the candidate needs; INSPIRE records are large.
const FIELDS: &str = "titles,authors.full_name,publication_info,dois,arxiv_eprints,abstracts,document_type,earliest_date,texkeys,isbns";

/// `literature?q=arxiv:{id}`. Sends the arXiv id. `Ok(None)` when INSPIRE
/// has no record.
pub async fn fetch_by_arxiv(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    arxiv_id: &str,
) -> AppResult<Option<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let id = lookup::normalize_arxiv_id(arxiv_id);
    if id.is_empty() {
        return Ok(None);
    }
    let found = search(client, limiter, &format!("arxiv:{id}")).await?;
    Ok(found.into_iter().next())
}

/// `literature?q=doi:{doi}`. Sends the DOI. `Ok(None)` when INSPIRE has no
/// record.
pub async fn fetch_by_doi(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    doi: &str,
) -> AppResult<Option<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let doi = lookup::normalize_doi(doi);
    if doi.is_empty() {
        return Ok(None);
    }
    let found = search(client, limiter, &format!("doi:{doi}")).await?;
    Ok(found.into_iter().next())
}

/// `literature?q=t "{title}"`. Sends the title's words. Up to five
/// candidates in INSPIRE's relevance order.
pub async fn search_by_title(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    title: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let words = lookup::query_words(title);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    search(client, limiter, &format!("t \"{words}\"")).await
}

/// One `literature` query, `size=5`, trimmed to the candidate's fields.
async fn search(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    q: &str,
) -> AppResult<Vec<LookupCandidateDto>> {
    let size = lookup::TITLE_RESULTS.to_string();
    let request = client.get(INSPIRE_LITERATURE).query(&[
        ("q", q),
        ("size", size.as_str()),
        ("fields", FIELDS),
    ]);
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_search(&body);
    candidates.truncate(lookup::TITLE_RESULTS);
    Ok(candidates)
}

/// Parses a `literature` search response: one candidate per titled record
/// in `hits.hits[].metadata`, in API order. Not JSON, or no hits -> empty.
pub fn parse_search(json: &str) -> Vec<LookupCandidateDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.pointer("/hits/hits")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| record(hit.get("metadata")?))
                .collect()
        })
        .unwrap_or_default()
}

fn record(metadata: &Value) -> Option<LookupCandidateDto> {
    let first = |key: &str| metadata.get(key)?.as_array()?.first();
    let first_value = |key: &str| first(key)?.get("value").and_then(lookup::text_field);
    let title = first("titles")?.get("title").and_then(lookup::text_field)?;
    let authors = metadata
        .get("authors")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|author| lookup::split_name(author.get("full_name")?.as_str()?))
                .collect()
        })
        .unwrap_or_default();
    let info = first("publication_info");
    let info_field = |key: &str| info.and_then(|i| i.get(key)).and_then(lookup::text_field);
    let year = info
        .and_then(|i| i.get("year"))
        .and_then(Value::as_i64)
        .and_then(|year| i32::try_from(year).ok())
        .or_else(|| {
            metadata
                .get("earliest_date")?
                .as_str()?
                .get(..4)?
                .parse()
                .ok()
        });
    let pages =
        info_field("artid").or_else(
            || match (info_field("page_start"), info_field("page_end")) {
                (Some(start), Some(end)) if start != end => Some(format!("{start}-{end}")),
                (Some(start), _) => Some(start),
                (None, _) => None,
            },
        );
    let kind = first("document_type")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "article" => PaperKind::Article,
            "conference paper" => PaperKind::Inproceedings,
            "book" => PaperKind::Book,
            "thesis" => PaperKind::Thesis,
            _ => PaperKind::Misc,
        });
    Some(LookupCandidateDto {
        source: MetaSource::Inspire,
        title,
        authors,
        year,
        venue: info_field("journal_title"),
        volume: info_field("journal_volume"),
        issue: info_field("journal_issue"),
        pages,
        publisher: None,
        abstract_: first_value("abstracts"),
        kind,
        doi: first_value("dois")
            .map(|doi| lookup::normalize_doi(&doi))
            .and_then(lookup::non_empty),
        arxiv: first_value("arxiv_eprints")
            .map(|id| lookup::normalize_arxiv_id(&id))
            .and_then(lookup::non_empty),
        isbn: first_value("isbns").and_then(|isbn| crate::library::normalize_isbn(&isbn)),
    })
}

// ---------------------------------------------------------------------------
// Tests - no network. The fixture is trimmed from INSPIRE's documented
// literature record shape.
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

    const SEARCH: &str = r#"{
      "hits": {
        "hits": [
          {
            "id": "1234567",
            "created": "2013-04-10T00:00:00+00:00",
            "metadata": {
              "titles": [{"title": "Hydrodynamics of soft\n active matter", "source": "arXiv"}, {"title": "Alternative title"}],
              "authors": [
                {"full_name": "Marchetti, M. Cristina", "recid": 1},
                {"full_name": "Ramaswamy, Sriram"},
                {"full_name": "Ramaswamy"},
                {"recid": 4}
              ],
              "publication_info": [
                {"journal_title": "Rev.Mod.Phys.", "journal_volume": "85", "journal_issue": "3", "artid": "1143", "year": 2013},
                {"journal_title": "Later reprint"}
              ],
              "dois": [{"value": "10.1103/RevModPhys.85.1143"}, {"value": "10.1103/other"}],
              "arxiv_eprints": [{"value": "1207.2929v2", "categories": ["cond-mat.soft"]}],
              "abstracts": [{"value": "This article reviews the\n  hydrodynamics of active matter.", "source": "arXiv"}],
              "document_type": ["article"],
              "earliest_date": "2012-07-12",
              "texkeys": ["Marchetti:2013zda"],
              "control_number": 1234567
            }
          },
          {
            "id": "2",
            "metadata": {
              "titles": [{"title": "A thesis"}],
              "publication_info": [{"page_start": "10", "page_end": "20"}],
              "document_type": ["thesis"],
              "earliest_date": "2020-01-01",
              "isbns": [{"value": "978-0-306-40615-7"}]
            }
          },
          {"id": "3", "metadata": {"titles": [{"title": "Pages the same"}, {"title": "x"}], "publication_info": [{"page_start": "7", "page_end": "7"}], "document_type": ["conference paper"]}},
          {"id": "4", "metadata": {"titles": []}},
          {"id": "5"}
        ],
        "total": 5
      },
      "links": {"self": "https://inspirehep.net/api/literature?q=arxiv%3A1207.2929&size=5"}
    }"#;

    #[test]
    fn inspire_hits_parse_in_order_with_every_field() {
        let found = parse_search(SEARCH);
        assert_eq!(
            found.len(),
            3,
            "untitled and metadata-less hits are skipped"
        );

        let first = &found[0];
        assert_eq!(first.source, MetaSource::Inspire);
        assert_eq!(first.title, "Hydrodynamics of soft active matter");
        assert_eq!(
            first.authors,
            vec![
                author("Marchetti", "M. Cristina"),
                author("Ramaswamy", "Sriram"),
                author("Ramaswamy", ""),
            ]
        );
        assert_eq!(
            first.year,
            Some(2013),
            "publication_info year before earliest_date"
        );
        assert_eq!(first.venue.as_deref(), Some("Rev.Mod.Phys."));
        assert_eq!(first.volume.as_deref(), Some("85"));
        assert_eq!(first.issue.as_deref(), Some("3"));
        assert_eq!(first.pages.as_deref(), Some("1143"), "artid");
        assert_eq!(first.publisher, None);
        assert_eq!(
            first.abstract_.as_deref(),
            Some("This article reviews the hydrodynamics of active matter.")
        );
        assert_eq!(first.kind, Some(PaperKind::Article));
        assert_eq!(
            first.doi.as_deref(),
            Some("10.1103/revmodphys.85.1143"),
            "the first DOI"
        );
        assert_eq!(
            first.arxiv.as_deref(),
            Some("1207.2929"),
            "version stripped"
        );
        assert_eq!(first.isbn, None);

        let thesis = &found[1];
        assert_eq!(thesis.title, "A thesis");
        assert!(thesis.authors.is_empty());
        assert_eq!(thesis.year, Some(2020), "from earliest_date");
        assert_eq!(thesis.pages.as_deref(), Some("10-20"));
        assert_eq!(thesis.kind, Some(PaperKind::Thesis));
        assert_eq!(thesis.doi, None);
        assert_eq!(thesis.arxiv, None);
        assert_eq!(thesis.isbn.as_deref(), Some("9780306406157"));

        let same = &found[2];
        assert_eq!(same.pages.as_deref(), Some("7"));
        assert_eq!(same.kind, Some(PaperKind::Inproceedings));
        assert_eq!(same.year, None);
    }

    #[test]
    fn inspire_type_mapping() {
        let kind = |t: &str| {
            parse_search(&format!(
                r#"{{"hits": {{"hits": [{{"metadata": {{"titles": [{{"title": "T"}}], "document_type": ["{t}"]}}}}]}}}}"#
            ))
            .into_iter()
            .next()
            .and_then(|r| r.kind)
        };
        assert_eq!(kind("article"), Some(PaperKind::Article));
        assert_eq!(kind("conference paper"), Some(PaperKind::Inproceedings));
        assert_eq!(kind("book"), Some(PaperKind::Book));
        assert_eq!(kind("thesis"), Some(PaperKind::Thesis));
        assert_eq!(kind("proceedings"), Some(PaperKind::Misc));
    }

    #[test]
    fn inspire_empty_and_junk_yield_nothing() {
        assert!(parse_search(r#"{"hits": {"hits": [], "total": 0}}"#).is_empty());
        assert!(parse_search(r#"{"message": "Malformed query", "status": 400}"#).is_empty());
        assert!(parse_search("").is_empty());
        assert!(parse_search("[1]").is_empty());
    }

    #[test]
    fn inspire_refuses_when_lookup_is_off() {
        let off = LookupSettings::default();
        let client = lookup::client();
        let limiter = HostLimiter::new();
        fn run<F: std::future::Future>(future: F) -> F::Output {
            tauri::async_runtime::block_on(future)
        }
        assert!(run(fetch_by_arxiv(&client, &limiter, &off, "1207.2929")).is_err());
        assert!(run(fetch_by_doi(&client, &limiter, &off, "10.1/x")).is_err());
        assert!(run(search_by_title(&client, &limiter, &off, "Active matter")).is_err());
    }
}
