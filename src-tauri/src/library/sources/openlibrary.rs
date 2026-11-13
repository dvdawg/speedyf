//! Open Library (projects.md §4), for books. `isbn/{isbn}.json` is the
//! edition; it names its authors only by key, so each is fetched as
//! `{key}.json` for the name, and when the edition lists no authors its
//! work (`/works/OL…W`) is read for them. What leaves the machine is the
//! ISBN and then Open Library's own keys (`/authors/OL…A`, `/works/OL…W`),
//! which name authors and works, not the user. No key, no contact.

use crate::errors::AppResult;
use crate::library::lookup::{self, LookupSettings};
use crate::library::normalize_isbn;
use crate::library::ratelimit::HostLimiter;
use crate::library::types::{Author, LookupCandidateDto, MetaSource, PaperKind};
use serde_json::Value;

const OPENLIBRARY: &str = "https://openlibrary.org";
/// Authors fetched per book at most. Each is a request, a second apart.
const MAX_AUTHORS: usize = 10;

/// An edition record as parsed: the candidate without its authors, and the
/// keys to fetch them by.
#[derive(Clone, Debug, PartialEq)]
pub struct Edition {
    pub candidate: LookupCandidateDto,
    /// `/authors/OL…A`, in the edition's order.
    pub author_keys: Vec<String>,
    /// `/works/OL…W`; consulted for authors when the edition lists none.
    pub work_keys: Vec<String>,
}

/// `isbn/{isbn}.json`, then the authors. Sends the ISBN and the keys.
/// `Ok(None)` when the ISBN does not check out, Open Library has no edition
/// (404), or the edition has no title. An author whose record is missing
/// is left out rather than failing the book.
pub async fn fetch_by_isbn(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    settings: &LookupSettings,
    isbn: &str,
) -> AppResult<Option<LookupCandidateDto>> {
    lookup::ensure_enabled(settings)?;
    let Some(isbn) = normalize_isbn(isbn) else {
        return Ok(None);
    };
    let request = client.get(format!("{OPENLIBRARY}/isbn/{isbn}.json"));
    let Some(body) = lookup::fetch_text(client, limiter, request).await? else {
        return Ok(None);
    };
    let Some(edition) = parse_edition(&body, &isbn) else {
        return Ok(None);
    };
    let mut author_keys = edition.author_keys;
    if author_keys.is_empty() {
        if let Some(work) = edition.work_keys.first() {
            if let Some(body) = fetch_record(client, limiter, work).await? {
                author_keys = parse_work_authors(&body);
            }
        }
    }
    let mut candidate = edition.candidate;
    for key in author_keys.iter().take(MAX_AUTHORS) {
        if let Some(body) = fetch_record(client, limiter, key).await? {
            if let Some(author) = parse_author(&body) {
                candidate.authors.push(author);
            }
        }
    }
    Ok(Some(candidate))
}

/// `{key}.json` for a key Open Library gave us. `None` on 404.
async fn fetch_record(
    client: &reqwest::Client,
    limiter: &HostLimiter,
    key: &str,
) -> AppResult<Option<String>> {
    let request = client.get(format!("{OPENLIBRARY}{key}.json"));
    lookup::fetch_text(client, limiter, request).await
}

/// Parses an edition record. `None` when it is not one or has no title.
/// `isbn` is the normalized ISBN asked for, recorded on the candidate.
pub fn parse_edition(json: &str, isbn: &str) -> Option<Edition> {
    let root: Value = serde_json::from_str(json).ok()?;
    if !root.is_object() {
        return None;
    }
    let title = root.get("title").and_then(lookup::text_field)?;
    let year = root
        .get("publish_date")
        .and_then(Value::as_str)
        .and_then(year_in);
    let publisher = root.get("publishers").and_then(lookup::text_field);
    let abstract_ = root
        .get("description")
        .and_then(|description| match description {
            Value::Object(_) => description.get("value").and_then(lookup::text_field),
            other => lookup::text_field(other),
        });
    let keys = |field: &str, prefix: &str| -> Vec<String> {
        root.get(field)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|entry| entry.get("key")?.as_str())
                    .map(str::trim)
                    .filter(|key| key.starts_with(prefix) && key.len() > prefix.len())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(Edition {
        candidate: LookupCandidateDto {
            source: MetaSource::OpenLibrary,
            title,
            authors: Vec::new(),
            year,
            venue: None,
            volume: None,
            issue: None,
            pages: None,
            publisher,
            abstract_,
            kind: Some(PaperKind::Book),
            doi: None,
            arxiv: None,
            isbn: Some(isbn.to_string()),
        },
        author_keys: keys("authors", "/authors/"),
        work_keys: keys("works", "/works/"),
    })
}

/// Parses an author record: `name` (else `personal_name`), split with the
/// last word as the family name. `None` without one.
pub fn parse_author(json: &str) -> Option<Author> {
    let root: Value = serde_json::from_str(json).ok()?;
    let name = ["name", "personal_name"]
        .iter()
        .find_map(|key| root.get(*key).and_then(lookup::text_field))?;
    lookup::split_name(&name)
}

/// The author keys of a work record: `authors[].author.key`.
pub fn parse_work_authors(json: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    root.get("authors")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entry| entry.pointer("/author/key")?.as_str())
                .map(str::trim)
                .filter(|key| key.starts_with("/authors/") && key.len() > "/authors/".len())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The year in a `publish_date` — `1994`, `March 2001`, `2001-03-01` — as
/// the first run of four digits.
fn year_in(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    let mut run = 0;
    for (at, byte) in bytes.iter().enumerate() {
        if byte.is_ascii_digit() {
            run += 1;
            if run == 4 && bytes.get(at + 1).is_none_or(|next| !next.is_ascii_digit()) {
                return text[at + 1 - 4..=at].parse().ok();
            }
        } else {
            run = 0;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests — no network. Fixtures are trimmed from Open Library's edition,
// work and author records.
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

    const EDITION: &str = r#"{
      "publishers": ["Addison-Wesley", "Pearson"],
      "number_of_pages": 464,
      "isbn_10": ["0201633612"],
      "isbn_13": ["9780201633610"],
      "title": "Design patterns",
      "subtitle": "elements of reusable object-oriented software",
      "publish_date": "March 1995",
      "authors": [
        {"key": "/authors/OL2653302A"},
        {"key": "/authors/OL2653303A"},
        {"key": "/books/OL1M"},
        {"key": "/authors/"},
        {"name": "no key"}
      ],
      "works": [{"key": "/works/OL1908735W"}],
      "description": {"type": "/type/text", "value": "Capturing a wealth of\n experience about the design of object-oriented software."},
      "key": "/books/OL7409710M",
      "type": {"key": "/type/edition"}
    }"#;

    #[test]
    fn edition_parses_with_keys_and_no_authors_yet() {
        let edition = parse_edition(EDITION, "9780201633610").expect("an edition");
        let book = &edition.candidate;
        assert_eq!(book.source, MetaSource::OpenLibrary);
        assert_eq!(book.title, "Design patterns");
        assert!(book.authors.is_empty());
        assert_eq!(book.year, Some(1995));
        assert_eq!(book.publisher.as_deref(), Some("Addison-Wesley"));
        assert_eq!(
            book.abstract_.as_deref(),
            Some("Capturing a wealth of experience about the design of object-oriented software.")
        );
        assert_eq!(book.kind, Some(PaperKind::Book));
        assert_eq!(book.isbn.as_deref(), Some("9780201633610"));
        assert_eq!(book.doi, None);
        assert_eq!(book.venue, None);
        assert_eq!(
            edition.author_keys,
            vec!["/authors/OL2653302A", "/authors/OL2653303A"],
            "only author keys, and only real ones"
        );
        assert_eq!(edition.work_keys, vec!["/works/OL1908735W"]);
    }

    #[test]
    fn edition_tolerates_missing_fields() {
        let json =
            r#"{"title": "Bare", "publish_date": "2001-03-01", "description": "Plain text"}"#;
        let edition = parse_edition(json, "0306406152").expect("an edition");
        assert_eq!(edition.candidate.year, Some(2001));
        assert_eq!(edition.candidate.publisher, None);
        assert_eq!(edition.candidate.abstract_.as_deref(), Some("Plain text"));
        assert!(edition.author_keys.is_empty());
        assert!(edition.work_keys.is_empty());

        assert_eq!(
            parse_edition(r#"{"key": "/books/OL1M"}"#, "x"),
            None,
            "no title"
        );
        assert_eq!(parse_edition("not json", "x"), None);
        assert_eq!(parse_edition("[]", "x"), None);
    }

    #[test]
    fn years_are_read_out_of_publish_dates() {
        assert_eq!(year_in("1994"), Some(1994));
        assert_eq!(year_in("March 2001"), Some(2001));
        assert_eq!(year_in("2001-03-01"), Some(2001));
        assert_eq!(year_in("Jan 5, 1999"), Some(1999));
        assert_eq!(year_in("19940"), None, "five digits are not a year");
        assert_eq!(year_in("c. 95"), None);
        assert_eq!(year_in(""), None);
    }

    #[test]
    fn author_records_split_on_the_last_word() {
        let gamma = r#"{"name": "Erich Gamma", "personal_name": "Gamma, Erich", "key": "/authors/OL2653302A", "type": {"key": "/type/author"}}"#;
        assert_eq!(parse_author(gamma), Some(author("Gamma", "Erich")));
        let personal = r#"{"personal_name": "Helm, Richard", "key": "/authors/OL2653303A"}"#;
        assert_eq!(parse_author(personal), Some(author("Helm", "Richard")));
        let one = r#"{"name": "Voltaire"}"#;
        assert_eq!(parse_author(one), Some(author("Voltaire", "")));
        assert_eq!(parse_author(r#"{"key": "/authors/OL1A"}"#), None);
        assert_eq!(parse_author("nope"), None);
    }

    #[test]
    fn work_records_list_author_keys() {
        let work = r#"{
          "title": "Design Patterns",
          "authors": [
            {"author": {"key": "/authors/OL2653302A"}, "type": {"key": "/type/author_role"}},
            {"author": {"key": "/authors/OL2653303A"}},
            {"author": {"key": "/works/OL9W"}},
            {"type": {"key": "/type/author_role"}}
          ],
          "key": "/works/OL1908735W"
        }"#;
        assert_eq!(
            parse_work_authors(work),
            vec!["/authors/OL2653302A", "/authors/OL2653303A"]
        );
        assert!(parse_work_authors(r#"{"title": "No authors"}"#).is_empty());
        assert!(parse_work_authors("").is_empty());
    }

    #[test]
    fn openlibrary_refuses_when_lookup_is_off_and_checks_the_isbn_first() {
        let client = lookup::client();
        let limiter = HostLimiter::new();
        fn run<F: std::future::Future>(future: F) -> F::Output {
            tauri::async_runtime::block_on(future)
        }
        let off = LookupSettings::default();
        assert!(run(fetch_by_isbn(&client, &limiter, &off, "9780201633610")).is_err());

        // On, but the ISBN fails its checksum: nothing is sent, so no error
        // from the network and no wait recorded for the host.
        let on = LookupSettings {
            enabled: true,
            ..Default::default()
        };
        let result = run(fetch_by_isbn(&client, &limiter, &on, "9780201633611"));
        assert!(matches!(result, Ok(None)));
        assert_eq!(
            limiter.wait_for("openlibrary.org"),
            std::time::Duration::ZERO
        );
    }
}
