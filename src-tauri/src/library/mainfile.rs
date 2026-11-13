//! Which of an item's files is its main one - the one opened, previewed and
//! read for fields - and which files are byte-identical copies of one
//! another (projects.md sec. 2).
//!
//! The main file is the user's choice when they made one
//! (`PaperRecord::primary_path`). Otherwise it is chosen by narrowing the
//! files rule by rule, each rule applied only when some file passes it:
//!
//! 0. on disk over missing;
//! 1. the published version: a file whose own DOI (first page or XMP) is
//!    the item's journal DOI, or whose classifier reason names a publisher;
//! 2. the highest arXiv version: a file known to be an older version than
//!    another loses (a file whose version is unknown is not ruled out);
//! 3. authors on the first page over none (an anonymous review copy loses);
//! 4. not a browser duplicate (`name (1).pdf`, `copy of name.pdf`);
//! 5. the newest modification time; a tie keeps the current order.

use super::identifiers;
use super::papers::normalize_doi;
use super::types::{FileTraits, MetaSource, PaperFile};
use super::LibraryEntry;
use std::collections::HashMap;

/// What the scan's entry says about the file, for ranking it later.
pub fn traits_of(entry: &LibraryEntry) -> FileTraits {
    let mut dois: Vec<String> = Vec::new();
    // The page's DOI (a PII the scan converted included); not the file
    // name's, which says what the file was saved as, not what it is.
    let page = entry
        .doi
        .as_deref()
        .filter(|_| entry.doi_source != Some(MetaSource::Filename));
    let packet = entry.xmp.as_ref().and_then(|xmp| {
        xmp.doi.clone().or_else(|| {
            xmp.pii
                .as_deref()
                .filter(|pii| identifiers::pii_has_valid_issn(pii))
                .map(identifiers::doi_from_pii)
        })
    });
    for doi in page
        .and_then(normalize_doi)
        .into_iter()
        .chain(packet.as_deref().and_then(normalize_doi))
    {
        if !dois.contains(&doi) {
            dois.push(doi);
        }
    }
    let name = entry
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    FileTraits {
        dois,
        publisher: entry.paper_reason.as_deref().is_some_and(names_publisher),
        // The scan's reading (the stamp, or an arXiv-id file name), else
        // whatever version the name gives ("paper_v3.pdf").
        version: entry.arxiv_version.or_else(|| version_from_name(&name)),
        authors: !entry.first_page_authors.is_empty(),
    }
}

/// Whether a classifier reason (`paperness::classify`) names a publisher:
/// "Elsevier on the first page", "an APS journal on the first page",
/// "Elsevier PII ... on the first page", "made by IEEE", "an edition from
/// Cambridge University Press". Not the arXiv stamp, a bare DOI, or a
/// file made by arXiv.
pub fn names_publisher(reason: &str) -> bool {
    let reason = reason.trim();
    if let Some(name) = reason.strip_prefix("made by ") {
        return !name.eq_ignore_ascii_case("arxiv");
    }
    if reason.starts_with("an edition from ") || reason.starts_with("a copyright page from ") {
        return true;
    }
    match reason.strip_suffix(" on the first page") {
        Some(what) => !(what.starts_with("arXiv:") || what.starts_with("DOI ")),
        None => false,
    }
}

/// The version a file name gives: `v` and one or two digits after a digit
/// (`2606.13614v2`) or a separator (`paper_v3`, `paper-v2`), not followed
/// by another letter or digit. `None` when the name says nothing.
pub fn version_from_name(name: &str) -> Option<u32> {
    let stem = name
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
        .map_or(name, |(stem, _)| stem);
    let chars: Vec<char> = stem.chars().collect();
    let mut found = None;
    for at in 1..chars.len() {
        if !matches!(chars[at], 'v' | 'V') {
            continue;
        }
        let before = chars[at - 1];
        if !(before.is_ascii_digit() || matches!(before, '_' | '-' | ' ' | '.')) {
            continue;
        }
        let digits: String = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let after = chars.get(at + 1 + digits.len());
        if digits.is_empty() || digits.len() > 2 || after.is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        found = digits.parse::<u32>().ok().filter(|v| *v > 0).or(found);
    }
    found
}

/// A name a browser or the Finder gives a second download or a copy:
/// `name (1).pdf`, `name (2).pdf`, `Copy of name.pdf`, `name copy.pdf`,
/// `name copy 2.pdf`.
pub fn is_browser_duplicate(name: &str) -> bool {
    let lower = name.to_lowercase();
    let stem = lower
        .strip_suffix(".pdf")
        .unwrap_or(&lower)
        .trim_end()
        .to_owned();
    if stem.starts_with("copy of ") {
        return true;
    }
    let numbered = |rest: &str| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    if let Some(open) = stem.rfind(" (") {
        if let Some(inner) = stem[open + 2..].strip_suffix(')') {
            if numbered(inner) {
                return true;
            }
        }
    }
    if stem.ends_with(" copy") {
        return true;
    }
    stem.rsplit_once(" copy ")
        .is_some_and(|(_, number)| numbered(number))
}

fn published(file: &PaperFile, journal_doi: Option<&str>) -> bool {
    file.traits.publisher
        || journal_doi.is_some_and(|doi| file.traits.dois.iter().any(|own| own == doi))
}

/// Keep only the files that pass `rule`, when any does.
fn narrow(pool: &mut Vec<usize>, rule: impl Fn(usize) -> bool) {
    if pool.iter().any(|&at| rule(at)) {
        pool.retain(|&at| rule(at));
    }
}

/// The index of the file the rules make the main one, `None` for no files.
/// `journal_doi` is the item's DOI unless it is an arXiv DOI (10.48550).
pub fn main_index(files: &[PaperFile], journal_doi: Option<&str>) -> Option<usize> {
    let mut pool: Vec<usize> = (0..files.len()).collect();
    narrow(&mut pool, |at| !files[at].missing);
    narrow(&mut pool, |at| published(&files[at], journal_doi));
    if let Some(newest) = pool.iter().filter_map(|&at| files[at].traits.version).max() {
        pool.retain(|&at| files[at].traits.version.is_none_or(|v| v == newest));
    }
    narrow(&mut pool, |at| files[at].traits.authors);
    narrow(&mut pool, |at| !is_browser_duplicate(&files[at].name));
    let newest = pool.iter().map(|&at| files[at].mtime_ms).max()?;
    pool.into_iter().find(|&at| files[at].mtime_ms == newest)
}

/// For each file, the path of the copy that stands for it in the UI when
/// it is a byte-identical copy (same fingerprint, both on disk) of another
/// file of the item, else `None`. A group's representative is the main
/// file when it is in the group, else the copy that is not a browser
/// duplicate, else the oldest.
pub fn duplicate_of(files: &[PaperFile]) -> Vec<Option<String>> {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (at, file) in files.iter().enumerate() {
        if let (false, Some(fingerprint)) = (file.missing, file.fingerprint.as_deref()) {
            groups.entry(fingerprint).or_default().push(at);
        }
    }
    let mut out = vec![None; files.len()];
    for members in groups.values().filter(|members| members.len() > 1) {
        let representative = if members.contains(&0) {
            0
        } else {
            *members
                .iter()
                .min_by_key(|&&at| {
                    (
                        is_browser_duplicate(&files[at].name),
                        files[at].mtime_ms,
                        at,
                    )
                })
                .expect("a group has members")
        };
        for &at in members {
            if at != representative {
                out[at] = Some(files[representative].path.clone());
            }
        }
    }
    out
}

/// Files on disk, a byte-identical copy counted once.
pub fn distinct_present(files: &[PaperFile]) -> usize {
    let copies = duplicate_of(files);
    files
        .iter()
        .zip(copies)
        .filter(|(file, copy)| !file.missing && copy.is_none())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, mtime_ms: u64) -> PaperFile {
        PaperFile {
            path: format!("/l/{name}"),
            name: name.to_owned(),
            size: 1,
            mtime_ms,
            fingerprint: Some(format!("fp-{name}")),
            ..Default::default()
        }
    }

    fn main_name(files: &[PaperFile], doi: Option<&str>) -> String {
        files[main_index(files, doi).unwrap()].name.clone()
    }

    #[test]
    fn publisher_reasons_are_told_from_the_rest() {
        for yes in [
            "Elsevier on the first page",
            "an APS journal on the first page",
            "Elsevier PII S0022-0000(88)90028-1 on the first page",
            "made by IEEE",
            "made by a journal typesetter",
            "an edition from Cambridge University Press",
            "a copyright page from MIT Press",
        ] {
            assert!(names_publisher(yes), "{yes}");
        }
        for no in [
            "arXiv:2401.00001 on the first page",
            "DOI 10.1/x on the first page",
            "made by arXiv",
            "has an abstract and the sections of a paper",
            "a thesis title page",
            "",
        ] {
            assert!(!names_publisher(no), "{no}");
        }
    }

    #[test]
    fn versions_and_browser_duplicates_are_read_from_names() {
        assert_eq!(version_from_name("2606.13614v1.pdf"), Some(1));
        assert_eq!(version_from_name("2606.13614v12 (1).pdf"), Some(12));
        assert_eq!(version_from_name("paper_v3.pdf"), Some(3));
        assert_eq!(version_from_name("paper-V2.PDF"), Some(2));
        for none in [
            "mot.pdf",
            "conv2d.pdf",
            "review2.pdf",
            "v2.pdf",
            "paper_v2b.pdf",
        ] {
            assert_eq!(version_from_name(none), None, "{none}");
        }
        for dup in [
            "mot (1).pdf",
            "2606.13614v1 (2).pdf",
            "Copy of mot.pdf",
            "mot copy.pdf",
            "mot copy 2.pdf",
        ] {
            assert!(is_browser_duplicate(dup), "{dup}");
        }
        for not in [
            "mot.pdf",
            "copyright.pdf",
            "mot (draft).pdf",
            "photocopy.pdf",
        ] {
            assert!(!is_browser_duplicate(not), "{not}");
        }
    }

    #[test]
    fn the_published_version_wins_by_its_doi_or_its_publisher() {
        let mut preprint = file("pre.pdf", 9);
        preprint.traits.authors = true;
        preprint.traits.version = Some(4);
        let mut journal = file("jour.pdf", 1);
        journal.traits.dois = vec!["10.1103/x".into()];
        assert_eq!(
            main_name(&[preprint.clone(), journal.clone()], Some("10.1103/x")),
            "jour.pdf"
        );
        // Without the item's DOI to match, the DOI says nothing...
        assert_eq!(
            main_name(&[preprint.clone(), journal.clone()], None),
            "pre.pdf"
        );
        // ...but a publisher's name on the page does.
        journal.traits.dois.clear();
        journal.traits.publisher = true;
        assert_eq!(main_name(&[preprint, journal], None), "jour.pdf");
    }

    #[test]
    fn a_later_arxiv_version_wins_and_an_unknown_one_is_not_ruled_out() {
        let mut v1 = file("2401.00001v1.pdf", 9);
        v1.traits.version = Some(1);
        let mut v3 = file("2401.00001v3.pdf", 1);
        v3.traits.version = Some(3);
        assert_eq!(
            main_name(&[v1.clone(), v3.clone()], None),
            "2401.00001v3.pdf"
        );
        let unknown = file("paper.pdf", 5);
        assert_eq!(
            main_name(&[v1, unknown.clone(), v3], None),
            "paper.pdf",
            "the unknown version stays in, and is newer than v3"
        );
    }

    #[test]
    fn authors_on_the_first_page_beat_an_anonymous_copy() {
        let anonymous = file("review.pdf", 9);
        let mut named = file("camera.pdf", 1);
        named.traits.authors = true;
        assert_eq!(main_name(&[anonymous, named], None), "camera.pdf");
    }

    #[test]
    fn a_browser_duplicate_loses_then_the_newest_wins_and_ties_keep_order() {
        let dup = file("a (1).pdf", 9);
        let original = file("a.pdf", 1);
        assert_eq!(main_name(&[dup, original], None), "a.pdf");
        let old = file("old.pdf", 1);
        let new = file("new.pdf", 2);
        assert_eq!(main_name(&[old.clone(), new], None), "new.pdf");
        let twin = file("twin.pdf", 1);
        assert_eq!(main_name(&[old.clone(), twin.clone()], None), "old.pdf");
        assert_eq!(main_name(&[twin, old], None), "twin.pdf");
        // A missing file is never the main one while another is on disk.
        let mut gone = file("gone.pdf", 99);
        gone.missing = true;
        assert_eq!(main_name(&[gone, file("here.pdf", 1)], None), "here.pdf");
        assert_eq!(main_index(&[], None), None);
    }

    #[test]
    fn byte_identical_copies_group_under_one_representative() {
        let mut files = vec![
            file("other.pdf", 1),
            file("mot (1).pdf", 5),
            file("mot.pdf", 7),
            file("x (2).pdf", 3),
            file("x (1).pdf", 2),
        ];
        files[1].fingerprint = Some("same".into());
        files[2].fingerprint = Some("same".into());
        files[3].fingerprint = Some("xx".into());
        files[4].fingerprint = Some("xx".into());
        let copies = duplicate_of(&files);
        assert_eq!(copies[0], None);
        assert_eq!(
            copies[1].as_deref(),
            Some("/l/mot.pdf"),
            "not the browser copy"
        );
        assert_eq!(copies[2], None);
        assert_eq!(copies[4], None, "both are duplicates: the oldest");
        assert_eq!(copies[3].as_deref(), Some("/l/x (1).pdf"));
        assert_eq!(distinct_present(&files), 3);
        // The main file stands for its group whatever its name.
        files.swap(0, 1);
        let copies = duplicate_of(&files);
        assert_eq!(copies[0], None);
        assert_eq!(copies[2].as_deref(), Some("/l/mot (1).pdf"));
        // A missing copy is not grouped.
        files[2].missing = true;
        assert_eq!(duplicate_of(&files)[2], None);
        assert_eq!(distinct_present(&files), 3);
    }
}
