//! dblp `search/publ/api` (projects.md sec. 4). dblp is computer science's
//! bibliography, and the one service here that knows a workshop paper by
//! its title. What leaves the machine is the title, for a title search -
//! dblp has no lookup by identifier. No key, no contact.

use crate::errors::AppResult;
use crate::library::lookup::{self, LookupSettings};
use crate::library::ratelimit::HostLimiter;
use crate::library::types::{Author, LookupCandidateDto, MetaSource, PaperKind};
use serde_json::Value;

const DBLP_SEARCH: &str = "https://dblp.org/search/publ/api";

/// `search/publ/api?q={title}&format=json&h=5`. Sends the title. Up to
/// five candidates in dblp's relevance order.
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
    let hits = lookup::TITLE_RESULTS.to_string();
    let request = client.get(DBLP_SEARCH).query(&[
        ("q", title.as_str()),
        ("format", "json"),
        ("h", hits.as_str()),
    ]);
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(Vec::new());
    };
    let mut candidates = parse_search(&body);
    candidates.truncate(lookup::TITLE_RESULTS);
    Ok(candidates)
}

/// Parses a dblp search response: one candidate per titled
/// `result.hits.hit[].info`, in API order. No hits (dblp then omits `hit`
/// altogether), not JSON -> empty.
pub fn parse_search(json: &str) -> Vec<LookupCandidateDto> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.pointer("/result/hits/hit")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| info(hit.get("info")?))
                .collect()
        })
        .unwrap_or_default()
}

fn info(info: &Value) -> Option<LookupCandidateDto> {
    let title = info
        .get("title")
        .and_then(lookup::text_field)
        .map(|title| title.trim_end_matches('.').trim_end().to_string())
        .and_then(lookup::non_empty)?;
    // `authors.author` is one object or an array of them.
    let authors = info
        .pointer("/authors/author")
        .map(|author| match author {
            Value::Array(list) => list.iter().filter_map(dblp_author).collect(),
            one => dblp_author(one).into_iter().collect(),
        })
        .unwrap_or_default();
    let field = |key: &str| info.get(key).and_then(lookup::text_field);
    let year = field("year").and_then(|year| year.parse().ok());
    let kind = info
        .get("type")
        .and_then(Value::as_str)
        .map(|kind| match kind {
            "Journal Articles" => PaperKind::Article,
            "Conference and Workshop Papers" => PaperKind::Inproceedings,
            "Books and Theses" => PaperKind::Book,
            _ => PaperKind::Misc,
        });
    Some(LookupCandidateDto {
        source: MetaSource::Dblp,
        title,
        authors,
        year,
        venue: field("venue"),
        volume: field("volume"),
        issue: field("number"),
        pages: field("pages"),
        publisher: field("publisher"),
        abstract_: None,
        kind,
        doi: field("doi")
            .map(|doi| lookup::normalize_doi(&doi))
            .and_then(lookup::non_empty),
        arxiv: None,
        isbn: field("isbn").and_then(|isbn| crate::library::normalize_isbn(&isbn)),
    })
}

/// `{"text": "Given Family", "@pid": ...}` or a bare string. dblp tells
/// namesakes apart with a four-digit suffix (`Wei Wang 0001`), which is
/// not part of the name.
fn dblp_author(author: &Value) -> Option<Author> {
    let text = match author {
        Value::Object(_) => author.get("text").and_then(lookup::text_field)?,
        other => lookup::text_field(other)?,
    };
    let name = match text.rsplit_once(' ') {
        Some((head, tail)) if tail.len() == 4 && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => text.as_str(),
    };
    lookup::split_name(name)
}

// ---------------------------------------------------------------------------
// Tests - no network. The fixture is trimmed from dblp's documented JSON
// search response.
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

    const SEARCH: &str = r#"{
      "result": {
        "query": "attention is all you need",
        "status": {"@code": "200", "text": "OK"},
        "time": {"@unit": "msecs", "text": "12.3"},
        "completions": {"@total": "1", "@computed": "1", "@sent": "1", "c": {"@sc": "5", "@dc": "5", "@oc": "5", "text": "need"}},
        "hits": {
          "@total": "3", "@computed": "3", "@sent": "3", "@first": "0",
          "hit": [
            {
              "@score": "6", "@id": "2958896",
              "info": {
                "authors": {"author": [
                  {"@pid": "10/6519", "text": "Ashish Vaswani"},
                  {"@pid": "26/4458", "text": "Noam Shazeer"},
                  {"@pid": "w/WeiWang1", "text": "Wei Wang 0001"}
                ]},
                "title": "Attention is All you Need.",
                "venue": "NIPS",
                "pages": "5998-6008",
                "year": "2017",
                "type": "Conference and Workshop Papers",
                "access": "open",
                "key": "conf/nips/VaswaniSPUJGKP17",
                "doi": "10.5555/3295222.3295349",
                "ee": "https://dl.acm.org/doi/10.5555/3295222.3295349",
                "url": "https://dblp.org/rec/conf/nips/VaswaniSPUJGKP17"
              },
              "url": "URL#2958896"
            },
            {
              "@score": "3", "@id": "1",
              "info": {
                "authors": {"author": {"@pid": "x", "text": "Single Author"}},
                "title": "A Journal Paper",
                "venue": ["J. Mach. Learn. Res.", "Another"],
                "volume": "20",
                "number": "4",
                "year": "2019",
                "type": "Journal Articles"
              }
            },
            {
              "@score": "1", "@id": "2",
              "info": {
                "title": "Books and Theses Entry",
                "publisher": "Springer",
                "isbn": "978-0-306-40615-7",
                "year": "not a year",
                "type": "Books and Theses"
              }
            },
            {"@id": "3", "info": {"title": "...", "type": "Informal Publications"}},
            {"@id": "4", "info": {"year": "2000"}},
            {"@id": "5"}
          ]
        }
      }
    }"#;

    #[test]
    fn dblp_hits_parse_in_order_with_every_field() {
        let found = parse_search(SEARCH);
        assert_eq!(
            found.len(),
            3,
            "a title of dots alone, no title, no info: skipped"
        );

        let first = &found[0];
        assert_eq!(first.source, MetaSource::Dblp);
        assert_eq!(
            first.title, "Attention is All you Need",
            "the trailing period goes"
        );
        assert_eq!(
            first.authors,
            vec![
                author("Vaswani", "Ashish"),
                author("Shazeer", "Noam"),
                author("Wang", "Wei"),
            ],
            "the namesake number goes"
        );
        assert_eq!(first.year, Some(2017));
        assert_eq!(first.venue.as_deref(), Some("NIPS"));
        assert_eq!(first.pages.as_deref(), Some("5998-6008"));
        assert_eq!(first.volume, None);
        assert_eq!(first.kind, Some(PaperKind::Inproceedings));
        assert_eq!(first.doi.as_deref(), Some("10.5555/3295222.3295349"));
        assert_eq!(first.abstract_, None);
        assert_eq!(first.arxiv, None);

        let journal = &found[1];
        assert_eq!(
            journal.authors,
            vec![author("Author", "Single")],
            "one author is an object"
        );
        assert_eq!(
            journal.venue.as_deref(),
            Some("J. Mach. Learn. Res."),
            "the first of a list"
        );
        assert_eq!(journal.volume.as_deref(), Some("20"));
        assert_eq!(journal.issue.as_deref(), Some("4"));
        assert_eq!(journal.kind, Some(PaperKind::Article));
        assert_eq!(journal.doi, None);

        let book = &found[2];
        assert!(book.authors.is_empty());
        assert_eq!(book.publisher.as_deref(), Some("Springer"));
        assert_eq!(book.isbn.as_deref(), Some("9780306406157"));
        assert_eq!(book.year, None);
        assert_eq!(book.kind, Some(PaperKind::Book));
    }

    #[test]
    fn dblp_type_mapping() {
        let kind = |t: &str| {
            parse_search(&format!(
                r#"{{"result": {{"hits": {{"hit": [{{"info": {{"title": "T", "type": "{t}"}}}}]}}}}}}"#
            ))
            .into_iter()
            .next()
            .and_then(|r| r.kind)
        };
        assert_eq!(kind("Journal Articles"), Some(PaperKind::Article));
        assert_eq!(
            kind("Conference and Workshop Papers"),
            Some(PaperKind::Inproceedings)
        );
        assert_eq!(kind("Books and Theses"), Some(PaperKind::Book));
        assert_eq!(kind("Informal Publications"), Some(PaperKind::Misc));
        let untyped = r#"{"result": {"hits": {"hit": [{"info": {"title": "T"}}]}}}"#;
        assert_eq!(parse_search(untyped)[0].kind, None);
    }

    #[test]
    fn dblp_no_hits_and_junk_yield_nothing() {
        // dblp leaves `hit` out entirely when nothing matched.
        let none = r#"{"result": {"query": "zzz", "status": {"@code": "200"}, "hits": {"@total": "0", "@sent": "0", "@first": "0"}}}"#;
        assert!(parse_search(none).is_empty());
        assert!(parse_search("").is_empty());
        assert!(parse_search(r#"{"result": {"hits": {"hit": "nope"}}}"#).is_empty());
    }

    #[test]
    fn dblp_author_names() {
        assert_eq!(
            dblp_author(&serde_json::json!({"text": "Wei Wang 0001"})),
            Some(author("Wang", "Wei"))
        );
        assert_eq!(
            dblp_author(&serde_json::json!("Jean-Luc Picard")),
            Some(author("Picard", "Jean-Luc"))
        );
        assert_eq!(
            dblp_author(&serde_json::json!({"text": "Lab 2024"})),
            Some(author("Lab", "")),
            "a four-digit tail is a namesake number, not a name"
        );
        assert_eq!(
            dblp_author(&serde_json::json!({"text": "R2 D2"})),
            Some(author("D2", "R2")),
            "two digits are a name"
        );
        assert_eq!(dblp_author(&serde_json::json!({"@pid": "x"})), None);
        assert_eq!(dblp_author(&serde_json::json!(null)), None);
    }

    #[test]
    fn dblp_refuses_when_lookup_is_off() {
        let off = LookupSettings::default();
        let client = lookup::client();
        let limiter = HostLimiter::new();
        let result =
            tauri::async_runtime::block_on(search_by_title(&client, &limiter, &off, "A title"));
        assert!(result.is_err());
    }
}
