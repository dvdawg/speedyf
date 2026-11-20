//! The scan checked against a real library: every item whose title,
//! authors or year a registry (or the user) settled is scanned again from
//! its file, offline, as a new file would be, and the scan's answer is
//! graded against the settled one. The library is the answer key; nothing
//! is written to it and nothing goes over the network.
//!
//! Run through the `metaeval` binary. The report names each miss with what
//! the scan said, what the record says and which local source said it, so
//! a heuristic in `firstpage`, `xmp` or the Info filter can be judged on
//! the papers it gets wrong.

use super::papers::{fold, scanned_record};
use super::types::{Author, MetaField, MetaSource};
use crate::engine::types::Priority;
use crate::engine::{EngineHandle, Work};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How one field of one item came out.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Grade {
    /// The same, once case, accents and punctuation are set aside.
    Exact,
    /// Most of it: a title cut short or run on, an author list with a name
    /// missed or one too many.
    Partial,
    Wrong,
    /// The scan said nothing.
    Missing,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FieldResult {
    pub grade: Grade,
    pub expected: String,
    pub scanned: Option<String>,
    /// The local source the scan's value came from.
    pub scanned_source: Option<MetaSource>,
    /// The registry (or the user) the expected value came from.
    pub truth_source: MetaSource,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub id: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<FieldResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<FieldResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<FieldResult>,
}

/// Counts of each grade for one field, overall and by the local source
/// that answered.
#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub graded: u32,
    pub grades: BTreeMap<String, u32>,
    pub by_source: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub library: String,
    pub items: u32,
    pub skipped_no_file: u32,
    pub scan_errors: Vec<String>,
    pub title: Tally,
    pub authors: Tally,
    pub year: Tally,
    pub results: Vec<ItemResult>,
}

/// What a lookup or the user said: the only values worth grading against.
/// The file's own sources are what is being graded; what citing papers
/// agree on is a heuristic of its own.
fn is_settled(source: MetaSource) -> bool {
    !matches!(
        source,
        MetaSource::Filename
            | MetaSource::FirstPage
            | MetaSource::PdfInfo
            | MetaSource::Citations
            | MetaSource::Identifier
            | MetaSource::Xmp
    )
}

/// A registry's title as plain text: Crossref writes `<sup>235</sup>U`
/// and arXiv `$\mathtt{FREYA}$`, which a page never prints.
fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    let mut plain = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // A LaTeX command name goes; its argument stays.
            while chars.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                chars.next();
            }
        } else if !matches!(c, '$' | '{' | '}') {
            plain.push(c);
        }
    }
    plain
}

/// Letters and digits only, lowercased and unaccented, words single-spaced.
fn normalize(text: &str) -> String {
    fold(&strip_markup(text))
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn grade_title(expected: &str, scanned: Option<&str>) -> Grade {
    let Some(scanned) = scanned else {
        return Grade::Missing;
    };
    let (want, got) = (normalize(expected), normalize(scanned));
    // Spacing aside too: "from239Pu" and "from ²³⁹Pu" are one title.
    if want.replace(' ', "") == got.replace(' ', "") {
        return Grade::Exact;
    }
    let want_words: Vec<&str> = want.split(' ').collect();
    let got_words: Vec<&str> = got.split(' ').collect();
    let shared = want_words.iter().filter(|w| got_words.contains(w)).count();
    let recall = shared as f32 / want_words.len().max(1) as f32;
    let precision = shared as f32 / got_words.len().max(1) as f32;
    if recall >= 0.6 && precision >= 0.6 {
        Grade::Partial
    } else {
        Grade::Wrong
    }
}

fn family_key(author: &Author) -> String {
    normalize(&author.family)
}

pub fn grade_authors(expected: &[Author], scanned: Option<&[Author]>) -> Grade {
    let Some(scanned) = scanned.filter(|s| !s.is_empty()) else {
        return Grade::Missing;
    };
    let want: Vec<String> = expected.iter().map(family_key).collect();
    let got: Vec<String> = scanned.iter().map(family_key).collect();
    if want == got {
        return Grade::Exact;
    }
    let shared = want.iter().filter(|w| got.contains(w)).count();
    let first_agrees = want.first() == got.first();
    if first_agrees && shared * 2 >= want.len() {
        Grade::Partial
    } else {
        Grade::Wrong
    }
}

pub fn grade_year(expected: i32, scanned: Option<i32>) -> Grade {
    match scanned {
        None => Grade::Missing,
        Some(year) if year == expected => Grade::Exact,
        Some(_) => Grade::Wrong,
    }
}

fn authors_text(authors: &[Author]) -> String {
    authors
        .iter()
        .map(|a| {
            if a.given.is_empty() {
                a.family.clone()
            } else {
                format!("{} {}", a.given, a.family)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn settled<T: serde::de::DeserializeOwned>(meta: &Value, key: &str) -> Option<MetaField<T>> {
    let field: MetaField<T> = serde_json::from_value(meta.get(key)?.clone()).ok()?;
    is_settled(field.source).then_some(field)
}

/// The item's first file that is still on disk.
fn file_on_disk(paper: &Value) -> Option<PathBuf> {
    paper.get("files")?.as_array()?.iter().find_map(|file| {
        let path = PathBuf::from(file.get("path")?.as_str()?);
        path.is_file().then_some(path)
    })
}

fn tally(t: &mut Tally, field: &FieldResult) {
    let grade = serde_json::to_value(field.grade)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let source = field
        .scanned_source
        .and_then(|s| serde_json::to_value(s).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "none".to_owned());
    t.graded += 1;
    *t.grades.entry(grade.clone()).or_default() += 1;
    *t.by_source
        .entry(source)
        .or_default()
        .entry(grade)
        .or_default() += 1;
}

/// Grade the scan against every settled item of `library_json`. Each file
/// is scanned with `scratch_dir` as the app data folder, so the OCR cache
/// the scan consults is an empty one, not the app's.
pub fn run(
    engine: &EngineHandle,
    library_json: &Path,
    scratch_dir: &Path,
) -> Result<Report, String> {
    let text = std::fs::read_to_string(library_json)
        .map_err(|e| format!("cannot read {}: {e}", library_json.display()))?;
    let library: Value =
        serde_json::from_str(&text).map_err(|e| format!("cannot parse library: {e}"))?;
    let papers = library
        .get("papers")
        .and_then(Value::as_array)
        .ok_or("the library has no papers")?;
    let mut report = Report {
        library: library_json.display().to_string(),
        ..Report::default()
    };
    for paper in papers {
        let Some(meta) = paper.get("meta") else {
            continue;
        };
        let title = settled::<String>(meta, "title");
        let authors = settled::<Vec<Author>>(meta, "authors").filter(|f| !f.value.is_empty());
        let year = settled::<i32>(meta, "year");
        if title.is_none() && authors.is_none() && year.is_none() {
            continue;
        }
        report.items += 1;
        let Some(path) = file_on_disk(paper) else {
            report.skipped_no_file += 1;
            continue;
        };
        let scanned = engine.call_blocking(Priority::HoverPreview, 0, |respond| {
            Work::LibraryScanEntry {
                path: path.clone(),
                app_data_dir: scratch_dir.to_path_buf(),
                respond,
            }
        });
        let entry = match scanned {
            Ok(entry) => entry,
            Err(error) => {
                report
                    .scan_errors
                    .push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        let got = scanned_record(&entry).meta;
        let mut item = ItemResult {
            id: paper
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            file: path.display().to_string(),
            title: None,
            authors: None,
            year: None,
        };
        if let Some(want) = title {
            let field = FieldResult {
                grade: grade_title(&want.value, got.title.as_ref().map(|f| f.value.as_str())),
                expected: want.value,
                scanned: got.title.as_ref().map(|f| f.value.clone()),
                scanned_source: got.title.as_ref().map(|f| f.source),
                truth_source: want.source,
            };
            tally(&mut report.title, &field);
            item.title = Some(field);
        }
        if let Some(want) = authors {
            let scanned = got.authors.as_ref().map(|f| f.value.as_slice());
            let field = FieldResult {
                grade: grade_authors(&want.value, scanned),
                expected: authors_text(&want.value),
                scanned: scanned.map(authors_text),
                scanned_source: got.authors.as_ref().map(|f| f.source),
                truth_source: want.source,
            };
            tally(&mut report.authors, &field);
            item.authors = Some(field);
        }
        if let Some(want) = year {
            let field = FieldResult {
                grade: grade_year(want.value, got.year.as_ref().map(|f| f.value)),
                expected: want.value.to_string(),
                scanned: got.year.as_ref().map(|f| f.value.to_string()),
                scanned_source: got.year.as_ref().map(|f| f.source),
                truth_source: want.source,
            };
            tally(&mut report.year, &field);
            item.year = Some(field);
        }
        report.results.push(item);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(given: &str, family: &str) -> Author {
        Author {
            family: family.into(),
            given: given.into(),
        }
    }

    #[test]
    fn titles_differing_only_in_case_accents_and_punctuation_are_exact() {
        assert_eq!(
            grade_title(
                "Arthur-Merlin games: a randomized proof system",
                Some("ARTHUR MERLIN GAMES. A Randomized Proof System")
            ),
            Grade::Exact
        );
        assert_eq!(
            grade_title("Théorie des nombres", Some("Theorie des Nombres")),
            Grade::Exact
        );
    }

    #[test]
    fn a_title_cut_short_is_partial_and_a_banner_is_wrong() {
        let want = "A composite nuclear level density formula with shell corrections";
        assert_eq!(
            grade_title(want, Some("A composite nuclear level density formula")),
            Grade::Partial
        );
        assert_eq!(
            grade_title(want, Some("Canadian Journal of Physics")),
            Grade::Wrong
        );
        assert_eq!(grade_title(want, None), Grade::Missing);
    }

    #[test]
    fn authors_are_graded_by_family_name_in_order() {
        let want = [author("László", "Babai"), author("Shlomo", "Moran")];
        assert_eq!(
            grade_authors(&want, Some(&[author("L.", "Babai"), author("S.", "Moran")])),
            Grade::Exact
        );
        assert_eq!(
            grade_authors(&want, Some(&[author("L.", "Babai")])),
            Grade::Partial
        );
        assert_eq!(
            grade_authors(&want, Some(&[author("S.", "Moran"), author("L.", "Babai")])),
            Grade::Wrong
        );
        assert_eq!(grade_authors(&want, Some(&[])), Grade::Missing);
    }

    #[test]
    fn only_registry_and_user_values_are_an_answer_key() {
        assert!(is_settled(MetaSource::Crossref));
        assert!(is_settled(MetaSource::User));
        assert!(!is_settled(MetaSource::FirstPage));
        assert!(!is_settled(MetaSource::Xmp));
        assert!(!is_settled(MetaSource::Citations));
    }
}
