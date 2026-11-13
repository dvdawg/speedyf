//! DataCite `dois/{doi}` (projects.md §4). DataCite registers the DOIs
//! Crossref does not — datasets, software, theses, and every arXiv DOI
//! (`10.48550/arxiv.…`) — so it is where a DOI goes when Crossref answers
//! 404. What leaves the machine is the DOI and nothing else; DataCite's
//! API takes no key and has no polite pool.

use crate::errors::AppResult;
use crate::library::lookup::{self, LookupSettings};
use crate::library::ratelimit::HostLimiter;
use crate::library::types::{Author, LookupCandidateDto, MetaSource, PaperKind};
use percent_encoding::utf8_percent_encode;
use serde_json::Value;

const DATACITE_DOIS: &str = "https://api.datacite.org/dois/";
const ARXIV_DOI_PREFIX: &str = "10.48550/arxiv.";

/// `dois/{doi}`. Sends the DOI. `Ok(None)` when DataCite has no record
/// (404) or the record has no title.
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
    let url = format!(
        "{DATACITE_DOIS}{}",
        utf8_percent_encode(&doi, lookup::DOI_SEGMENT)
    );
    let Some(body) = lookup::fetch_text(client, limiter, client.get(url)).await? else {
        return Ok(None);
    };
    Ok(parse_doi(&body))
}

/// Parses a DataCite `dois/{doi}` response — JSON:API, the record under
/// `data.attributes`. `None` when it is not one or has no title.
pub fn parse_doi(json: &str) -> Option<LookupCandidateDto> {
    let root: Value = serde_json::from_str(json).ok()?;
    let attributes = root.pointer("/data/attributes")?;
    let titles = attributes.get("titles").and_then(Value::as_array)?;
    // The main title is the one without a `titleType`; subtitles and
    // translations carry one.
    let title = titles
        .iter()
        .filter(|entry| entry.get("titleType").is_none_or(Value::is_null))
        .chain(titles.iter())
        .find_map(|entry| entry.get("title").and_then(lookup::text_field))?;
    let authors = attributes
        .get("creators")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(creator).collect())
        .unwrap_or_default();
    let year = attributes
        .get("publicationYear")
        .and_then(|year| match year {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        })
        .and_then(|year| i32::try_from(year).ok());
    let publisher = attributes
        .get("publisher")
        .and_then(|publisher| match publisher {
            Value::Object(_) => publisher.get("name").and_then(lookup::text_field),
            other => lookup::text_field(other),
        });
    let container = attributes.get("container");
    let container_field = |key: &str| {
        container
            .and_then(|c| c.get(key))
            .and_then(lookup::text_field)
    };
    let pages = match (container_field("firstPage"), container_field("lastPage")) {
        (Some(first), Some(last)) if first != last => Some(format!("{first}-{last}")),
        (Some(first), _) => Some(first),
        (None, _) => None,
    };
    let kind = attributes
        .pointer("/types/resourceTypeGeneral")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "Text" | "JournalArticle" => PaperKind::Article,
            "Book" => PaperKind::Book,
            "Dissertation" => PaperKind::Thesis,
            _ => PaperKind::Misc,
        });
    let abstract_ = attributes
        .get("descriptions")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .filter(|entry| {
                    entry.get("descriptionType").and_then(Value::as_str) == Some("Abstract")
                })
                .find_map(|entry| entry.get("description").and_then(lookup::text_field))
        });
    let doi = attributes
        .get("doi")
        .and_then(Value::as_str)
        .map(lookup::normalize_doi)
        .and_then(lookup::non_empty);
    // An arXiv DOI names the preprint; the id is right there in it.
    let arxiv = doi
        .as_deref()
        .and_then(|doi| doi.strip_prefix(ARXIV_DOI_PREFIX))
        .map(lookup::normalize_arxiv_id)
        .and_then(lookup::non_empty);
    Some(LookupCandidateDto {
        source: MetaSource::Datacite,
        title,
        authors,
        year,
        venue: container_field("title"),
        volume: container_field("volume"),
        issue: container_field("issue"),
        pages,
        publisher,
        abstract_,
        kind,
        doi,
        arxiv,
        isbn: None,
    })
}

/// `{familyName, givenName}` first; else `name`, which is `Family, Given`
/// for a person and the whole name for an organization.
fn creator(entry: &Value) -> Option<Author> {
    if let Some(family) = entry.get("familyName").and_then(lookup::text_field) {
        let given = entry
            .get("givenName")
            .and_then(lookup::text_field)
            .unwrap_or_default();
        return Some(Author { family, given });
    }
    let name = entry.get("name").and_then(lookup::text_field)?;
    if entry.get("nameType").and_then(Value::as_str) == Some("Organizational") {
        return Some(Author {
            family: name,
            given: String::new(),
        });
    }
    lookup::split_name(&name)
}

// ---------------------------------------------------------------------------
// Tests — no network. The fixture is trimmed from DataCite's documented
// JSON:API response for an arXiv-registered DOI.
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

    const RECORD: &str = r#"{
      "data": {
        "id": "10.48550/arxiv.2401.12345",
        "type": "dois",
        "attributes": {
          "doi": "10.48550/ARXIV.2401.12345",
          "prefix": "10.48550",
          "creators": [
            {"name": "Marchetti, M. Cristina", "nameType": "Personal", "givenName": "M. Cristina", "familyName": "Marchetti", "affiliation": []},
            {"name": "Ramaswamy, Sriram", "nameType": "Personal"},
            {"name": "ATLAS Collaboration", "nameType": "Organizational"},
            {"name": "Lone Token"},
            {"nameIdentifiers": []}
          ],
          "titles": [
            {"title": "A Subtitle", "titleType": "Subtitle"},
            {"title": "Active Matter at the\n  Mesoscale: A Review", "titleType": null}
          ],
          "publisher": "arXiv",
          "container": {"type": "Journal", "title": "Reviews of Modern Physics", "volume": "85", "issue": "3", "firstPage": "1143", "lastPage": "1189"},
          "publicationYear": 2024,
          "types": {"resourceTypeGeneral": "Text", "resourceType": "Article", "bibtex": "misc"},
          "descriptions": [
            {"description": "Not this one", "descriptionType": "Other"},
            {"description": "We review the hydrodynamics of\n active matter.", "descriptionType": "Abstract"}
          ],
          "url": "https://arxiv.org/abs/2401.12345"
        }
      }
    }"#;

    #[test]
    fn datacite_record_parses_every_field() {
        let record = parse_doi(RECORD).expect("a record");
        assert_eq!(record.source, MetaSource::Datacite);
        assert_eq!(
            record.title, "Active Matter at the Mesoscale: A Review",
            "the untyped title"
        );
        assert_eq!(
            record.authors,
            vec![
                author("Marchetti", "M. Cristina"),
                author("Ramaswamy", "Sriram"),
                author("ATLAS Collaboration", ""),
                author("Token", "Lone"),
            ]
        );
        assert_eq!(record.year, Some(2024));
        assert_eq!(record.venue.as_deref(), Some("Reviews of Modern Physics"));
        assert_eq!(record.volume.as_deref(), Some("85"));
        assert_eq!(record.issue.as_deref(), Some("3"));
        assert_eq!(record.pages.as_deref(), Some("1143-1189"));
        assert_eq!(record.publisher.as_deref(), Some("arXiv"));
        assert_eq!(
            record.abstract_.as_deref(),
            Some("We review the hydrodynamics of active matter.")
        );
        assert_eq!(record.kind, Some(PaperKind::Article));
        assert_eq!(record.doi.as_deref(), Some("10.48550/arxiv.2401.12345"));
        assert_eq!(
            record.arxiv.as_deref(),
            Some("2401.12345"),
            "read off the arXiv DOI"
        );
        assert_eq!(record.isbn, None);
    }

    #[test]
    fn datacite_tolerates_missing_fields_and_other_shapes() {
        let json = r#"{"data": {"attributes": {
            "doi": "10.5281/zenodo.1234",
            "titles": [{"title": "A Dataset"}],
            "publisher": {"name": "Zenodo", "publisherIdentifier": "https://ror.org/x"},
            "publicationYear": "2020",
            "container": {"firstPage": "7"},
            "types": {"resourceTypeGeneral": "Dataset"}
        }}}"#;
        let record = parse_doi(json).expect("a record");
        assert!(record.authors.is_empty());
        assert_eq!(record.year, Some(2020));
        assert_eq!(record.publisher.as_deref(), Some("Zenodo"));
        assert_eq!(record.pages.as_deref(), Some("7"));
        assert_eq!(record.venue, None);
        assert_eq!(record.kind, Some(PaperKind::Misc));
        assert_eq!(record.abstract_, None);
        assert_eq!(record.arxiv, None);
        assert_eq!(record.doi.as_deref(), Some("10.5281/zenodo.1234"));
    }

    #[test]
    fn datacite_type_mapping() {
        let kind = |t: &str| {
            parse_doi(&format!(
                r#"{{"data": {{"attributes": {{"titles": [{{"title": "T"}}], "types": {{"resourceTypeGeneral": "{t}"}}}}}}}}"#
            ))
            .and_then(|r| r.kind)
        };
        assert_eq!(kind("Text"), Some(PaperKind::Article));
        assert_eq!(kind("JournalArticle"), Some(PaperKind::Article));
        assert_eq!(kind("Book"), Some(PaperKind::Book));
        assert_eq!(kind("Dissertation"), Some(PaperKind::Thesis));
        assert_eq!(kind("Software"), Some(PaperKind::Misc));
        let untyped = r#"{"data": {"attributes": {"titles": [{"title": "T"}]}}}"#;
        assert_eq!(parse_doi(untyped).and_then(|r| r.kind), None);
    }

    #[test]
    fn datacite_rejects_what_is_not_a_titled_record() {
        assert_eq!(parse_doi("not json"), None);
        assert_eq!(
            parse_doi(
                r#"{"errors": [{"status": "404", "title": "The resource you are looking for doesn't exist."}]}"#
            ),
            None
        );
        assert_eq!(
            parse_doi(r#"{"data": {"attributes": {"doi": "10.1/x", "titles": []}}}"#),
            None
        );
        assert_eq!(
            parse_doi(r#"{"data": {"attributes": {"doi": "10.1/x", "titles": [{"title": " "}]}}}"#),
            None
        );
        assert_eq!(
            parse_doi(r#"{"data": {"attributes": {"doi": "10.1/x"}}}"#),
            None
        );
    }

    #[test]
    fn datacite_refuses_when_lookup_is_off() {
        let off = LookupSettings::default();
        let client = lookup::client();
        let limiter = HostLimiter::new();
        let result =
            tauri::async_runtime::block_on(fetch_by_doi(&client, &limiter, &off, "10.1/x"));
        assert!(result.is_err());
    }
}
