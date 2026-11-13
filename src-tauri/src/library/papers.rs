//! Paper records: one per work, with its files, identifiers, metadata with
//! per-field provenance, and a cite key.
//!
//! Three rules live here and nowhere else (projects.md sec. 2, sec. 12):
//!
//! - **Identity.** A scanned file finds its paper by identifier, then by
//!   content fingerprint, then by path. No match makes a new paper. A paper
//!   id is random and permanent, never derived from either.
//! - **Provenance.** A value never loses to a weaker source, and a user's
//!   value never loses at all. That is `offer`, and every write of metadata
//!   that is not a user edit goes through it.
//! - **Cite keys** follow the metadata until pinned, and are unique across
//!   the library.

use super::chain::{ChainKind, PARK_LIMIT};
use super::consensus::CitationFacts;
use super::lookup::NetworkEdges;
use super::references::{DocRefs, StoredRef, TitleIndex};
use super::store::{format_rfc3339, now_rfc3339, parse_rfc3339, PaperStore};
use super::types::{
    Author, CitationLinksDto, GraphDto, GraphEdgeDto, LibraryCountsDto, LookupCandidateDto,
    MetaField, MetaPatchDto, MetaSource, PaperDto, PaperFile, PaperFilterDto, PaperListDto,
    PaperMeta, PaperRowDto, PapernessVerdict,
};
use super::{bibtex, citekey, identifiers, mainfile, titlematch};
use super::{normalize_isbn, plausible_info_title, LibraryEntry};
use crate::engine::citation::parse_citation_id;
use crate::engine::types::CitationIdDto;
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaperIds {
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub arxiv: Option<String>,
    /// For books. Not an identity: two chapters of one book share it.
    #[serde(default)]
    pub isbn: Option<String>,
    /// Where each identifier came from: the scan's own (`FirstPage`,
    /// `Filename`, `Identifier`, `Xmp`), a lookup's, or the user's. The
    /// scan's are the primary file's to re-derive on every rescan; a
    /// lookup's or the user's persist (`offer_identifier`). `None` on a
    /// record written before the sources were kept - unknown, and read as
    /// the scan's, so a rescan can put it right.
    #[serde(default)]
    pub doi_source: Option<MetaSource>,
    #[serde(default)]
    pub arxiv_source: Option<MetaSource>,
    #[serde(default)]
    pub isbn_source: Option<MetaSource>,
}

/// The on-disk and in-memory record. Field names are those of projects.md
/// sec. 3; the DTOs in `types.rs` are built from it, never serialized from it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PaperRecord {
    pub id: String,
    #[serde(default)]
    pub files: Vec<PaperFile>,
    #[serde(default)]
    pub ids: PaperIds,
    #[serde(default)]
    pub meta: PaperMeta,
    pub cite_key: String,
    #[serde(default)]
    pub cite_key_pinned: bool,
    pub added_at: String,
    pub updated_at: String,
    /// The scan's verdict on the primary file, and why. `None` until a
    /// scan that classifies has reached the file.
    #[serde(default)]
    pub verdict: Option<PapernessVerdict>,
    #[serde(default)]
    pub verdict_reason: String,
    /// The user's say on whether this is a paper. Beats the verdict.
    #[serde(default)]
    pub paper_override: Option<bool>,
    /// Title-search matches parked for the user to pick from. Cleared when
    /// one is applied or they are discarded.
    #[serde(default)]
    pub candidates: Vec<LookupCandidateDto>,
    /// When a lookup chain last ran for this paper (RFC 3339), whatever it
    /// found. `None` until one has. A batch leaves a paper looked up within
    /// [`LOOKUP_FRESH`] alone unless it is still incomplete.
    #[serde(default)]
    pub looked_up_at: Option<String>,
    /// Title-search matches the user declined (`titlematch::identity`), so
    /// a later search does not park the same work again.
    #[serde(default)]
    pub declined: Vec<String>,
    /// The primary category off a file's arXiv stamp (`cs.CL`, `hep-th`).
    /// Like the identifiers, the first one seen stays: it tells the lookup
    /// chain whether to ask INSPIRE or dblp before arXiv has been asked.
    #[serde(default)]
    pub arxiv_category: Option<String>,
    /// Ids of records the user said are not duplicates of this one, so
    /// `duplicates` stops pairing them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_duplicates: Vec<String>,
    /// Every record merged into this one, whole as it was, for unmerge.
    /// Flat: a merged record's own snapshots move up beside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_from: Vec<PaperRecord>,
    /// Identifiers of merged records that conflicted with this one's own
    /// (the preprint's arXiv id beside the journal DOI). Citations and
    /// scanned files carrying one resolve to this record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alias_ids: Vec<CitationIdDto>,
    /// Pinned keys of merged records this one did not adopt. Still taken,
    /// so no other paper is given one a manuscript may cite; BibTeX output
    /// uses `cite_key` only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alias_keys: Vec<String>,
    /// On a snapshot in `merged_from` only: how the merge changed things,
    /// for unmerge to put back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_trace: Option<MergeTrace>,
    /// Why records were merged into this one automatically ("same arXiv
    /// id 2606.13614"), one entry per distinct reason (`automerge`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_merge_reasons: Vec<String>,
    /// The file the user made the main one (`set_primary_file`). A rescan
    /// that reattaches files keeps it first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_path: Option<String>,
    /// What the library's citation graph says about this record
    /// (`consensus`): which citing items vouched for each field taken
    /// from them, and the year its own references put it at or after.
    #[serde(default, skip_serializing_if = "CitationFacts::is_empty")]
    pub citation_facts: CitationFacts,
}

/// What a merge did, kept on the merged record's snapshot.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeTrace {
    /// The record it was merged into.
    pub into: String,
    /// The projects the merged record was in.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Projects the target joined because of this merge.
    #[serde(default)]
    pub added_target_to: Vec<String>,
    /// The target as it was just before this merge, for the parts unmerge
    /// restores (metadata and identifiers the user has not since edited,
    /// aliases, and the key).
    pub target_meta: PaperMeta,
    pub target_ids: PaperIds,
    pub target_cite_key: String,
    pub target_cite_key_pinned: bool,
    #[serde(default)]
    pub target_alias_ids: Vec<CitationIdDto>,
    #[serde(default)]
    pub target_alias_keys: Vec<String>,
    /// The merge was automatic (`automerge`): splitting it back says the
    /// records are not the same work.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

/// How long every file of an item must have been missing before the item
/// is removed (`PaperStore::remove_long_missing`).
pub const MISSING_GRACE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Start or stop a file's missing clock to match its `missing` flag.
/// Returns whether it changed.
fn stamp_missing_since(file: &mut PaperFile, now: &str) -> bool {
    match (file.missing, file.missing_since.is_some()) {
        (true, false) => {
            file.missing_since = Some(now.to_owned());
            true
        }
        (false, true) => {
            file.missing_since = None;
            true
        }
        _ => false,
    }
}

/// Whether `path` is on a volume under `/Volumes` that is not mounted now.
pub(crate) fn on_unmounted_volume(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/Volumes/") else {
        return false;
    };
    let volume = rest.split('/').next().unwrap_or_default();
    !volume.is_empty() && !Path::new("/Volumes").join(volume).is_dir()
}

/// How long a lookup stands before a batch asks about the paper again.
pub const LOOKUP_FRESH: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Set `field` to `value` from `source` unless a stronger source already
/// filled it. Equal strength replaces, so a rescan of a changed file updates
/// a heuristic value; a user value is only ever replaced by the user.
/// Returns whether the field changed.
/// Drops a value that came from `source` when `gone` says the file no longer
/// supplies it. Pairs with `offer`: together they make file-derived fields a
/// function of the file as it is now.
fn retract<T>(field: &mut Option<MetaField<T>>, source: MetaSource, gone: bool) -> bool {
    if gone
        && field
            .as_ref()
            .is_some_and(|existing| existing.source == source)
    {
        *field = None;
        return true;
    }
    false
}

pub fn offer<T: PartialEq>(field: &mut Option<MetaField<T>>, value: T, source: MetaSource) -> bool {
    if let Some(existing) = field {
        if existing.source == MetaSource::User && source != MetaSource::User {
            return false;
        }
        if source.rank() < existing.source.rank() {
            return false;
        }
        if existing.value == value && existing.source == source {
            return false;
        }
    }
    *field = Some(MetaField { value, source });
    true
}

/// Whether an identifier with this source is the scan's to re-derive:
/// read off the file - the page, the file name, a PII, the XMP packet -
/// or of unknown origin, a record from before sources were kept, which is
/// read as the scan's so a rescan can correct it. A lookup's or the
/// user's is not.
fn scan_owned(source: Option<MetaSource>) -> bool {
    source.is_none_or(|source| source.rank() < MetaSource::OpenLibrary.rank())
}

/// One identifier as the scan of a file offers it, with its source. A
/// value the record lacks is filled from any file. What the record holds
/// is re-derived from the primary file only, and only when it is the
/// scan's own: replaced when the file now says something else, cleared
/// when the file no longer carries one and is the record's only present
/// file (`lone`) - the journal version beside the preprint may be what
/// supplied it. A lookup's or the user's identifier is never touched by a
/// scan. Returns whether the record changed.
fn offer_identifier(
    held: &mut Option<String>,
    held_source: &mut Option<MetaSource>,
    offered: Option<(String, MetaSource)>,
    primary: bool,
    lone: bool,
) -> bool {
    if held.is_none() {
        return match offered {
            Some((value, source)) => {
                *held = Some(value);
                *held_source = Some(source);
                true
            }
            None => false,
        };
    }
    if !primary || !scan_owned(*held_source) {
        return false;
    }
    match offered {
        Some((value, source)) => {
            if held.as_ref() == Some(&value) && *held_source == Some(source) {
                return false;
            }
            *held = Some(value);
            *held_source = Some(source);
            true
        }
        None if lone => {
            *held = None;
            *held_source = None;
            true
        }
        None => false,
    }
}

/// A DOI as the library stores it: lower-case, without a `doi:` or URL
/// prefix. Anything the parser does not recognize is kept lower-cased and
/// trimmed rather than dropped, since the user typed it on purpose.
pub fn normalize_doi(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match parse_citation_id(trimmed) {
        Some(CitationIdDto::Doi(value)) => Some(value),
        _ => Some(trimmed.to_ascii_lowercase()),
    }
}

/// An arXiv id without its version suffix, so `2401.12345v2` and
/// `2401.12345v3` are one work.
pub fn normalize_arxiv(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(CitationIdDto::ArXiv(value)) = parse_citation_id(trimmed) {
        return Some(value);
    }
    if let Some(CitationIdDto::ArXiv(value)) = parse_citation_id(&format!("arXiv:{trimmed}")) {
        return Some(value);
    }
    let stripped = match trimmed.rfind(['v', 'V']) {
        Some(at)
            if at > 0
                && trimmed[at + 1..].chars().all(|c| c.is_ascii_digit())
                && !trimmed[at + 1..].is_empty() =>
        {
            &trimmed[..at]
        }
        _ => trimmed,
    };
    Some(stripped.to_owned())
}

/// Authors from an Info-dictionary `Author` string. Publishers write
/// "A. Author, B. Author and C. Author", "Author A; Author B" or one name;
/// the pieces are split on `;`, ` and ` and `,`, and a piece that is only
/// initials is the given name of the piece before it, so
/// "Jarzynski, C., Crooks, G." still comes out as two people.
pub fn parse_info_authors(raw: &str) -> Vec<Author> {
    const JUNK: &[&str] = &[
        "unknown",
        "anonymous",
        "administrator",
        "admin",
        "user",
        "author",
        "owner",
    ];
    const MAX_AUTHORS: usize = 50;
    let raw = raw.trim();
    if raw.is_empty()
        || !raw.chars().any(char::is_alphabetic)
        || JUNK.contains(&raw.to_lowercase().as_str())
    {
        return Vec::new();
    }
    let mut authors: Vec<Author> = Vec::new();
    for chunk in raw
        .split(';')
        .flat_map(|chunk| chunk.split(" and "))
        .flat_map(|chunk| chunk.split(" & "))
    {
        for piece in chunk.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if is_initials(piece) {
                if let Some(last) = authors.last_mut().filter(|a| a.given.is_empty()) {
                    last.given = piece.to_owned();
                    continue;
                }
            }
            if let Some(author) = author_from_name(piece) {
                authors.push(author);
            }
            if authors.len() >= MAX_AUTHORS {
                return authors;
            }
        }
    }
    authors
}

fn is_initials(piece: &str) -> bool {
    piece.split_whitespace().all(|word| {
        let letters = word.trim_end_matches('.');
        !letters.is_empty()
            && letters.chars().count() <= 2
            && letters.chars().all(|c| c.is_uppercase())
    })
}

/// "Given Family" -> family is the last word. Good for the two forms Info
/// strings actually use; "van der Waals" becomes family "Waals", which the
/// cite key would have reduced to anyway.
fn author_from_name(name: &str) -> Option<Author> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let (family, given) = words.split_last()?;
    if !family.chars().any(char::is_alphabetic) {
        return None;
    }
    Some(Author {
        family: (*family).to_owned(),
        given: given.join(" "),
    })
}

/// Authors from XMP `dc:creator` entries. Each entry is one person, as
/// "Given Family" or "Family, Given"; a packet that crams several into one
/// entry ("A and B", "A; B") is read like an Info-dictionary Author string.
pub fn parse_xmp_creators(creators: &[String]) -> Vec<Author> {
    let mut authors = Vec::new();
    for creator in creators {
        let creator = creator.trim();
        if creator.contains(';') || creator.contains(" and ") || creator.contains(" & ") {
            authors.extend(parse_info_authors(creator));
            continue;
        }
        match creator.split_once(',') {
            // "Family, Given" - unless the comma separates two people,
            // which is the Info string's problem to sort out.
            Some((family, given)) if !given.contains(',') => {
                let (family, given) = (family.trim(), given.trim());
                if family.chars().any(char::is_alphabetic) {
                    authors.push(Author {
                        family: family.to_owned(),
                        given: given.to_owned(),
                    });
                }
            }
            Some(_) => authors.extend(parse_info_authors(creator)),
            None => authors.extend(parse_info_authors(creator)),
        }
    }
    authors
}

impl PaperRecord {
    pub fn new() -> Self {
        let now = now_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            files: Vec::new(),
            ids: PaperIds::default(),
            meta: PaperMeta::default(),
            cite_key: String::new(),
            cite_key_pinned: false,
            added_at: now.clone(),
            updated_at: now,
            verdict: None,
            verdict_reason: String::new(),
            paper_override: None,
            candidates: Vec::new(),
            looked_up_at: None,
            declined: Vec::new(),
            arxiv_category: None,
            not_duplicates: Vec::new(),
            merged_from: Vec::new(),
            alias_ids: Vec::new(),
            alias_keys: Vec::new(),
            merge_trace: None,
            auto_merge_reasons: Vec::new(),
            primary_path: None,
            citation_facts: CitationFacts::default(),
        }
    }

    fn touch(&mut self) {
        self.updated_at = now_rfc3339();
    }

    /// The DOI and arXiv id this record answers to: its own, then its
    /// aliases (from merged records), each once.
    pub fn citation_ids(&self) -> Vec<CitationIdDto> {
        let mut ids: Vec<CitationIdDto> = [
            self.ids.doi.clone().map(CitationIdDto::Doi),
            self.ids.arxiv.clone().map(CitationIdDto::ArXiv),
        ]
        .into_iter()
        .flatten()
        .collect();
        for alias in &self.alias_ids {
            if !ids.contains(alias) {
                ids.push(alias.clone());
            }
        }
        ids
    }

    /// Whether `id` is one of this record's aliases.
    fn has_alias(&self, id: &CitationIdDto) -> bool {
        self.alias_ids.contains(id)
    }

    pub fn has_identifier(&self) -> bool {
        self.ids.doi.is_some() || self.ids.arxiv.is_some()
    }

    /// Whether this record is a paper: the user's word when they gave it,
    /// else the scan's verdict. A record no classifying scan has reached is
    /// given the benefit of the doubt, so nothing vanishes from the list
    /// during the rescan that fills the verdicts in.
    pub fn is_paper(&self) -> bool {
        match (self.paper_override, self.verdict) {
            (Some(answer), _) => answer,
            (None, Some(verdict)) => {
                matches!(verdict, PapernessVerdict::Paper | PapernessVerdict::Likely)
            }
            (None, None) => true,
        }
    }

    /// Why `is_paper` says what it says, for the panel. Empty when the
    /// scan had nothing to say (or has not said yet).
    pub fn paper_reason(&self) -> String {
        match self.paper_override {
            Some(true) => "marked by you".to_owned(),
            Some(false) => "marked as not a paper by you".to_owned(),
            None => self.verdict_reason.clone(),
        }
    }

    /// Every file gone - or no file at all, which the UI treats the same.
    pub fn is_missing(&self) -> bool {
        self.files.iter().all(|file| file.missing)
    }

    pub fn title(&self) -> Option<&str> {
        self.meta
            .title
            .as_ref()
            .map(|field| field.value.as_str())
            .filter(|title| !title.trim().is_empty())
    }

    pub fn authors(&self) -> &[Author] {
        self.meta
            .authors
            .as_ref()
            .map(|field| field.value.as_slice())
            .unwrap_or(&[])
    }

    pub fn year(&self) -> Option<i32> {
        self.meta.year.as_ref().map(|field| field.value)
    }

    pub fn venue(&self) -> Option<&str> {
        self.meta
            .venue
            .as_ref()
            .map(|field| field.value.as_str())
            .filter(|venue| !venue.trim().is_empty())
    }

    /// The title, or the first file's name when there is none.
    pub fn display_title(&self) -> String {
        self.title()
            .map(str::to_owned)
            .or_else(|| self.files.first().map(|file| file.name.clone()))
            .unwrap_or_default()
    }

    /// "Jarzynski", "Evans & Cohen", "Evans, Cohen & Morriss",
    /// "Marchetti et al." - or "".
    pub fn authors_display(&self) -> String {
        let families: Vec<&str> = self
            .authors()
            .iter()
            .map(|author| author.family.trim())
            .filter(|family| !family.is_empty())
            .collect();
        match families.as_slice() {
            [] => String::new(),
            [one] => (*one).to_owned(),
            [a, b] => format!("{a} & {b}"),
            [a, b, c] => format!("{a}, {b} & {c}"),
            [first, ..] => format!("{first} et al."),
        }
    }

    /// What a BibTeX entry needs and this paper lacks. A venue is not asked
    /// of an arXiv-only work: it goes out as `@misc` with an eprint.
    pub fn incomplete(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.authors().is_empty() {
            missing.push("authors".to_owned());
        }
        if self.year().is_none() {
            missing.push("year".to_owned());
        }
        if self.title().is_none() {
            missing.push("title".to_owned());
        }
        if self.venue().is_none() && self.ids.arxiv.is_none() {
            missing.push("venue".to_owned());
        }
        missing
    }

    /// Whether an identifier lookup has something to add: an author or a
    /// year missing, or a title no lookup has confirmed - one a heuristic
    /// guessed, which is what a running header looks like. A paper a lookup
    /// already filled is left alone by a project-wide one, so the batch
    /// costs what the gaps cost.
    pub fn wants_lookup(&self) -> bool {
        if !self.has_lookup_identifier() {
            return false;
        }
        if self.authors().is_empty() || self.year().is_none() {
            return true;
        }
        self.meta
            .title
            .as_ref()
            .is_none_or(|title| title.source.rank() < MetaSource::Openalex.rank())
    }

    /// Something the identifier chain can start from: a DOI, an arXiv id,
    /// or for a book an ISBN.
    pub fn has_lookup_identifier(&self) -> bool {
        self.has_identifier() || self.ids.isbn.is_some()
    }

    /// Whether the last lookup is old enough not to count: none yet, or
    /// more than [`LOOKUP_FRESH`] before `now` (unix seconds). A stamp the
    /// parser cannot read, or one from the future, is stale too.
    pub fn lookup_stale(&self, now: u64) -> bool {
        match self.looked_up_at.as_deref().and_then(parse_rfc3339) {
            Some(at) => at > now || now - at >= LOOKUP_FRESH.as_secs(),
            None => true,
        }
    }

    /// Whether a batch looks this paper up now, and by which chain. Never
    /// a record that is not a paper. With an identifier: the identifier
    /// chain when it has a gap a lookup could fill (`wants_lookup`) or has
    /// not been looked up lately. Without: the title chain when it has a
    /// title to search for, nothing parked waiting on the user, and is
    /// incomplete or has not been looked up lately.
    pub fn batch_chain(&self, now: u64) -> Option<ChainKind> {
        if !self.is_paper() {
            return None;
        }
        let stale = self.lookup_stale(now);
        if self.has_lookup_identifier() {
            return (stale || self.wants_lookup()).then_some(ChainKind::Identifier);
        }
        if self.search_title().is_none() || !self.candidates.is_empty() {
            return None;
        }
        (stale || !self.incomplete().is_empty()).then_some(ChainKind::Title)
    }

    /// The title to search for: the paper's own, else the file's stem,
    /// which is what the list shows for a paper without one.
    pub fn search_title(&self) -> Option<String> {
        if let Some(title) = self.title() {
            return Some(title.to_owned());
        }
        let name = &self.files.first()?.name;
        let stem = Path::new(name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        non_empty(Some(&stem))
    }

    fn attach_file(&mut self, file: PaperFile) -> bool {
        if let Some(existing) = self.files.iter_mut().find(|f| f.path == file.path) {
            if *existing == file {
                return false;
            }
            *existing = file;
            self.order_files();
            return true;
        }
        // A missing row with the same bytes is this file, renamed or moved:
        // replace it rather than keep a ghost beside it. Different bytes
        // could be the journal version arriving after the preprint vanished,
        // so both stay.
        // "Missing" is only stamped when a scan finishes; a rename seen
        // mid-scan has the old path still marked present, so the disk is
        // asked directly rather than leaving a ghost row until next time.
        let moved = file.fingerprint.as_ref().and_then(|fingerprint| {
            self.files.iter().position(|f| {
                f.fingerprint.as_ref() == Some(fingerprint)
                    && (f.missing || !std::path::Path::new(&f.path).is_file())
            })
        });
        match moved {
            Some(position) => {
                if self.primary_path.as_deref() == Some(self.files[position].path.as_str()) {
                    self.primary_path = Some(file.path.clone());
                }
                self.files[position] = file;
            }
            None => self.files.push(file),
        }
        self.order_files();
        true
    }

    /// Put the main file first: the one the user made the main one,
    /// wherever a rescan put it back, else the one `mainfile` chooses.
    /// Returns whether the main file changed.
    pub(super) fn order_files(&mut self) -> bool {
        let chosen = match self.primary_path.as_deref() {
            Some(primary) if self.files.iter().any(|file| file.path == primary) => {
                self.files.iter().position(|file| file.path == primary)
            }
            _ => {
                let journal = self
                    .ids
                    .doi
                    .as_deref()
                    .filter(|doi| !doi.starts_with("10.48550/"));
                mainfile::main_index(&self.files, journal)
            }
        };
        match chosen {
            Some(at) if at > 0 => {
                let file = self.files.remove(at);
                self.files.insert(0, file);
                true
            }
            _ => false,
        }
    }

    /// Everything a scan found about a file, offered at its rank. The
    /// identifiers carry their source as the metadata does: the scan's own
    /// are the primary file's to re-derive on every rescan, a lookup's or
    /// the user's persist, and any file fills one the record lacks
    /// (`offer_identifier`). The arXiv category goes with the arXiv id.
    fn offer_entry(
        &mut self,
        entry: &LibraryEntry,
        doi: Option<(String, MetaSource)>,
        arxiv: Option<(String, MetaSource)>,
    ) -> bool {
        let mut changed = false;
        // What the file itself said is re-derived on every rescan: a value the
        // Info dictionary gave last time and no longer gives (the filter got
        // stricter, the file was replaced) must not outlive the evidence for
        // it. Only lookups and the user's own edits persist. Judged from the
        // paper's primary file, which is also the only one whose fields are
        // offered: a second file can neither retract nor overwrite what the
        // first one supplies.
        let path = entry.path.to_string_lossy();
        let primary = self.files.first().is_none_or(|file| file.path == path);
        // Whether this is the only file present: what the record holds may
        // be another file's say, which a rescan of this one cannot retract.
        let lone = !self
            .files
            .iter()
            .any(|file| !file.missing && file.path != path);
        // What the file's own identifiers say about its year, before they
        // are moved into the record: the arXiv id's submission year, else
        // the year a DOI of a known family carries (a PII's included).
        let identifier_year = arxiv
            .as_ref()
            .and_then(|(id, _)| identifiers::year_from_arxiv(id))
            .or_else(|| {
                doi.as_ref()
                    .and_then(|(id, _)| identifiers::year_from_doi(id))
            });
        changed |= offer_identifier(
            &mut self.ids.doi,
            &mut self.ids.doi_source,
            doi,
            primary,
            lone,
        );
        changed |= offer_identifier(
            &mut self.ids.arxiv,
            &mut self.ids.arxiv_source,
            arxiv,
            primary,
            lone,
        );
        let isbn = entry
            .isbn
            .as_deref()
            .and_then(normalize_isbn)
            .map(|isbn| (isbn, MetaSource::FirstPage))
            .or_else(|| {
                let isbn = normalize_isbn(entry.xmp.as_ref()?.isbn.as_deref()?)?;
                Some((isbn, MetaSource::Xmp))
            });
        changed |= offer_identifier(
            &mut self.ids.isbn,
            &mut self.ids.isbn_source,
            isbn,
            primary,
            lone,
        );
        // The category off the stamp belongs to the arXiv id: filled from
        // any file when the record has none, and re-derived with a
        // scan-owned id from the primary file under the same guard - a
        // stale one would send the lookup chain to the wrong services.
        let category = entry.arxiv_category.clone();
        let rederive = primary && scan_owned(self.ids.arxiv_source) && (category.is_some() || lone);
        if (self.arxiv_category.is_none() || rederive) && self.arxiv_category != category {
            self.arxiv_category = category;
            changed = true;
        }
        // Everything below is the primary file's say. A second file - the
        // journal version beside the preprint, a supplement - contributes
        // identifiers above and nothing else, so the record is a function
        // of one file and a rescan of the other cannot flip its fields.
        if !primary {
            return changed;
        }

        // What the XMP packet offers, filtered the way the Info dictionary
        // is: a Title that is a file name is no title whoever wrote it.
        let xmp = entry.xmp.as_ref();
        let xmp_title = xmp
            .and_then(|xmp| xmp.title.as_deref())
            .filter(|title| plausible_info_title(title))
            .and_then(|title| non_empty(Some(title)));
        let xmp_authors = xmp
            .map(|xmp| parse_xmp_creators(&xmp.creators))
            .unwrap_or_default();
        let xmp_year = xmp.and_then(|xmp| xmp.year);
        let xmp_venue = xmp.and_then(|xmp| non_empty(xmp.publication_name.as_deref()));
        let xmp_volume = xmp.and_then(|xmp| non_empty(xmp.volume.as_deref()));
        let xmp_issue = xmp.and_then(|xmp| non_empty(xmp.number.as_deref()));
        let xmp_pages = xmp.and_then(|xmp| xmp.pages());
        let xmp_publisher = xmp.and_then(|xmp| non_empty(xmp.publisher.as_deref()));

        let info_title = non_empty(entry.info_title.as_deref());
        let info_authors = entry
            .info_author
            .as_deref()
            .map(parse_info_authors)
            .unwrap_or_default();
        changed |= retract(
            &mut self.meta.title,
            MetaSource::PdfInfo,
            info_title.is_none(),
        );
        // What the printed first page says (`firstpage`). Ranked below the
        // Info dictionary and XMP either way; a reading the layout was not
        // sure of is offered only when the file says nothing else, so a
        // misread page can fill a gap but never stand in for metadata.
        let page_title = non_empty(entry.title.as_deref()).filter(|_| {
            entry.first_page_title_confident || (info_title.is_none() && xmp_title.is_none())
        });
        let page_authors = if entry.first_page_authors_confident
            || (info_authors.is_empty() && xmp_authors.is_empty())
        {
            entry.first_page_authors.clone()
        } else {
            Vec::new()
        };
        changed |= retract(
            &mut self.meta.title,
            MetaSource::FirstPage,
            page_title.is_none(),
        );
        changed |= retract(
            &mut self.meta.authors,
            MetaSource::FirstPage,
            page_authors.is_empty(),
        );
        changed |= retract(
            &mut self.meta.authors,
            MetaSource::PdfInfo,
            info_authors.is_empty(),
        );
        changed |= retract(
            &mut self.meta.year,
            MetaSource::PdfInfo,
            entry.info_year.is_none(),
        );
        changed |= retract(
            &mut self.meta.year,
            MetaSource::Identifier,
            identifier_year.is_none(),
        );
        changed |= retract(&mut self.meta.title, MetaSource::Xmp, xmp_title.is_none());
        changed |= retract(
            &mut self.meta.authors,
            MetaSource::Xmp,
            xmp_authors.is_empty(),
        );
        changed |= retract(&mut self.meta.year, MetaSource::Xmp, xmp_year.is_none());
        changed |= retract(&mut self.meta.venue, MetaSource::Xmp, xmp_venue.is_none());
        changed |= retract(&mut self.meta.volume, MetaSource::Xmp, xmp_volume.is_none());
        changed |= retract(&mut self.meta.issue, MetaSource::Xmp, xmp_issue.is_none());
        changed |= retract(&mut self.meta.pages, MetaSource::Xmp, xmp_pages.is_none());
        changed |= retract(
            &mut self.meta.publisher,
            MetaSource::Xmp,
            xmp_publisher.is_none(),
        );
        // The verdict is the primary file's, re-read on every scan. An
        // entry without one (indexed before the scan classified) says
        // nothing and changes nothing.
        if let Some(verdict) = entry.paper_verdict {
            let reason = entry.paper_reason.clone().unwrap_or_default();
            if self.verdict != Some(verdict) || self.verdict_reason != reason {
                self.verdict = Some(verdict);
                self.verdict_reason = reason;
                changed = true;
            }
        }
        // The kind the classifier saw beside the verdict - a thesis's title
        // page, a book's ISBN - is offered at first-page rank, where any
        // lookup and the user outrank it, and taken back by a classified
        // rescan that no longer sees it.
        changed |= retract(
            &mut self.meta.kind,
            MetaSource::FirstPage,
            entry.paper_verdict.is_some() && entry.paper_kind.is_none(),
        );
        if let Some(kind) = entry.paper_kind {
            changed |= offer(&mut self.meta.kind, kind, MetaSource::FirstPage);
        }
        if let Some(title) = page_title {
            changed |= offer(&mut self.meta.title, title, MetaSource::FirstPage);
        }
        if !page_authors.is_empty() {
            changed |= offer(&mut self.meta.authors, page_authors, MetaSource::FirstPage);
        }
        if let Some(title) = info_title {
            changed |= offer(&mut self.meta.title, title, MetaSource::PdfInfo);
        }
        if !info_authors.is_empty() {
            changed |= offer(&mut self.meta.authors, info_authors, MetaSource::PdfInfo);
        }
        if let Some(year) = entry.info_year {
            changed |= offer(&mut self.meta.year, year, MetaSource::PdfInfo);
        }
        if let Some(year) = identifier_year {
            changed |= offer(&mut self.meta.year, year, MetaSource::Identifier);
        }
        if let Some(title) = xmp_title {
            changed |= offer(&mut self.meta.title, title, MetaSource::Xmp);
        }
        if !xmp_authors.is_empty() {
            changed |= offer(&mut self.meta.authors, xmp_authors, MetaSource::Xmp);
        }
        if let Some(year) = xmp_year {
            changed |= offer(&mut self.meta.year, year, MetaSource::Xmp);
        }
        for (field, value) in [
            (&mut self.meta.venue, xmp_venue),
            (&mut self.meta.volume, xmp_volume),
            (&mut self.meta.issue, xmp_issue),
            (&mut self.meta.pages, xmp_pages),
            (&mut self.meta.publisher, xmp_publisher),
        ] {
            if let Some(value) = value {
                changed |= offer(field, value, MetaSource::Xmp);
            }
        }
        changed
    }

    fn search_text(&self) -> String {
        let mut text = String::new();
        text.push_str(&self.display_title());
        for author in self.authors() {
            text.push(' ');
            text.push_str(&author.given);
            text.push(' ');
            text.push_str(&author.family);
        }
        if let Some(year) = self.year() {
            text.push(' ');
            text.push_str(&year.to_string());
        }
        if let Some(venue) = self.venue() {
            text.push(' ');
            text.push_str(venue);
        }
        text.push(' ');
        text.push_str(&self.cite_key);
        fold(&text)
    }

    fn matches(&self, terms: &[String]) -> bool {
        if terms.is_empty() {
            return true;
        }
        let haystack = self.search_text();
        terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

impl Default for PaperRecord {
    fn default() -> Self {
        Self::new()
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Lower-case with diacritics removed: "Gärtner" and "gartner" match each
/// other whichever one was typed.
pub fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(fold)
        .filter(|term| !term.is_empty())
        .collect()
}

/// A BibTeX key the user can actually cite with. Whitespace and the
/// characters BibTeX reserves are refused rather than silently stripped,
/// because the user is about to paste this key into a manuscript.
fn validate_cite_key(key: &str) -> AppResult<()> {
    const RESERVED: &[char] = &[',', '{', '}', '(', ')', '"', '#', '%', '\'', '=', '~', '\\'];
    if key.is_empty() {
        return Err(AppError::Internal("cite key cannot be empty".into()));
    }
    if key.chars().any(char::is_whitespace) || key.contains(RESERVED) {
        return Err(AppError::Internal(
            "cite key cannot contain spaces or any of , { } ( ) \" # % ' = ~ \\".into(),
        ));
    }
    Ok(())
}

/// What a lookup has to go on for one paper: its identifiers, and what it
/// already has, which decides which services are worth asking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupTarget {
    pub paper_id: String,
    /// For a title search; `None` when the paper has neither a title nor a
    /// file to take a name from.
    pub title: Option<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    pub isbn: Option<String>,
    pub has_volume: bool,
    pub has_pages: bool,
    /// Candidates the user declined before (`titlematch::identity`).
    pub declined: Vec<String>,
    /// The category off the file's arXiv stamp, for the physics and
    /// computer-science decisions when arXiv itself has not been asked.
    pub arxiv_category: Option<String>,
}

/// What a batch lookup will do, settled before any request is made: the
/// papers worth asking about, each with the chain to run, and how many it
/// leaves alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupPlan {
    pub targets: Vec<(ChainKind, LookupTarget)>,
    /// Not a paper, parked candidates waiting on the user, nothing to ask
    /// with, or looked up lately and complete.
    pub skipped: u32,
}

impl PaperStore {
    pub(super) fn index_of(&self, id: &str) -> AppResult<usize> {
        self.papers
            .iter()
            .position(|paper| paper.id == id)
            .ok_or_else(|| AppError::Internal("paper not found".into()))
    }

    /// Like `index_of`, but a record merged away answers with the record it
    /// was merged into: a lookup that merged its paper mid-way, or a window
    /// that has not heard of the merge yet, still reaches the item.
    pub(super) fn live_index(&self, id: &str) -> AppResult<usize> {
        self.index_of(id).or_else(|error| {
            self.papers
                .iter()
                .position(|paper| paper.merged_from.iter().any(|snapshot| snapshot.id == id))
                .ok_or(error)
        })
    }

    /// Make the file at `path` the item's main one: first in its list, so
    /// it is the one opened, previewed and read for fields. Kept across
    /// rescans.
    pub fn set_primary_file(&mut self, id: &str, path: &str) -> AppResult<PaperDto> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        let paper = &mut self.papers[index];
        let at = paper
            .files
            .iter()
            .position(|file| file.path == path)
            .ok_or_else(|| AppError::Internal("that file is not one of this item's".into()))?;
        let file = paper.files.remove(at);
        paper.files.insert(0, file);
        paper.primary_path = Some(path.to_owned());
        paper.touch();
        self.mark_dirty();
        Ok(self.dto_for(&self.papers[index]))
    }

    /// A free key for `meta`, counting every paper's key but `except`'s.
    pub(super) fn free_key(&self, meta: &PaperMeta, except: Option<usize>) -> String {
        let taken: HashSet<&str> = self
            .papers
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != except)
            .flat_map(|(_, paper)| {
                std::iter::once(paper.cite_key.as_str())
                    .chain(paper.alias_keys.iter().map(String::as_str))
            })
            .collect();
        citekey::generate(meta, |key| taken.contains(key))
    }

    /// Match a scanned file to its paper, or make one, and fold what the
    /// scan found into it. Returns whether anything changed, so a scan can
    /// batch its saves and change events.
    pub fn reconcile_file(&mut self, entry: &LibraryEntry) -> bool {
        if !self.writable() {
            return false;
        }
        // The DOI from the file name or the first page (a PII the scan
        // converted included), else the one the XMP packet carries - or
        // stands for, when it holds a checked PII instead: the publisher
        // wrote it, and it is the same DOI by any route. But a packet's DOI
        // is a weaker identity than the page's - a supplement and the
        // issue's front matter carry the article's DOI in their packets -
        // so it resolves a file to a paper only when the file is not
        // already known by its bytes or its path.
        // Each identifier carries where the scan read it, for the record to
        // keep beside the value; an entry from before the scan recorded
        // sources is taken for the page's.
        let page_doi = entry
            .doi
            .as_deref()
            .and_then(normalize_doi)
            .map(|doi| (doi, entry.doi_source.unwrap_or(MetaSource::FirstPage)));
        let xmp_doi = entry
            .xmp
            .as_ref()
            .and_then(|xmp| {
                xmp.doi.clone().or_else(|| {
                    xmp.pii
                        .as_deref()
                        .filter(|pii| identifiers::pii_has_valid_issn(pii))
                        .map(identifiers::doi_from_pii)
                })
            })
            .as_deref()
            .and_then(normalize_doi)
            .map(|doi| (doi, MetaSource::Xmp));
        let xmp_only = page_doi.is_none() && xmp_doi.is_some();
        let mut doi = page_doi.or(xmp_doi);
        let arxiv = entry
            .arxiv
            .as_deref()
            .and_then(normalize_arxiv)
            .map(|arxiv| (arxiv, entry.arxiv_source.unwrap_or(MetaSource::FirstPage)));
        let path = entry.path.to_string_lossy().into_owned();
        let file = PaperFile {
            name: entry
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            path: path.clone(),
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            fingerprint: entry.fingerprint.clone(),
            missing: false,
            missing_since: None,
            traits: mainfile::traits_of(entry),
            duplicate_of: None,
        };

        let strong_doi = if xmp_only {
            None
        } else {
            doi.as_ref().map(|(doi, _)| doi.as_str())
        };
        let arxiv_id = arxiv.as_ref().map(|(arxiv, _)| arxiv.as_str());
        let strong_doi_id = strong_doi.map(|doi| CitationIdDto::Doi(doi.to_owned()));
        let arxiv_cid = arxiv_id.map(|arxiv| CitationIdDto::ArXiv(arxiv.to_owned()));
        let by_identifier = self
            .papers
            .iter()
            .position(|paper| {
                (strong_doi.is_some() && paper.ids.doi.as_deref() == strong_doi)
                    || (arxiv_id.is_some() && paper.ids.arxiv.as_deref() == arxiv_id)
            })
            .or_else(|| {
                // A merged record's identifier: the file belongs to the
                // record it was merged into.
                self.papers.iter().position(|paper| {
                    strong_doi_id.as_ref().is_some_and(|id| paper.has_alias(id))
                        || arxiv_cid.as_ref().is_some_and(|id| paper.has_alias(id))
                })
            });
        let by_fingerprint = || {
            let fingerprint = entry.fingerprint.as_ref()?;
            self.papers.iter().position(|paper| {
                paper
                    .files
                    .iter()
                    .any(|f| f.fingerprint.as_ref() == Some(fingerprint))
            })
        };
        let by_path = || {
            self.papers
                .iter()
                .position(|paper| paper.files.iter().any(|f| f.path == path))
        };
        let by_xmp_doi = || {
            if !xmp_only {
                return None;
            }
            let packet_doi = doi.as_ref().map(|(doi, _)| doi.as_str());
            self.papers.iter().position(|paper| {
                paper.ids.doi.as_deref() == packet_doi
                    || packet_doi
                        .is_some_and(|doi| paper.has_alias(&CitationIdDto::Doi(doi.to_owned())))
            })
        };

        let mut changed = false;
        let own = by_identifier.or_else(by_fingerprint).or_else(by_path);
        let index = match (own, by_xmp_doi()) {
            // The file's own record, and the packet's DOI names another: a
            // placeholder for the file moves to that paper; a record the
            // user filed or edited keeps the file, and keeps the DOI out so
            // two papers never share one.
            (Some(own), Some(holder)) if holder != own => {
                let id = self.papers[own].id.clone();
                let protected =
                    !self.project_ids_for(&id).is_empty() || has_user_edit(&self.papers[own]);
                if protected {
                    doi = None;
                    own
                } else {
                    holder
                }
            }
            (Some(own), _) => own,
            (None, Some(holder)) => holder,
            (None, None) => {
                let mut paper = PaperRecord::new();
                paper.offer_entry(entry, doi.clone(), arxiv.clone());
                paper.cite_key = self.free_key(&paper.meta, None);
                self.papers.push(paper);
                changed = true;
                self.papers.len() - 1
            }
        };
        let (detached, index) = self.detach_path_elsewhere(index, &path);
        changed |= detached;

        let base_before = citekey::base_key(&self.papers[index].meta);
        let paper = &mut self.papers[index];
        // An identifier the record answers to only as an alias stays an
        // alias: it is the merged record's, never this one's own.
        let doi = doi.filter(|(doi, _)| !paper.has_alias(&CitationIdDto::Doi(doi.clone())));
        let arxiv =
            arxiv.filter(|(arxiv, _)| !paper.has_alias(&CitationIdDto::ArXiv(arxiv.clone())));
        changed |= paper.attach_file(file);
        changed |= paper.offer_entry(entry, doi, arxiv);
        let base_after = citekey::base_key(&paper.meta);
        if !paper.cite_key_pinned && (base_before != base_after || paper.cite_key.is_empty()) {
            let meta = paper.meta.clone();
            let key = self.free_key(&meta, Some(index));
            self.papers[index].cite_key = key;
            changed = true;
        }
        if changed {
            self.papers[index].touch();
            self.mark_dirty();
            let id = self.papers[index].id.clone();
            self.auto_merge_around(&id);
        }
        changed
    }

    /// A path belongs to one paper. When a file is re-identified as another
    /// paper's - its DOI matched after a user set one, say - the old row goes;
    /// a paper left with no files, no memberships and no user edits was only
    /// ever a placeholder for that file and goes with it.
    ///
    /// Returns whether anything changed and where `keep` is now: removing a
    /// placeholder that sat before it shifts it down by one, and the caller
    /// indexes with the answer, not with what it passed in.
    fn detach_path_elsewhere(&mut self, keep: usize, path: &str) -> (bool, usize) {
        let mut keep = keep;
        let mut changed = false;
        let mut index = 0;
        while index < self.papers.len() {
            if index != keep && self.papers[index].files.iter().any(|f| f.path == path) {
                self.papers[index].files.retain(|f| f.path != path);
                self.papers[index].touch();
                changed = true;
                let id = self.papers[index].id.clone();
                let placeholder = self.papers[index].files.is_empty()
                    && self.project_ids_for(&id).is_empty()
                    && !has_user_edit(&self.papers[index]);
                if placeholder {
                    self.papers.remove(index);
                    if index < keep {
                        keep -= 1;
                    }
                    continue;
                }
            }
            index += 1;
        }
        (changed, keep)
    }

    /// After a scan: every file not in `present` is missing, every file in
    /// it is not. Never deletes a paper - it keeps its projects and the next
    /// matching file reattaches.
    pub fn mark_missing(&mut self, present: &HashSet<PathBuf>) -> bool {
        if !self.writable() {
            return false;
        }
        let now = now_rfc3339();
        let mut changed = false;
        for paper in &mut self.papers {
            let mut touched = false;
            for file in &mut paper.files {
                let missing = !present.contains(Path::new(&file.path));
                if file.missing != missing {
                    file.missing = missing;
                    touched = true;
                }
                touched |= stamp_missing_since(file, &now);
            }
            if touched {
                paper.order_files();
                paper.touch();
                changed = true;
            }
        }
        changed |= self.remove_long_missing(parse_rfc3339(&now).unwrap_or(0));
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// The watcher saw these paths go - files, or folders whose files went
    /// with them: mark each paper file at or below one missing now, with
    /// its missing clock started, rather than at the next full scan. A
    /// file that is somehow still there is left alone.
    pub fn mark_paths_missing(&mut self, gone: &[PathBuf]) -> bool {
        if !self.writable() || gone.is_empty() {
            return false;
        }
        let now = now_rfc3339();
        let mut changed = false;
        for paper in &mut self.papers {
            let mut touched = false;
            for file in &mut paper.files {
                let path = Path::new(&file.path);
                if !file.missing && gone.iter().any(|g| path.starts_with(g)) && !path.is_file() {
                    file.missing = true;
                    stamp_missing_since(file, &now);
                    touched = true;
                }
            }
            if touched {
                paper.order_files();
                paper.touch();
                changed = true;
            }
        }
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// Remove every item all of whose files have been missing for at least
    /// [`MISSING_GRACE`] at `now` (unix seconds), unless dropping it would
    /// lose something: it is in a project, has the user's edits or a
    /// pinned key, carries merged records or aliases, or has candidates
    /// parked for the user. A file on a volume that is not mounted is not
    /// counted as gone: an external drive away for a week is not a
    /// deletion. Returns whether anything was removed.
    pub fn remove_long_missing(&mut self, now: u64) -> bool {
        if !self.writable() {
            return false;
        }
        let filed: HashSet<String> = self
            .projects
            .iter()
            .flat_map(|project| project.paper_ids.iter().cloned())
            .collect();
        let before = self.papers.len();
        self.papers.retain(|paper| {
            let gone_long = !paper.files.is_empty()
                && paper.files.iter().all(|file| {
                    file.missing
                        && !on_unmounted_volume(&file.path)
                        && file
                            .missing_since
                            .as_deref()
                            .and_then(parse_rfc3339)
                            .is_some_and(|since| {
                                now.saturating_sub(since) >= MISSING_GRACE.as_secs()
                            })
                });
            let keep = !gone_long
                || filed.contains(&paper.id)
                || has_user_edit(paper)
                || !paper.alias_ids.is_empty()
                || !paper.candidates.is_empty();
            if !keep {
                log::info!(
                    "removed library item {} ({}): its files have been missing for a week",
                    paper.id,
                    paper.display_title()
                );
            }
            keep
        });
        let removed = self.papers.len() != before;
        if removed {
            self.mark_dirty();
        }
        removed
    }

    /// What a load does once before anything else reads the records:
    /// start the missing clock on files missing from before it was kept,
    /// choose every item's main file again, and remove the items that have
    /// been gone for a week. Returns whether anything changed.
    pub fn load_sweep(&mut self, now: u64) -> bool {
        if !self.writable() {
            return false;
        }
        let stamp = format_rfc3339(now);
        let mut changed = false;
        for paper in &mut self.papers {
            let mut touched = false;
            for file in &mut paper.files {
                touched |= stamp_missing_since(file, &stamp);
            }
            touched |= paper.order_files();
            if touched {
                changed = true;
            }
        }
        changed |= self.remove_long_missing(now);
        if changed {
            self.mark_dirty();
        }
        changed
    }

    pub fn paper_dto(&self, id: &str) -> AppResult<PaperDto> {
        let paper = &self.papers[self.live_index(id)?];
        Ok(self.dto_for(paper))
    }

    pub(super) fn dto_for(&self, paper: &PaperRecord) -> PaperDto {
        let mut files = paper.files.clone();
        for (file, copy) in files.iter_mut().zip(mainfile::duplicate_of(&paper.files)) {
            file.duplicate_of = copy;
        }
        PaperDto {
            id: paper.id.clone(),
            files,
            doi: paper.ids.doi.clone(),
            arxiv: paper.ids.arxiv.clone(),
            isbn: paper.ids.isbn.clone(),
            meta: paper.meta.clone(),
            cite_key: paper.cite_key.clone(),
            cite_key_pinned: paper.cite_key_pinned,
            project_ids: self.project_ids_for(&paper.id),
            added_at: paper.added_at.clone(),
            updated_at: paper.updated_at.clone(),
            incomplete: paper.incomplete(),
            is_paper: paper.is_paper(),
            paper_reason: paper.paper_reason(),
            paper_override: paper.paper_override,
            candidates: paper.candidates.clone(),
            merged_count: paper.merged_from.len().min(u32::MAX as usize) as u32,
            auto_merged: paper.auto_merge_reasons.clone(),
            vouched: paper.vouched_counts(),
            year_floor: paper.shown_year_floor(),
            year_hint: paper.year_hint(),
        }
    }

    /// The paper one of whose files is at `path` (canonical), if any.
    pub fn paper_for_path(&self, path: &Path) -> Option<PaperDto> {
        let path = path.to_string_lossy();
        self.papers
            .iter()
            .find(|paper| paper.files.iter().any(|f| f.path == path))
            .map(|paper| self.dto_for(paper))
    }

    /// The cite key of the paper one of whose files is at `path` (canonical).
    pub fn cite_key_for_path(&self, path: &Path) -> Option<String> {
        let path = path.to_string_lossy();
        self.papers
            .iter()
            .find(|paper| paper.files.iter().any(|f| f.path == path))
            .map(|paper| paper.cite_key.clone())
            .filter(|key| !key.is_empty())
    }

    /// Files of a paper that are on disk, in record order.
    pub fn present_files(&self, id: &str) -> AppResult<Vec<PathBuf>> {
        let paper = &self.papers[self.index_of(id)?];
        Ok(paper
            .files
            .iter()
            .filter(|file| !file.missing)
            .map(|file| PathBuf::from(&file.path))
            .collect())
    }

    /// `all` and `unfiled` count papers (`is_paper`); `others` counts the
    /// rest, in a project or not.
    pub fn counts(&self) -> LibraryCountsDto {
        let filed: HashSet<&str> = self
            .projects
            .iter()
            .flat_map(|project| project.paper_ids.iter().map(String::as_str))
            .collect();
        let cap = |value: usize| value.min(u32::MAX as usize) as u32;
        let papers = self.papers.iter().filter(|paper| paper.is_paper());
        LibraryCountsDto {
            all: cap(papers.clone().count()),
            unfiled: cap(papers
                .filter(|paper| !filed.contains(paper.id.as_str()))
                .count()),
            others: cap(self.papers.iter().filter(|paper| !paper.is_paper()).count()),
        }
    }

    /// Rows for the list. `total` counts the selection before the query
    /// narrows it, so a header can say "23" while a search shows 4.
    ///
    /// The scope: a `project` is every member, paper or not; `others` is
    /// every non-paper, filed or not; `unfiled` is the papers in no
    /// project; nothing set is every paper. Non-papers only ever show
    /// under `others` or inside a project they were put in.
    pub fn paper_rows(&self, filter: &PaperFilterDto) -> PaperListDto {
        let memberships = self.membership_map();
        let in_project: Option<HashSet<&str>> = filter.project.as_deref().map(|id| {
            self.projects
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.paper_ids.iter().map(String::as_str).collect())
                .unwrap_or_default()
        });
        let selected: Vec<&PaperRecord> = self
            .papers
            .iter()
            .filter(|paper| match &in_project {
                Some(members) => members.contains(paper.id.as_str()),
                None if filter.others => !paper.is_paper(),
                None => {
                    paper.is_paper()
                        && (!filter.unfiled || !memberships.contains_key(paper.id.as_str()))
                }
            })
            .collect();
        let total = selected.len().min(u32::MAX as usize) as u32;

        let terms = query_terms(&filter.query);
        let matched: Vec<&PaperRecord> = selected
            .into_iter()
            .filter(|paper| paper.matches(&terms))
            .collect();
        PaperListDto {
            rows: self.rows_in_list_order(matched),
            total,
        }
    }

    fn row_for(
        &self,
        paper: &PaperRecord,
        memberships: &HashMap<&str, Vec<String>>,
    ) -> PaperRowDto {
        PaperRowDto {
            id: paper.id.clone(),
            title: paper.display_title(),
            authors: paper.authors_display(),
            year: paper.year(),
            venue: paper.venue().map(str::to_owned),
            cite_key: paper.cite_key.clone(),
            alpha_label: citekey::alpha_label(
                paper.authors(),
                paper.year(),
                &paper.display_title(),
            ),
            project_ids: memberships
                .get(paper.id.as_str())
                .cloned()
                .unwrap_or_default(),
            missing: paper.is_missing(),
            has_identifier: paper.has_identifier(),
            incomplete_count: paper.incomplete().len() as u32,
            is_paper: paper.is_paper(),
            candidate_count: paper.candidates.len().min(u32::MAX as usize) as u32,
            file_count: mainfile::distinct_present(&paper.files).min(u32::MAX as usize) as u32,
            year_floor: paper.shown_year_floor(),
        }
    }

    /// Rows in list order, however the papers were picked, so every list
    /// the screen shows - a project, a search, a paper's citations - reads
    /// the same way.
    pub(super) fn rows_in_list_order(&self, papers: Vec<&PaperRecord>) -> Vec<PaperRowDto> {
        let memberships = self.membership_map();
        self.in_list_order(papers)
            .into_iter()
            .map(|paper| self.row_for(paper, &memberships))
            .collect()
    }

    /// First author's family, then year, then title - the order of the list.
    pub(super) fn in_list_order<'a>(
        &self,
        mut papers: Vec<&'a PaperRecord>,
    ) -> Vec<&'a PaperRecord> {
        papers.sort_by_cached_key(|paper| SortKey::for_paper(paper));
        papers
    }

    /// The identifier -> paper and path -> paper tables citation resolution
    /// goes through, and the title index for references that print no
    /// identifier, built once per call over every paper.
    pub(super) fn citation_tables(&self) -> CitationTables<'_> {
        let mut by_id: HashMap<CitationIdDto, usize> = HashMap::new();
        let mut by_path: HashMap<&str, usize> = HashMap::new();
        for (index, paper) in self.papers.iter().enumerate() {
            if let Some(doi) = &paper.ids.doi {
                by_id
                    .entry(CitationIdDto::Doi(doi.clone()))
                    .or_insert(index);
            }
            if let Some(arxiv) = &paper.ids.arxiv {
                by_id
                    .entry(CitationIdDto::ArXiv(arxiv.clone()))
                    .or_insert(index);
            }
            for file in &paper.files {
                by_path.entry(file.path.as_str()).or_insert(index);
            }
        }
        // Aliases after every record's own identifiers, so one never
        // shadows another record's own.
        for (index, paper) in self.papers.iter().enumerate() {
            for alias in &paper.alias_ids {
                by_id.entry(alias.clone()).or_insert(index);
            }
        }
        let titles = TitleIndex::new(
            self.papers
                .iter()
                .enumerate()
                .filter_map(|(index, paper)| Some((index, paper.title()?, paper.year()))),
        );
        CitationTables {
            by_id,
            by_path,
            titles,
        }
    }

    /// Which library papers this one cites and which cite it, and how many
    /// of its citations name no paper here (projects.md sec. 8). Three sources
    /// of edges, unioned: the identifiers the text pass read out of each
    /// file (`entries[].cites`), each file's reference list entry by entry
    /// (`refs`, the refs sidecars: an entry resolves by the identifiers in
    /// it, else by the library title it contains - `resolve_reference`),
    /// and what the lookup services said (`network`: what each paper cites
    /// and is cited by, from the cache). _Cites_ is all three for this
    /// paper; _cited by_ is every paper whose cites, reference entries or
    /// network references name this one, plus this paper's network
    /// citations. The tables are built once per call.
    ///
    /// `unresolved` counts, for a paper whose reference list was read,
    /// its reference entries that resolve to no library paper; for one
    /// whose list was not (no refs sidecar: not indexed with this build, or
    /// no list found), the distinct cited identifiers that name no paper,
    /// as before reference lists were read.
    pub fn citation_links(
        &self,
        paper_id: &str,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<CitationLinksDto> {
        let own = self.index_of(paper_id)?;
        let tables = self.citation_tables();
        let (cites, cited_by, unresolved) = self.link_indices(own, &tables, entries, refs, network);
        let papers = |indices: &[usize]| indices.iter().map(|&i| &self.papers[i]).collect();
        Ok(CitationLinksDto {
            cites: self.rows_in_list_order(papers(&cites)),
            cited_by: self.rows_in_list_order(papers(&cited_by)),
            unresolved: unresolved.min(u32::MAX as usize) as u32,
        })
    }

    /// `citation_links` as paper indices, against tables the caller built:
    /// the papers `own` cites and the papers citing it, each once, and how
    /// many of its citations resolve to nothing.
    pub(super) fn link_indices(
        &self,
        own: usize,
        tables: &CitationTables<'_>,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> (Vec<usize>, Vec<usize>, usize) {
        let own_ids: HashSet<&CitationIdDto> = tables
            .by_id
            .iter()
            .filter(|(_, index)| **index == own)
            .map(|(id, _)| id)
            .collect();

        // Paper indices, each once, in index order; the rows are sorted
        // afterwards. A paper with two files cites the union of both.
        let mut cites: Vec<usize> = Vec::new();
        let mut cited_by: Vec<usize> = Vec::new();
        let mut unresolved_ids: HashSet<&CitationIdDto> = HashSet::new();
        let mut unresolved_entries = 0usize;
        let mut own_refs_read = false;
        for entry in entries {
            let Some(cited) = entry.cites.as_ref() else {
                continue;
            };
            let path = entry.path.to_string_lossy();
            let Some(&holder) = tables.by_path.get(path.as_ref()) else {
                continue;
            };
            let list = refs.get(&entry.path);
            if holder == own {
                own_refs_read |= list.is_some();
                for id in cited {
                    match tables.by_id.get(id) {
                        Some(&target) if target == own => {}
                        Some(&target) => push_once(&mut cites, target),
                        None if list.is_none() => {
                            unresolved_ids.insert(id);
                        }
                        None => {}
                    }
                }
                for reference in list.map(|list| list.as_slice()).unwrap_or(&[]) {
                    match tables.resolve_reference(reference, own) {
                        Some(target) if target == own => {}
                        Some(target) => push_once(&mut cites, target),
                        None => unresolved_entries += 1,
                    }
                }
            } else if !cited_by.contains(&holder)
                && (cited.iter().any(|id| own_ids.contains(id))
                    || list.is_some_and(|list| {
                        list.iter().any(|reference| {
                            tables.resolve_reference(reference, holder) == Some(own)
                        })
                    }))
            {
                cited_by.push(holder);
            }
        }
        // The network's edges, keyed by each paper's own identifiers.
        for (index, paper) in self.papers.iter().enumerate() {
            for id in &paper.citation_ids() {
                let network_refs = network.refs.get(id).map(Vec::as_slice).unwrap_or(&[]);
                if index == own {
                    for cited in network_refs {
                        match tables.by_id.get(cited) {
                            Some(&target) if target == own => {}
                            Some(&target) => push_once(&mut cites, target),
                            None if !own_refs_read => {
                                unresolved_ids.insert(cited);
                            }
                            None => {}
                        }
                    }
                    for citing in network.cites.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                        if let Some(&source) = tables.by_id.get(citing) {
                            if source != own && !cited_by.contains(&source) {
                                cited_by.push(source);
                            }
                        }
                    }
                } else if !cited_by.contains(&index)
                    && network_refs.iter().any(|id| own_ids.contains(id))
                {
                    cited_by.push(index);
                }
            }
        }
        let unresolved = unresolved_entries + unresolved_ids.len();
        (cites, cited_by, unresolved)
    }

    /// The citation graph over a set of papers (projects.md sec. 8): every
    /// item in the library (`is_paper`), or every member of one project,
    /// paper or not, in list order. Edges come from the sources
    /// `citation_links` reads - the identifiers the text pass read out of
    /// each file (`entries`), each file's reference entries (`refs`,
    /// resolved by identifier, else by title), and the lookup cache's
    /// network (`network`) in both its directions - resolved through
    /// tables built once for the whole library, so a citation of a paper
    /// outside the set is told from one of nothing and a title is matched
    /// against every paper. An edge is drawn only between two nodes, once,
    /// never from a node to itself.
    ///
    /// `external` counts citations that leave the set: for a node whose
    /// reference list was read, each reference entry that resolves to no
    /// node (a paper outside the set, or nothing in the library); for a
    /// node without one, each distinct cited identifier that names no node,
    /// as before reference lists were read. A node none of whose files has
    /// had its cites read yet is counted in `unindexed`, so the view can
    /// say its citations are unknown rather than absent.
    pub fn citation_graph(
        &self,
        project_id: Option<&str>,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<GraphDto> {
        let nodes: Vec<&PaperRecord> = match project_id {
            Some(project_id) => self.project_members(project_id)?,
            None => self.in_list_order(self.papers.iter().filter(|p| p.is_paper()).collect()),
        };
        Ok(self.graph_over(nodes, entries, refs, network).0)
    }

    /// The graph around one paper: it and every item (`is_paper`) within
    /// `depth` citation steps of it, either direction, across the whole
    /// library - `depth` clamped to 1..=2 - with the edges among them and
    /// `external` / `unindexed` relative to that set, as `citation_graph`
    /// counts them for a project.
    pub fn focus_graph(
        &self,
        focus: &str,
        depth: u32,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<GraphDto> {
        let nodes = self.focus_nodes(focus, depth, entries, refs, network)?;
        Ok(self.graph_over(nodes, entries, refs, network).0)
    }

    /// The focus paper and the items within `depth` steps of it, in list
    /// order. The steps follow every edge the whole-library graph has.
    pub(super) fn focus_nodes(
        &self,
        focus: &str,
        depth: u32,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<Vec<&PaperRecord>> {
        let focus_index = self.index_of(focus)?;
        let universe: Vec<&PaperRecord> = self
            .papers
            .iter()
            .enumerate()
            .filter(|(index, paper)| *index == focus_index || paper.is_paper())
            .map(|(_, paper)| paper)
            .collect();
        let start = universe
            .iter()
            .position(|paper| paper.id == focus)
            .expect("the focus is in its own universe");
        let (_, edges) = self.graph_over(universe.clone(), entries, refs, network);
        let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); universe.len()];
        for (from, to) in edges {
            neighbours[from].push(to);
            neighbours[to].push(from);
        }
        let mut reached = vec![false; universe.len()];
        reached[start] = true;
        let mut frontier = vec![start];
        for _ in 0..depth.clamp(1, 2) {
            let mut next = Vec::new();
            for node in frontier {
                for &other in &neighbours[node] {
                    if !reached[other] {
                        reached[other] = true;
                        next.push(other);
                    }
                }
            }
            frontier = next;
        }
        let nodes = universe
            .into_iter()
            .zip(reached)
            .filter_map(|(paper, reached)| reached.then_some(paper))
            .collect();
        Ok(self.in_list_order(nodes))
    }

    /// `citation_graph` over a given node set, and its edges as node
    /// positions in `nodes` order.
    fn graph_over(
        &self,
        nodes: Vec<&PaperRecord>,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> (GraphDto, Vec<(usize, usize)>) {
        // Paper index -> node position.
        let node_by_id: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(position, paper)| (paper.id.as_str(), position))
            .collect();
        let node_of: Vec<Option<usize>> = self
            .papers
            .iter()
            .map(|paper| node_by_id.get(paper.id.as_str()).copied())
            .collect();
        let tables = self.citation_tables();
        let node_for = |id: &CitationIdDto| tables.by_id.get(id).and_then(|&index| node_of[index]);

        // Every citation by identifier a node makes - (node, cited
        // identifier) - from the sidecars and from the network's
        // references, resolved in one pass below. The network's other
        // direction, a paper the cache says cites a node, is an edge when
        // that paper is a node too and nothing to count when it is not.
        // Reference entries resolve as they are read.
        let mut citations: Vec<(usize, &CitationIdDto)> = Vec::new();
        let mut edges: HashSet<(usize, usize)> = HashSet::new();
        let mut indexed = vec![false; nodes.len()];
        let mut refs_read = vec![false; nodes.len()];
        let mut external_entries = 0usize;
        for entry in entries {
            let Some(cited) = entry.cites.as_ref() else {
                continue;
            };
            let path = entry.path.to_string_lossy();
            let Some(&holder) = tables.by_path.get(path.as_ref()) else {
                continue;
            };
            let Some(from) = node_of[holder] else {
                continue;
            };
            indexed[from] = true;
            citations.extend(cited.iter().map(|id| (from, id)));
            let Some(list) = refs.get(&entry.path) else {
                continue;
            };
            refs_read[from] = true;
            for reference in list.iter() {
                match tables.resolve_reference(reference, holder) {
                    Some(target) if target == holder => {}
                    Some(target) => match node_of[target] {
                        Some(to) if to == from => {}
                        Some(to) => {
                            edges.insert((from, to));
                        }
                        None => external_entries += 1,
                    },
                    None => external_entries += 1,
                }
            }
        }
        for (position, paper) in nodes.iter().enumerate() {
            for id in &paper.citation_ids() {
                let network_refs = network.refs.get(id).map(Vec::as_slice).unwrap_or(&[]);
                citations.extend(network_refs.iter().map(|cited| (position, cited)));
                for citing in network.cites.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(source) = node_for(citing) {
                        if source != position {
                            edges.insert((source, position));
                        }
                    }
                }
            }
        }
        let mut external_ids: HashSet<&CitationIdDto> = HashSet::new();
        for (from, id) in citations {
            match node_for(id) {
                Some(to) if to == from => {}
                Some(to) => {
                    edges.insert((from, to));
                }
                // A node whose reference list was read counts its entries
                // above; counting its identifiers too would count twice.
                None if !refs_read[from] => {
                    external_ids.insert(id);
                }
                None => {}
            }
        }
        let mut edges: Vec<(usize, usize)> = edges.into_iter().collect();
        edges.sort_unstable();
        let edge_dtos = edges
            .iter()
            .map(|&(from, to)| GraphEdgeDto {
                from: nodes[from].id.clone(),
                to: nodes[to].id.clone(),
            })
            .collect();
        let cap = |value: usize| value.min(u32::MAX as usize) as u32;
        let graph = GraphDto {
            edges: edge_dtos,
            external: cap(external_entries + external_ids.len()),
            unindexed: cap(indexed.iter().filter(|known| !**known).count()),
            nodes: self.rows_in_list_order(nodes),
        };
        (graph, edges)
    }

    /// What a lookup of this paper has to go on.
    pub fn lookup_target(&self, id: &str) -> AppResult<LookupTarget> {
        Ok(target_for(&self.papers[self.live_index(id)?]))
    }

    /// Which of a project's papers a project-wide lookup asks about, and
    /// how (`PaperRecord::batch_chain`), in list order.
    pub fn plan_project_lookup(&self, project_id: &str, now: u64) -> AppResult<LookupPlan> {
        Ok(plan_lookup(self.project_members(project_id)?, now))
    }

    /// The same over the whole library.
    pub fn plan_library_lookup(&self, now: u64) -> LookupPlan {
        plan_lookup(self.in_list_order(self.papers.iter().collect()), now)
    }

    /// Everything a chain found, applied in turn - each candidate at its
    /// own source's rank, so the best source wins field by field.
    pub fn apply_candidates(
        &mut self,
        id: &str,
        candidates: &[LookupCandidateDto],
    ) -> AppResult<PaperDto> {
        let mut id = id.to_owned();
        for candidate in candidates {
            id = self.apply_candidate(&id, candidate)?.id;
        }
        self.paper_dto(&id)
    }

    /// Stamp the paper as looked up now, whatever was found.
    pub fn mark_looked_up(&mut self, id: &str) -> AppResult<()> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        let paper = &mut self.papers[index];
        paper.looked_up_at = Some(now_rfc3339());
        paper.touch();
        Ok(())
    }

    /// Park what a title search found but could not apply, for the user to
    /// pick from - at most [`PARK_LIMIT`], replacing anything parked
    /// before. Returns how many were parked.
    pub fn park_candidates(
        &mut self,
        id: &str,
        mut candidates: Vec<LookupCandidateDto>,
    ) -> AppResult<usize> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        candidates.truncate(PARK_LIMIT);
        let paper = &mut self.papers[index];
        let parked = candidates.len();
        if paper.candidates != candidates {
            paper.candidates = candidates;
            paper.touch();
        }
        Ok(parked)
    }

    /// What an online lookup found, offered field by field at the
    /// candidate's rank: a lookup overwrites the local heuristics and never
    /// a user edit, and `offer` is what decides. A paper keeps the
    /// identifiers it has, since one of them is what resolved to this
    /// record; one it lacks lands with the candidate's source, which a
    /// rescan then leaves alone. An unpinned key follows the metadata as
    /// always.
    pub fn apply_candidate(
        &mut self,
        id: &str,
        candidate: &LookupCandidateDto,
    ) -> AppResult<PaperDto> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        let source = candidate.source;
        let base_before = citekey::base_key(&self.papers[index].meta);
        let paper = &mut self.papers[index];
        let mut changed = false;
        if let Some(title) = non_empty(Some(&candidate.title)) {
            changed |= offer(&mut paper.meta.title, title, source);
        }
        let authors: Vec<Author> = candidate
            .authors
            .iter()
            .map(|author| Author {
                family: author.family.trim().to_owned(),
                given: author.given.trim().to_owned(),
            })
            .filter(|author| !author.family.is_empty() || !author.given.is_empty())
            .collect();
        if !authors.is_empty() {
            changed |= offer(&mut paper.meta.authors, authors, source);
        }
        if let Some(year) = candidate.year {
            changed |= offer(&mut paper.meta.year, year, source);
        }
        for (field, value) in [
            (&mut paper.meta.venue, &candidate.venue),
            (&mut paper.meta.volume, &candidate.volume),
            (&mut paper.meta.issue, &candidate.issue),
            (&mut paper.meta.pages, &candidate.pages),
            (&mut paper.meta.publisher, &candidate.publisher),
            (&mut paper.meta.abstract_, &candidate.abstract_),
        ] {
            if let Some(value) = non_empty(value.as_deref()) {
                changed |= offer(field, value, source);
            }
        }
        if let Some(kind) = candidate.kind {
            changed |= offer(&mut paper.meta.kind, kind, source);
        }
        if paper.ids.doi.is_none() {
            if let Some(doi) = candidate.doi.as_deref().and_then(normalize_doi) {
                paper.ids.doi = Some(doi);
                paper.ids.doi_source = Some(source);
                changed = true;
            }
        }
        if paper.ids.arxiv.is_none() {
            if let Some(arxiv) = candidate.arxiv.as_deref().and_then(normalize_arxiv) {
                paper.ids.arxiv = Some(arxiv);
                paper.ids.arxiv_source = Some(source);
                changed = true;
            }
        }
        if paper.ids.isbn.is_none() {
            if let Some(isbn) = candidate.isbn.as_deref().and_then(normalize_isbn) {
                paper.ids.isbn = Some(isbn);
                paper.ids.isbn_source = Some(source);
                changed = true;
            }
        }
        // Applying one answers the question the parked candidates asked.
        if !paper.candidates.is_empty() {
            paper.candidates.clear();
            changed = true;
        }
        let base_after = citekey::base_key(&paper.meta);
        if !paper.cite_key_pinned && (base_before != base_after || paper.cite_key.is_empty()) {
            let meta = paper.meta.clone();
            let key = self.free_key(&meta, Some(index));
            self.papers[index].cite_key = key;
            changed = true;
        }
        if changed {
            self.papers[index].touch();
        }
        let id = self.papers[index].id.clone();
        let (survivor, _) = self.auto_merge_around(&id);
        self.paper_dto(&survivor)
    }

    /// Throw away the parked title-search candidates: the user looked and
    /// none was it. Each is remembered as declined, so a later search does
    /// not ask the same question.
    pub fn discard_candidates(&mut self, id: &str) -> AppResult<PaperDto> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        let paper = &mut self.papers[index];
        if !paper.candidates.is_empty() {
            for candidate in paper.candidates.drain(..) {
                let identity = titlematch::identity(&candidate);
                if !paper.declined.contains(&identity) {
                    paper.declined.push(identity);
                }
            }
            paper.touch();
        }
        self.paper_dto(id)
    }

    /// Pin the keys of these papers: they were just written to a `.bib`, or
    /// copied to be pasted into one, and a key that changes under a
    /// manuscript breaks every `\cite` in it (projects.md sec. 2). Returns
    /// whether any key was not pinned before. Nothing happens on a store
    /// that cannot be written - a pin that cannot be saved is not a pin.
    pub fn freeze_keys(&mut self, ids: &[String]) -> bool {
        if !self.writable() {
            return false;
        }
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut changed = false;
        for paper in &mut self.papers {
            if !paper.cite_key_pinned && ids.contains(paper.id.as_str()) {
                paper.cite_key_pinned = true;
                paper.touch();
                changed = true;
            }
        }
        changed
    }

    /// BibTeX for these papers in this order, for the clipboard, and
    /// whether rendering froze a key: the text is about to be pasted into a
    /// `.bib`, which is where keys freeze.
    pub fn bibtex_for(&mut self, ids: &[String]) -> AppResult<(String, bool)> {
        let dtos = ids
            .iter()
            .map(|id| self.paper_dto(id))
            .collect::<AppResult<Vec<_>>>()?;
        let text = bibtex::render(&dtos, false, &self.bibtex_options);
        let frozen = self.freeze_keys(ids);
        Ok((text, frozen))
    }

    /// A user edit. Fields left out of the patch are untouched; `null`
    /// clears. Everything set lands with source `user`, which nothing but
    /// another edit can replace.
    pub fn apply_patch(&mut self, id: &str, patch: MetaPatchDto) -> AppResult<PaperDto> {
        self.ensure_writable()?;
        let index = self.live_index(id)?;
        let identifier_set =
            matches!(patch.doi, Some(Some(_))) || matches!(patch.arxiv, Some(Some(_)));
        let pinned_key = match &patch.cite_key {
            Some(Some(key)) => {
                let key = key.trim();
                validate_cite_key(key)?;
                if self.papers.iter().enumerate().any(|(other, paper)| {
                    other != index
                        && (paper.cite_key == key || paper.alias_keys.iter().any(|k| k == key))
                }) {
                    return Err(AppError::Internal(format!(
                        "cite key \"{key}\" is already used by another paper"
                    )));
                }
                Some(key.to_owned())
            }
            _ => None,
        };

        let base_before = citekey::base_key(&self.papers[index].meta);
        let paper = &mut self.papers[index];
        set_text(&mut paper.meta.title, patch.title);
        if let Some(authors) = patch.authors {
            let authors = authors
                .map(|authors| {
                    authors
                        .into_iter()
                        .map(|author| Author {
                            family: author.family.trim().to_owned(),
                            given: author.given.trim().to_owned(),
                        })
                        .filter(|author| !author.family.is_empty() || !author.given.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|authors| !authors.is_empty());
            paper.meta.authors = authors.map(user_field);
        }
        if let Some(year) = patch.year {
            paper.meta.year = year.map(user_field);
        }
        set_text(&mut paper.meta.venue, patch.venue);
        set_text(&mut paper.meta.volume, patch.volume);
        set_text(&mut paper.meta.issue, patch.issue);
        set_text(&mut paper.meta.pages, patch.pages);
        set_text(&mut paper.meta.publisher, patch.publisher);
        set_text(&mut paper.meta.abstract_, patch.abstract_);
        if let Some(kind) = patch.kind {
            paper.meta.kind = kind.map(user_field);
        }
        // An identifier the user set is theirs, and no scan or lookup
        // replaces it; one they cleared is simply absent, and the next
        // scan may fill it as it would any other.
        if let Some(doi) = patch.doi {
            paper.ids.doi = doi.as_deref().and_then(normalize_doi);
            paper.ids.doi_source = paper.ids.doi.as_ref().map(|_| MetaSource::User);
        }
        if let Some(arxiv) = patch.arxiv {
            paper.ids.arxiv = arxiv.as_deref().and_then(normalize_arxiv);
            paper.ids.arxiv_source = paper.ids.arxiv.as_ref().map(|_| MetaSource::User);
        }
        if let Some(isbn) = patch.isbn {
            paper.ids.isbn = match isbn.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(raw) => Some(normalize_isbn(raw).ok_or_else(|| {
                    AppError::Internal(format!(
                        "\"{raw}\" is not a valid ISBN (10 or 13 digits with a correct check digit)"
                    ))
                })?),
            };
            paper.ids.isbn_source = paper.ids.isbn.as_ref().map(|_| MetaSource::User);
        }
        if let Some(is_paper) = patch.is_paper {
            paper.paper_override = is_paper;
        }
        let base_after = citekey::base_key(&paper.meta);
        let regenerate = match patch.cite_key {
            Some(Some(_)) => {
                paper.cite_key_pinned = true;
                false
            }
            Some(None) => {
                paper.cite_key_pinned = false;
                true
            }
            None => !paper.cite_key_pinned && base_before != base_after,
        };
        if let Some(key) = pinned_key {
            paper.cite_key = key;
        } else if regenerate {
            let meta = paper.meta.clone();
            let key = self.free_key(&meta, Some(index));
            self.papers[index].cite_key = key;
        }
        self.papers[index].touch();
        let id = self.papers[index].id.clone();
        if identifier_set {
            let (survivor, _) = self.auto_merge_around(&id);
            return self.paper_dto(&survivor);
        }
        self.paper_dto(&id)
    }
}

/// Add a paper to a list once, keeping first-seen order.
fn push_once(list: &mut Vec<usize>, index: usize) {
    if !list.contains(&index) {
        list.push(index);
    }
}

/// What citation resolution looks papers up by, for one query.
pub(super) struct CitationTables<'a> {
    pub(super) by_id: HashMap<CitationIdDto, usize>,
    pub(super) by_path: HashMap<&'a str, usize>,
    pub(super) titles: TitleIndex,
}

impl CitationTables<'_> {
    /// The paper one reference entry of paper `citing` names: by the first
    /// of its identifiers that names a paper (which may be `citing` itself,
    /// read as a self-citation, not an edge), else by the longest library
    /// title it contains (`TitleIndex::find`, never `citing`'s). An entry
    /// names at most one paper, and an identifier beats a title.
    pub(super) fn resolve_reference(&self, reference: &StoredRef, citing: usize) -> Option<usize> {
        reference
            .ids
            .iter()
            .find_map(|id| self.by_id.get(id).copied())
            .or_else(|| self.titles.find(reference, citing))
    }
}

fn target_for(paper: &PaperRecord) -> LookupTarget {
    LookupTarget {
        paper_id: paper.id.clone(),
        title: paper.search_title(),
        year: paper.year(),
        venue: paper.venue().map(str::to_owned),
        doi: paper.ids.doi.clone(),
        arxiv: paper.ids.arxiv.clone(),
        isbn: paper.ids.isbn.clone(),
        has_volume: paper.meta.volume.is_some(),
        has_pages: paper.meta.pages.is_some(),
        declined: paper.declined.clone(),
        arxiv_category: paper.arxiv_category.clone(),
    }
}

/// The batch plan over these papers, in the order given.
fn plan_lookup(papers: Vec<&PaperRecord>, now: u64) -> LookupPlan {
    let mut plan = LookupPlan::default();
    for paper in papers {
        match paper.batch_chain(now) {
            Some(chain) => plan.targets.push((chain, target_for(paper))),
            None => plan.skipped += 1,
        }
    }
    plan
}

fn user_field<T>(value: T) -> MetaField<T> {
    MetaField {
        value,
        source: MetaSource::User,
    }
}

/// A text field from a patch: absent leaves it, `null` or blank clears it.
fn set_text(field: &mut Option<MetaField<String>>, patch: Option<Option<String>>) {
    if let Some(value) = patch {
        *field = value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(user_field);
    }
}

/// Whether dropping this record would lose something the user did: an
/// edited field, an identifier they typed, or a key they pinned (and may
/// already have pasted into a manuscript).
pub(super) fn has_user_edit(paper: &PaperRecord) -> bool {
    if paper.cite_key_pinned
        || paper.paper_override.is_some()
        || !paper.merged_from.is_empty()
        || !paper.alias_keys.is_empty()
    {
        return true;
    }
    let is_user = |source: Option<MetaSource>| source == Some(MetaSource::User);
    let ids = &paper.ids;
    if is_user(ids.doi_source) || is_user(ids.arxiv_source) || is_user(ids.isbn_source) {
        return true;
    }
    let meta = &paper.meta;
    is_user(meta.title.as_ref().map(|f| f.source))
        || is_user(meta.authors.as_ref().map(|f| f.source))
        || is_user(meta.year.as_ref().map(|f| f.source))
        || is_user(meta.venue.as_ref().map(|f| f.source))
        || is_user(meta.volume.as_ref().map(|f| f.source))
        || is_user(meta.issue.as_ref().map(|f| f.source))
        || is_user(meta.pages.as_ref().map(|f| f.source))
        || is_user(meta.publisher.as_ref().map(|f| f.source))
        || is_user(meta.abstract_.as_ref().map(|f| f.source))
        || is_user(meta.kind.as_ref().map(|f| f.source))
}

/// First author's family, then year (else the year its references give),
/// then title; case- and accent-blind;
/// papers lacking a part sort after those that have it.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    no_author: bool,
    family: String,
    no_year: bool,
    year: i32,
    title: String,
}

impl SortKey {
    fn for_paper(paper: &PaperRecord) -> Self {
        let family = paper
            .authors()
            .first()
            .map(|author| fold(author.family.trim()))
            .unwrap_or_default();
        Self {
            no_author: family.is_empty(),
            family,
            no_year: paper.sort_year().is_none(),
            year: paper.sort_year().unwrap_or(0),
            title: fold(&paper.display_title()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::PaperKind;

    fn entry(path: &str, fingerprint: Option<&str>) -> LibraryEntry {
        LibraryEntry {
            path: PathBuf::from(path),
            mtime_ms: 1,
            size: 100,
            doi: None,
            arxiv: None,
            title: None,
            text_indexed: false,
            fingerprint: fingerprint.map(str::to_owned),
            info_title: None,
            info_author: None,
            info_year: None,
            cites: None,
            paper_verdict: None,
            paper_reason: None,
            paper_kind: None,
            xmp: None,
            isbn: None,
            arxiv_category: None,
            arxiv_version: None,
            first_page_authors: Vec::new(),
            first_page_title_confident: false,
            first_page_authors_confident: false,
            ocr_wanted: false,
            ocr_deferred: false,
            doi_source: None,
            arxiv_source: None,
        }
    }

    /// An entry the scan classified.
    fn classified(
        path: &str,
        fingerprint: &str,
        verdict: PapernessVerdict,
        reason: &str,
    ) -> LibraryEntry {
        let mut e = entry(path, Some(fingerprint));
        e.paper_verdict = Some(verdict);
        e.paper_reason = Some(reason.to_owned());
        e
    }

    fn new_store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        // Keep the directory alive for the store's lifetime by leaking it;
        // tests never save, so nothing lands there anyway.
        std::mem::forget(dir);
        store
    }

    fn meta_title(paper: &PaperRecord) -> (Option<&str>, Option<MetaSource>) {
        (
            paper.title(),
            paper.meta.title.as_ref().map(|field| field.source),
        )
    }

    fn page_authors(paper: &PaperRecord) -> (Vec<String>, Option<MetaSource>) {
        (
            paper
                .authors()
                .iter()
                .map(|a| format!("{} {}", a.given, a.family))
                .collect(),
            paper.meta.authors.as_ref().map(|field| field.source),
        )
    }

    fn with_page_authors(path: &str, confident: bool) -> LibraryEntry {
        let mut e = entry(path, Some("fp-page"));
        e.title = Some("Beyond Linear Response".into());
        e.first_page_title_confident = confident;
        e.first_page_authors_confident = confident;
        e.first_page_authors = vec![
            Author {
                given: "Adrianne".into(),
                family: "Zhong".into(),
            },
            Author {
                given: "Michael R.".into(),
                family: "DeWeese".into(),
            },
        ];
        e
    }

    /// Authors read off the printed first page reach the record at
    /// first-page rank, and a rescan that no longer finds them takes them
    /// back.
    #[test]
    fn first_page_authors_reach_the_record_and_are_retracted() {
        let mut store = new_store();
        let e = with_page_authors("/lib/zhong.pdf", true);
        assert!(store.reconcile_file(&e));
        let (names, source) = page_authors(&store.papers[0]);
        assert_eq!(names, ["Adrianne Zhong", "Michael R. DeWeese"]);
        assert_eq!(source, Some(MetaSource::FirstPage));

        let mut gone = e.clone();
        gone.first_page_authors.clear();
        gone.first_page_authors_confident = false;
        assert!(store.reconcile_file(&gone));
        assert_eq!(page_authors(&store.papers[0]), (Vec::new(), None));
    }

    /// The Info dictionary and XMP outrank the page, confident or not.
    #[test]
    fn info_and_xmp_authors_win_over_the_first_page() {
        let mut store = new_store();
        let mut e = with_page_authors("/lib/info.pdf", true);
        e.info_author = Some("A. Zhong; M. R. DeWeese".into());
        store.reconcile_file(&e);
        assert_eq!(page_authors(&store.papers[0]).1, Some(MetaSource::PdfInfo));

        let mut store = new_store();
        let mut e = with_page_authors("/lib/xmp.pdf", true);
        e.xmp = Some(crate::library::xmp::XmpFields {
            creators: vec!["Adrianne Zhong".into()],
            ..Default::default()
        });
        store.reconcile_file(&e);
        assert_eq!(
            page_authors(&store.papers[0]),
            (vec!["Adrianne Zhong".to_owned()], Some(MetaSource::Xmp))
        );
    }

    /// A reading the layout was unsure of fills a gap and nothing more:
    /// offered when the file names no authors, withheld (and taken back)
    /// when it does. Likewise an unsure title beside an Info title.
    #[test]
    fn unsure_first_page_values_only_fill_a_gap() {
        let mut store = new_store();
        let e = with_page_authors("/lib/unsure.pdf", false);
        store.reconcile_file(&e);
        assert_eq!(
            page_authors(&store.papers[0]).1,
            Some(MetaSource::FirstPage)
        );
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("Beyond Linear Response"), Some(MetaSource::FirstPage))
        );

        // The same file, now with an Info Author and Title: the page's
        // unsure values are withdrawn rather than kept underneath.
        let mut e = e.clone();
        e.info_author = Some("Adrianne Zhong".into());
        e.info_title = Some("Beyond linear response: the published title".into());
        store.reconcile_file(&e);
        assert_eq!(page_authors(&store.papers[0]).1, Some(MetaSource::PdfInfo));
        assert_eq!(meta_title(&store.papers[0]).1, Some(MetaSource::PdfInfo));
        // And when the Info dictionary drops them again, the page fills in.
        let mut e = e.clone();
        e.info_author = None;
        e.info_title = None;
        store.reconcile_file(&e);
        assert_eq!(
            page_authors(&store.papers[0]).1,
            Some(MetaSource::FirstPage)
        );
        assert_eq!(meta_title(&store.papers[0]).1, Some(MetaSource::FirstPage));
    }

    #[test]
    fn a_new_file_makes_a_paper_with_a_key() {
        let mut store = new_store();
        let mut e = entry("/lib/jarzynski1997.pdf", Some("fp1"));
        e.doi = Some("10.1103/PhysRevLett.78.2690".into());
        e.title = Some("Nonequilibrium Equality for Free Energy Differences".into());
        e.info_author = Some("C. Jarzynski".into());
        e.info_year = Some(1997);
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers.len(), 1);
        let paper = &store.papers[0];
        assert_eq!(
            paper.ids.doi.as_deref(),
            Some("10.1103/physrevlett.78.2690")
        );
        assert_eq!(paper.cite_key, "jarzynski1997nonequilibrium");
        assert_eq!(paper.files.len(), 1);
        assert_eq!(paper.files[0].name, "jarzynski1997.pdf");
        assert!(!paper.files[0].missing);
        // Reconciling the same entry again changes nothing.
        assert!(!store.reconcile_file(&e));
    }

    #[test]
    fn matches_by_identifier_then_fingerprint_then_path() {
        let mut store = new_store();
        let mut first = entry("/lib/a.pdf", Some("fp-a"));
        first.arxiv = Some("2401.12345v1".into());
        store.reconcile_file(&first);
        let id = store.papers[0].id.clone();

        // Same arXiv id, different bytes and path: the journal version.
        let mut journal = entry("/lib/journal/a-final.pdf", Some("fp-j"));
        journal.arxiv = Some("2401.12345v3".into());
        store.reconcile_file(&journal);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2, "two files, one paper");

        // Renamed: no identifier, same fingerprint. It becomes the primary
        // file, and the arXiv id stays: the journal version still carries
        // it.
        let renamed = entry("/lib/renamed.pdf", Some("fp-a"));
        store.reconcile_file(&renamed);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].id, id);
        assert_eq!(store.papers[0].files[0].path, "/lib/renamed.pdf");
        assert_eq!(store.papers[0].ids.arxiv.as_deref(), Some("2401.12345"));

        // Same path, new bytes, no identifier: still the same paper.
        let resaved = entry("/lib/renamed.pdf", Some("fp-a2"));
        store.reconcile_file(&resaved);
        assert_eq!(store.papers.len(), 1);

        // Nothing in common: a new paper.
        store.reconcile_file(&entry("/lib/other.pdf", Some("fp-o")));
        assert_eq!(store.papers.len(), 2);
    }

    #[test]
    fn a_missing_file_reattaches_by_fingerprint_and_drops_its_ghost() {
        let mut store = new_store();
        store.reconcile_file(&entry("/lib/a.pdf", Some("fp-a")));
        let id = store.papers[0].id.clone();
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&id, &project, true).unwrap();

        assert!(store.mark_missing(&HashSet::new()));
        assert!(store.papers[0].is_missing());
        assert_eq!(store.papers.len(), 1, "never deleted");
        assert_eq!(store.counts().unfiled, 0);

        store.reconcile_file(&entry("/lib/moved/a.pdf", Some("fp-a")));
        assert_eq!(store.papers.len(), 1);
        let paper = &store.papers[0];
        assert_eq!(paper.id, id);
        assert_eq!(paper.files.len(), 1, "the ghost row is replaced");
        assert_eq!(paper.files[0].path, "/lib/moved/a.pdf");
        assert!(!paper.is_missing());

        // A file with different bytes arriving by identifier keeps both rows.
        let mut store = new_store();
        let mut a = entry("/lib/a.pdf", Some("fp-a"));
        a.doi = Some("10.1/x".into());
        store.reconcile_file(&a);
        store.mark_missing(&HashSet::new());
        let mut b = entry("/lib/b.pdf", Some("fp-b"));
        b.doi = Some("10.1/x".into());
        store.reconcile_file(&b);
        assert_eq!(store.papers[0].files.len(), 2);
        assert!(store.papers[0].files.iter().any(|f| f.missing));
        assert!(!store.papers[0].is_missing());
    }

    #[test]
    fn provenance_never_loses_to_a_weaker_source() {
        let mut field: Option<MetaField<String>> = None;
        assert!(offer(
            &mut field,
            "first page".into(),
            MetaSource::FirstPage
        ));
        assert!(offer(&mut field, "info".into(), MetaSource::PdfInfo));
        assert!(!offer(
            &mut field,
            "first page again".into(),
            MetaSource::FirstPage
        ));
        assert_eq!(field.as_ref().unwrap().value, "info");
        // Equal rank replaces: a rescan of a changed file.
        assert!(offer(&mut field, "info 2".into(), MetaSource::PdfInfo));
        assert!(!offer(&mut field, "info 2".into(), MetaSource::PdfInfo));
        assert!(offer(&mut field, "crossref".into(), MetaSource::Crossref));
        assert!(offer(&mut field, "typed".into(), MetaSource::User));
        assert!(!offer(
            &mut field,
            "crossref 2".into(),
            MetaSource::Crossref
        ));
        assert!(!offer(&mut field, "info 3".into(), MetaSource::PdfInfo));
        assert!(offer(&mut field, "typed 2".into(), MetaSource::User));
        assert_eq!(field.as_ref().unwrap().value, "typed 2");
    }

    #[test]
    fn a_placeholder_before_the_target_is_dropped_without_misindexing() {
        let mut store = new_store();
        // P0: a.pdf scanned with no identifier yet.
        store.reconcile_file(&entry("/lib/a.pdf", Some("fp-a")));
        // P1: a paper known by DOI x. P2: a third, where a stale index lands.
        let mut b = entry("/lib/b.pdf", Some("fp-b"));
        b.doi = Some("10.1/x".into());
        store.reconcile_file(&b);
        store.reconcile_file(&entry("/lib/c.pdf", Some("fp-c")));
        assert_eq!(store.papers.len(), 3);

        // a.pdf is replaced by a copy carrying DOI x: it belongs to P1, and
        // P0 (a placeholder before P1 in the list) goes away.
        let mut replaced = entry("/lib/a.pdf", Some("fp-a2"));
        replaced.doi = Some("10.1/x".into());
        store.reconcile_file(&replaced);
        assert_eq!(store.papers.len(), 2, "the placeholder is gone");
        let owner = store
            .papers
            .iter()
            .find(|p| p.ids.doi.as_deref() == Some("10.1/x"))
            .expect("the DOI's paper");
        assert!(owner.files.iter().any(|f| f.path == "/lib/a.pdf"));
        assert!(owner.files.iter().any(|f| f.path == "/lib/b.pdf"));
        let other = store
            .papers
            .iter()
            .find(|p| p.files.iter().any(|f| f.path == "/lib/c.pdf"))
            .expect("the third paper");
        assert_eq!(other.files.len(), 1, "nothing landed on the wrong paper");

        // A placeholder with a pinned key is not a placeholder.
        let mut store = new_store();
        store.reconcile_file(&entry("/lib/d.pdf", Some("fp-d")));
        let id = store.papers[0].id.clone();
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    cite_key: Some(Some("mykey".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let mut e = entry("/lib/e.pdf", Some("fp-e"));
        e.doi = Some("10.1/y".into());
        store.reconcile_file(&e);
        let mut d2 = entry("/lib/d.pdf", Some("fp-d2"));
        d2.doi = Some("10.1/y".into());
        store.reconcile_file(&d2);
        assert_eq!(
            store.papers.len(),
            2,
            "the pinned record survives without files"
        );
        assert!(store
            .papers
            .iter()
            .any(|p| p.cite_key == "mykey" && p.files.is_empty()));
    }

    #[test]
    fn a_rescan_retracts_what_the_file_no_longer_says() {
        // The Info dictionary gave a title and a year; a stricter filter (or
        // a replaced file) takes them away. Both must fall back to what the
        // page says instead of outliving their evidence.
        let mut store = new_store();
        let mut e = entry("/lib/a.pdf", Some("fp"));
        e.title = Some("Entropy Production Fluctuation Theorem".into());
        e.info_title = Some("PII: 0022-0000(88)90028-1".into());
        e.info_year = Some(2003);
        store.reconcile_file(&e);
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("PII: 0022-0000(88)90028-1"), Some(MetaSource::PdfInfo))
        );
        assert_eq!(
            store.papers[0].meta.year.as_ref().map(|y| y.value),
            Some(2003)
        );
        let before = store.papers[0].cite_key.clone();

        e.info_title = None;
        e.info_year = None;
        assert!(store.reconcile_file(&e));
        assert_eq!(
            meta_title(&store.papers[0]),
            (
                Some("Entropy Production Fluctuation Theorem"),
                Some(MetaSource::FirstPage)
            )
        );
        assert!(store.papers[0].meta.year.is_none());
        assert_ne!(
            store.papers[0].cite_key, before,
            "an unpinned key follows the metadata"
        );

        // A second file without a Title does not retract what the primary
        // file supplies.
        let mut primary = entry("/lib/a.pdf", Some("fp"));
        primary.info_title = Some("The Real Title".into());
        primary.doi = Some("10.1/x".into());
        store.reconcile_file(&primary);
        let mut second = entry("/lib/a-journal.pdf", Some("fp2"));
        second.doi = Some("10.1/x".into());
        store.reconcile_file(&second);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("The Real Title"), Some(MetaSource::PdfInfo))
        );
    }

    #[test]
    fn a_rescan_keeps_the_users_title_and_info_beats_first_page() {
        let mut store = new_store();
        let mut e = entry("/lib/a.pdf", Some("fp"));
        e.title = Some("Running header of some journal".into());
        store.reconcile_file(&e);
        assert_eq!(
            meta_title(&store.papers[0]),
            (
                Some("Running header of some journal"),
                Some(MetaSource::FirstPage)
            )
        );
        e.info_title = Some("The Real Title".into());
        store.reconcile_file(&e);
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("The Real Title"), Some(MetaSource::PdfInfo))
        );

        let id = store.papers[0].id.clone();
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    title: Some(Some("What I Call It".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        e.info_title = Some("A Newer Info Title".into());
        store.reconcile_file(&e);
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("What I Call It"), Some(MetaSource::User))
        );
    }

    #[test]
    fn patch_semantics() {
        let mut store = new_store();
        let mut e = entry("/lib/a.pdf", Some("fp"));
        e.title = Some("Some Long Enough Title".into());
        store.reconcile_file(&e);
        store.reconcile_file(&entry("/lib/b.pdf", Some("fp-b")));
        let id = store.papers[0].id.clone();
        let other = store.papers[1].id.clone();

        // Set: source becomes user; untouched fields stay.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    authors: Some(Some(vec![Author {
                        family: "Crooks".into(),
                        given: "Gavin".into(),
                    }])),
                    year: Some(Some(1999)),
                    kind: Some(Some(PaperKind::Article)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(dto.meta.authors.as_ref().unwrap().source, MetaSource::User);
        assert_eq!(
            dto.meta.title.as_ref().unwrap().source,
            MetaSource::FirstPage
        );
        assert_eq!(
            dto.cite_key, "crooks1999long",
            "an unpinned key follows the metadata"
        );
        assert!(dto.incomplete.contains(&"venue".to_string()));
        assert!(!dto.incomplete.contains(&"year".to_string()));

        // Clear.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(None),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.meta.year.is_none());
        assert_eq!(dto.cite_key, "crooksndlong");

        // Pin.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    cite_key: Some(Some("crooks99".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.cite_key_pinned);
        assert_eq!(dto.cite_key, "crooks99");
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(Some(2000)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(dto.cite_key, "crooks99", "pinned keys do not follow");

        // A key another paper holds is refused, and nothing else changes.
        let error = store
            .apply_patch(
                &other,
                MetaPatchDto {
                    cite_key: Some(Some("crooks99".into())),
                    title: Some(Some("Should not land".into())),
                    ..MetaPatchDto::default()
                },
            )
            .expect_err("duplicate");
        assert!(error.to_string().contains("already used"), "{error}");
        assert!(store.papers[1].title().is_none());
        assert!(store
            .apply_patch(
                &other,
                MetaPatchDto {
                    cite_key: Some(Some("has space".into())),
                    ..MetaPatchDto::default()
                },
            )
            .is_err());

        // Unpin regenerates.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    cite_key: Some(None),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(!dto.cite_key_pinned);
        assert_eq!(dto.cite_key, "crooks2000long");

        // Identifiers normalize.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    doi: Some(Some("https://doi.org/10.1103/PhysRevE.60.2721".into())),
                    arxiv: Some(Some("cond-mat/9901352v4".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(dto.doi.as_deref(), Some("10.1103/physreve.60.2721"));
        assert_eq!(dto.arxiv.as_deref(), Some("cond-mat/9901352"));
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::User));
        assert_eq!(store.papers[0].ids.arxiv_source, Some(MetaSource::User));
        assert!(store.apply_patch("nope", MetaPatchDto::default()).is_err());
    }

    #[test]
    fn rows_display_authors_and_sort_by_family_year_title() {
        let mut store = new_store();
        for (path, authors, year, title) in [
            (
                "/l/1.pdf",
                "Zed Marchetti; A B; C D; E F",
                2013,
                "Hydrodynamics of soft active matter",
            ),
            (
                "/l/2.pdf",
                "Chris Jarzynski",
                1997,
                "Nonequilibrium equality",
            ),
            (
                "/l/3.pdf",
                "Denis Evans, E. Cohen and G. Morriss",
                1993,
                "Probability of second law violations",
            ),
            (
                "/l/4.pdf",
                "Gavin Crooks",
                1999,
                "Entropy production fluctuation theorem",
            ),
            (
                "/l/5.pdf",
                "Gavin Crooks",
                1998,
                "Nonequilibrium measurements",
            ),
            (
                "/l/6.pdf",
                "Étienne Fodor & Michael Cates",
                2016,
                "How far from equilibrium",
            ),
        ] {
            let mut e = entry(path, Some(path));
            e.info_author = Some(authors.into());
            e.info_year = Some(year);
            e.info_title = Some(title.into());
            store.reconcile_file(&e);
        }
        let mut no_author = entry("/l/7.pdf", Some("7"));
        no_author.title = Some("Anonymous but long enough".into());
        store.reconcile_file(&no_author);
        store.reconcile_file(&entry("/l/8 bare file.pdf", Some("8")));

        let list = store.paper_rows(&PaperFilterDto::default());
        assert_eq!(list.total, 8);
        let shown: Vec<(&str, &str)> = list
            .rows
            .iter()
            .map(|row| (row.authors.as_str(), row.title.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Crooks", "Nonequilibrium measurements"),
                ("Crooks", "Entropy production fluctuation theorem"),
                (
                    "Evans, Cohen & Morriss",
                    "Probability of second law violations"
                ),
                ("Fodor & Cates", "How far from equilibrium"),
                ("Jarzynski", "Nonequilibrium equality"),
                ("Marchetti et al.", "Hydrodynamics of soft active matter"),
                ("", "8 bare file.pdf"),
                ("", "Anonymous but long enough"),
            ]
        );
        let bare = list
            .rows
            .iter()
            .find(|row| row.title == "8 bare file.pdf")
            .unwrap();
        assert_eq!(bare.incomplete_count, 4);
        assert!(!bare.has_identifier);
    }

    #[test]
    fn query_matching_folds_case_and_diacritics_and_needs_every_term() {
        let mut store = new_store();
        let mut e = entry("/l/fodor.pdf", Some("f"));
        e.info_author = Some("Étienne Fodor & Michael Cates".into());
        e.info_year = Some(2016);
        e.info_title = Some("How far from equilibrium is active matter?".into());
        store.reconcile_file(&e);
        let mut other = entry("/l/other.pdf", Some("o"));
        other.info_title = Some("Something about Gärtner".into());
        store.reconcile_file(&other);

        let rows = |query: &str| {
            store
                .paper_rows(&PaperFilterDto {
                    query: query.into(),
                    ..PaperFilterDto::default()
                })
                .rows
                .len()
        };
        assert_eq!(rows("fodor"), 1);
        assert_eq!(rows("etienne"), 1);
        assert_eq!(rows("ÉTIENNE active"), 1);
        assert_eq!(rows("fodor 2016"), 1);
        assert_eq!(rows("fodor 2017"), 0);
        assert_eq!(rows("gartner"), 1);
        assert_eq!(rows("GÄRTNER"), 1);
        assert_eq!(rows("fodor2016far"), 1, "the cite key matches too");
        assert_eq!(rows("   "), 2);
        let list = store.paper_rows(&PaperFilterDto {
            query: "fodor".into(),
            ..PaperFilterDto::default()
        });
        assert_eq!(list.total, 2, "total counts the selection, not the matches");
    }

    #[test]
    fn info_author_strings_parse_in_their_common_forms() {
        let names = |raw: &str| -> Vec<String> {
            parse_info_authors(raw)
                .into_iter()
                .map(|a| format!("{}|{}", a.given, a.family))
                .collect()
        };
        assert_eq!(names("C. Jarzynski"), vec!["C.|Jarzynski"]);
        assert_eq!(
            names("Denis J. Evans, E. G. D. Cohen and G. P. Morriss"),
            vec!["Denis J.|Evans", "E. G. D.|Cohen", "G. P.|Morriss"]
        );
        assert_eq!(
            names("Jarzynski, C., Crooks, G."),
            vec!["C.|Jarzynski", "G.|Crooks"]
        );
        assert_eq!(names("Smith; Jones"), vec!["|Smith", "|Jones"]);
        assert!(names("Administrator").is_empty());
        assert!(names("   ").is_empty());
        assert!(names("1234").is_empty());
    }

    #[test]
    fn identifiers_normalize() {
        assert_eq!(
            normalize_doi(" DOI:10.1000/ABC ").as_deref(),
            Some("10.1000/abc")
        );
        assert_eq!(normalize_doi("weird").as_deref(), Some("weird"));
        assert_eq!(normalize_doi("  ").as_deref(), None);
        assert_eq!(
            normalize_arxiv("arXiv:2401.12345v2").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            normalize_arxiv("2401.12345v2").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            normalize_arxiv("hep-th/9901001v1").as_deref(),
            Some("hep-th/9901001")
        );
        assert_eq!(normalize_arxiv("oddball-v3").as_deref(), Some("oddball-"));
    }

    fn candidate(
        source: MetaSource,
        title: &str,
        authors: &[(&str, &str)],
        year: Option<i32>,
    ) -> LookupCandidateDto {
        LookupCandidateDto {
            source,
            title: title.to_owned(),
            authors: authors
                .iter()
                .map(|(family, given)| Author {
                    family: (*family).to_owned(),
                    given: (*given).to_owned(),
                })
                .collect(),
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

    fn source_of<T>(field: &Option<MetaField<T>>) -> Option<MetaSource> {
        field.as_ref().map(|f| f.source)
    }

    #[test]
    fn a_candidate_is_offered_at_its_rank_and_fills_identifiers_once() {
        let mut store = new_store();
        let mut e = entry("/lib/a.pdf", Some("fp"));
        e.title = Some("Running header of the journal".into());
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();

        // A lookup beats the first-page guess, fills the gaps, sets the
        // identifier the paper lacked, and the unpinned key follows.
        let mut crossref = candidate(
            MetaSource::Crossref,
            "Nonequilibrium Equality for Free Energy Differences",
            &[("Jarzynski", "C.")],
            Some(1997),
        );
        crossref.doi = Some("https://doi.org/10.1103/PhysRevLett.78.2690".into());
        crossref.venue = Some("  Physical Review Letters ".into());
        crossref.kind = Some(PaperKind::Article);
        let dto = store.apply_candidate(&id, &crossref).unwrap();
        assert_eq!(
            meta_title(&store.papers[0]),
            (
                Some("Nonequilibrium Equality for Free Energy Differences"),
                Some(MetaSource::Crossref)
            )
        );
        assert_eq!(source_of(&dto.meta.authors), Some(MetaSource::Crossref));
        assert_eq!(dto.meta.year.as_ref().map(|y| y.value), Some(1997));
        assert_eq!(
            dto.meta.venue.as_ref().map(|v| v.value.as_str()),
            Some("Physical Review Letters")
        );
        assert_eq!(dto.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::Crossref));
        assert_eq!(dto.cite_key, "jarzynski1997nonequilibrium");
        assert!(dto.incomplete.is_empty());

        // A user edit survives a later lookup, and a weaker lookup never
        // replaces a stronger one; what the paper lacked still lands.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    title: Some(Some("Entropy Production Theorem".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(store.papers[0].cite_key, "jarzynski1997entropy");
        let mut arxiv = candidate(
            MetaSource::Arxiv,
            "Preprint Title",
            &[("Jarzynski", "Christopher")],
            Some(1996),
        );
        arxiv.arxiv = Some("cond-mat/9610209v2".into());
        arxiv.doi = Some("10.1/some-other".into());
        arxiv.abstract_ = Some("An equality is derived.".into());
        let dto = store.apply_candidate(&id, &arxiv).unwrap();
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("Entropy Production Theorem"), Some(MetaSource::User))
        );
        assert_eq!(
            dto.meta.year.as_ref().map(|y| (y.value, y.source)),
            Some((1997, MetaSource::Crossref))
        );
        assert_eq!(
            dto.meta.authors.as_ref().map(|a| a.value[0].given.as_str()),
            Some("C.")
        );
        assert_eq!(source_of(&dto.meta.abstract_), Some(MetaSource::Arxiv));
        assert_eq!(
            dto.arxiv.as_deref(),
            Some("cond-mat/9610209"),
            "set because absent"
        );
        assert_eq!(store.papers[0].ids.arxiv_source, Some(MetaSource::Arxiv));
        assert_eq!(
            dto.doi.as_deref(),
            Some("10.1103/physrevlett.78.2690"),
            "kept because present"
        );
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::Crossref));
        assert_eq!(dto.cite_key, "jarzynski1997entropy");

        // A pinned key stays put whatever the lookup changes.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    cite_key: Some(Some("jarz97".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let again = candidate(
            MetaSource::Crossref,
            "Whatever",
            &[("Crooks", "G.")],
            Some(1998),
        );
        let dto = store.apply_candidate(&id, &again).unwrap();
        assert_eq!(
            dto.meta.year.as_ref().map(|y| y.value),
            Some(1998),
            "equal rank replaces"
        );
        assert_eq!(dto.cite_key, "jarz97");
        assert!(dto.cite_key_pinned);

        // Empty candidate fields offer nothing.
        let blank = candidate(MetaSource::Crossref, "  ", &[("", " ")], None);
        let dto = store.apply_candidate(&id, &blank).unwrap();
        assert_eq!(
            dto.meta
                .authors
                .as_ref()
                .map(|a| a.value[0].family.as_str()),
            Some("Crooks")
        );
        assert!(store.apply_candidate("nope", &blank).is_err());
    }

    #[test]
    fn a_batch_asks_by_identifier_where_one_can_add_something_and_by_title_otherwise() {
        let mut store = new_store();
        // No identifier but a title: the title chain.
        let mut plain = entry("/l/plain.pdf", Some("p"));
        plain.title = Some("A long enough title line".into());
        store.reconcile_file(&plain);
        // A DOI and only a first-page guess: the identifier chain.
        let mut guessed = entry("/l/guessed.pdf", Some("g"));
        guessed.doi = Some("10.1/g".into());
        guessed.title = Some("Guessed running header".into());
        store.reconcile_file(&guessed);
        // Author and year from the Info dictionary, title too: still a
        // target, since no lookup has confirmed the title.
        let mut info = entry("/l/info.pdf", Some("i"));
        info.arxiv = Some("2401.00001".into());
        info.info_author = Some("A. Author".into());
        info.info_year = Some(2024);
        info.info_title = Some("Info Title".into());
        store.reconcile_file(&info);
        // Already looked up, lately, and complete: nothing to gain.
        let mut done = entry("/l/done.pdf", Some("d"));
        done.doi = Some("10.1/d".into());
        store.reconcile_file(&done);
        let ids: Vec<String> = store.papers.iter().map(|p| p.id.clone()).collect();
        let (plain_id, guessed_id, info_id, done_id) = (
            ids[0].clone(),
            ids[1].clone(),
            ids[2].clone(),
            ids[3].clone(),
        );
        let mut filled = candidate(
            MetaSource::Crossref,
            "Done Title",
            &[("Done", "D.")],
            Some(2020),
        );
        filled.venue = Some("Venue".into());
        store.apply_candidate(&done_id, &filled).unwrap();
        store.mark_looked_up(&done_id).unwrap();
        let project = store.create_project("P").unwrap().id;
        for id in &ids {
            store.set_membership(id, &project, true).unwrap();
        }

        let now = crate::library::store::parse_rfc3339(&now_rfc3339()).unwrap();
        let plan = store.plan_project_lookup(&project, now).unwrap();
        assert_eq!(plan.skipped, 1);
        let mut planned: Vec<(ChainKind, &str)> = plan
            .targets
            .iter()
            .map(|(chain, target)| (*chain, target.paper_id.as_str()))
            .collect();
        planned.sort_by_key(|(_, id)| id.to_string());
        let mut expected = vec![
            (ChainKind::Title, plain_id.as_str()),
            (ChainKind::Identifier, guessed_id.as_str()),
            (ChainKind::Identifier, info_id.as_str()),
        ];
        expected.sort_by_key(|(_, id)| id.to_string());
        assert_eq!(planned, expected);
        assert!(store.plan_project_lookup("nope", now).is_err());
        // The whole library: the same, with nobody left out.
        assert_eq!(store.plan_library_lookup(now).targets.len(), 3);
        assert_eq!(store.plan_library_lookup(now).skipped, 1);
        // A month on, the finished one is asked again.
        let later = now + LOOKUP_FRESH.as_secs();
        assert_eq!(store.plan_library_lookup(later).skipped, 0);

        // What a target carries: the title, or the file's stem, and what
        // the paper already has.
        let target = store.lookup_target(&guessed_id).unwrap();
        assert_eq!(target.title.as_deref(), Some("Guessed running header"));
        assert_eq!(target.doi.as_deref(), Some("10.1/g"));
        assert!(!target.has_pages && !target.has_volume && target.venue.is_none());
        let target = store.lookup_target(&done_id).unwrap();
        assert_eq!(target.year, Some(2020));
        assert_eq!(target.venue.as_deref(), Some("Venue"));
        let bare = entry("/l/bare file.pdf", Some("b"));
        store.reconcile_file(&bare);
        let bare_id = store.papers.last().unwrap().id.clone();
        assert_eq!(
            store.lookup_target(&bare_id).unwrap().title.as_deref(),
            Some("bare file")
        );
    }

    #[test]
    fn batch_classification_skips_non_papers_parked_papers_and_fresh_complete_ones() {
        let now = 1_800_000_000;
        let fresh = crate::library::store::format_rfc3339(now - 60);
        let stale = crate::library::store::format_rfc3339(now - LOOKUP_FRESH.as_secs());
        let mut paper = PaperRecord::new();
        paper.files.push(PaperFile {
            path: "/l/a.pdf".into(),
            name: "a.pdf".into(),
            size: 1,
            mtime_ms: 1,
            fingerprint: None,
            missing: false,
            ..Default::default()
        });
        // Never looked up: by title, since the file name is a title to try.
        assert_eq!(paper.batch_chain(now), Some(ChainKind::Title));
        // Not a paper: never.
        paper.paper_override = Some(false);
        assert_eq!(paper.batch_chain(now), None);
        paper.paper_override = None;
        // Parked candidates: the user is being asked.
        paper.candidates = vec![candidate(MetaSource::Openalex, "x", &[], None)];
        assert_eq!(paper.batch_chain(now), None);
        paper.candidates.clear();
        // Looked up lately and incomplete: again (the cache makes it cheap).
        paper.looked_up_at = Some(fresh.clone());
        assert_eq!(paper.batch_chain(now), Some(ChainKind::Title));
        // Looked up lately and complete: left alone until it goes stale.
        paper.meta.title = Some(MetaField {
            value: "T".into(),
            source: MetaSource::User,
        });
        paper.meta.authors = Some(MetaField {
            value: vec![Author {
                family: "A".into(),
                given: "".into(),
            }],
            source: MetaSource::User,
        });
        paper.meta.year = Some(MetaField {
            value: 2020,
            source: MetaSource::User,
        });
        paper.meta.venue = Some(MetaField {
            value: "V".into(),
            source: MetaSource::User,
        });
        assert!(paper.incomplete().is_empty());
        assert_eq!(paper.batch_chain(now), None);
        paper.looked_up_at = Some(stale.clone());
        assert_eq!(paper.batch_chain(now), Some(ChainKind::Title));
        paper.looked_up_at = Some("garbage".into());
        assert_eq!(
            paper.batch_chain(now),
            Some(ChainKind::Title),
            "unreadable is stale"
        );
        // An ISBN is an identifier for the chain's purposes.
        paper.looked_up_at = Some(fresh.clone());
        paper.ids.isbn = Some("9780306406157".into());
        assert_eq!(paper.batch_chain(now), None, "fresh and complete");
        paper.looked_up_at = Some(stale.clone());
        assert_eq!(paper.batch_chain(now), Some(ChainKind::Identifier));
        // With a DOI: the identifier chain whenever a lookup could add
        // something, even if looked up yesterday.
        paper.looked_up_at = Some(fresh);
        paper.ids.doi = Some("10.1/x".into());
        assert_eq!(paper.batch_chain(now), None);
        paper.meta.year = None;
        assert_eq!(paper.batch_chain(now), Some(ChainKind::Identifier));
        // A record with nothing to go on at all is skipped.
        let empty = PaperRecord::new();
        assert_eq!(empty.search_title(), None);
        assert_eq!(empty.batch_chain(now), None);
    }

    #[test]
    fn marking_looked_up_stamps_now_and_goes_stale_after_a_month() {
        let mut store = new_store();
        store.reconcile_file(&entry("/l/a.pdf", Some("a")));
        let id = store.papers[0].id.clone();
        assert!(store.papers[0].lookup_stale(0));
        store.mark_looked_up(&id).unwrap();
        let at =
            crate::library::store::parse_rfc3339(store.papers[0].looked_up_at.as_deref().unwrap())
                .unwrap();
        assert!(!store.papers[0].lookup_stale(at));
        assert!(!store.papers[0].lookup_stale(at + LOOKUP_FRESH.as_secs() - 1));
        assert!(store.papers[0].lookup_stale(at + LOOKUP_FRESH.as_secs()));
        assert!(
            store.papers[0].lookup_stale(at - 1),
            "a stamp from the future"
        );
        assert!(store.mark_looked_up("nope").is_err());
    }

    #[test]
    fn parking_caps_at_five_and_discarding_remembers_what_was_declined() {
        let mut store = new_store();
        let mut e = entry("/l/a.pdf", Some("a"));
        e.title = Some("Some paper title here".into());
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();
        let many: Vec<LookupCandidateDto> = (0..7)
            .map(|i| {
                let mut c = candidate(MetaSource::Openalex, &format!("Match {i}"), &[], None);
                c.doi = Some(format!("10.1/{i}"));
                c
            })
            .collect();
        assert_eq!(store.park_candidates(&id, many.clone()).unwrap(), 5);
        assert_eq!(store.papers[0].candidates.len(), 5);
        assert_eq!(store.paper_dto(&id).unwrap().candidates[4].title, "Match 4");
        assert!(store.park_candidates("nope", many.clone()).is_err());

        // Parking the same again changes nothing; parking others replaces.
        let before = store.papers[0].updated_at.clone();
        store.park_candidates(&id, many[..5].to_vec()).unwrap();
        assert_eq!(store.papers[0].updated_at, before);
        assert_eq!(store.park_candidates(&id, many[5..].to_vec()).unwrap(), 2);
        assert_eq!(store.papers[0].candidates.len(), 2);

        store.discard_candidates(&id).unwrap();
        assert!(store.papers[0].candidates.is_empty());
        assert_eq!(
            store.papers[0].declined,
            vec!["doi:10.1/5".to_string(), "doi:10.1/6".to_string()]
        );
        // Declining again does not repeat; the target carries the list.
        store.park_candidates(&id, many[5..].to_vec()).unwrap();
        store.discard_candidates(&id).unwrap();
        assert_eq!(store.papers[0].declined.len(), 2);
        assert_eq!(store.lookup_target(&id).unwrap().declined.len(), 2);
        // Applying one clears the parked list without declining anything.
        store.park_candidates(&id, many[..2].to_vec()).unwrap();
        store.apply_candidate(&id, &many[0]).unwrap();
        assert!(store.papers[0].candidates.is_empty());
        assert_eq!(store.papers[0].declined.len(), 2);
    }

    #[test]
    fn citation_links_union_the_networks_edges_with_the_sidecars() {
        use CitationIdDto::{ArXiv, Doi};
        let mut store = new_store();
        let mut a = entry("/l/a.pdf", Some("a"));
        a.doi = Some("10.1/a".into());
        a.arxiv = Some("2401.0000a".into());
        let mut b = entry("/l/b.pdf", Some("b"));
        b.arxiv = Some("2401.00002".into());
        let mut c = entry("/l/c.pdf", Some("c"));
        c.doi = Some("10.1/c".into());
        let mut d = entry("/l/d.pdf", Some("d"));
        d.doi = Some("10.1/d".into());
        for e in [&a, &b, &c, &d] {
            store.reconcile_file(e);
        }
        let ids: Vec<String> = store.papers.iter().map(|p| p.id.clone()).collect();
        // The sidecar: a cites b.
        let mut entries = vec![a.clone(), b.clone(), c.clone(), d.clone()];
        entries[0].cites = Some(vec![ArXiv("2401.00002".into())]);
        let no_network = NetworkEdges::default();
        let links = store
            .citation_links(&ids[0], &entries, &DocRefs::new(), &no_network)
            .unwrap();
        assert_eq!(links.cites.len(), 1);
        assert!(links.cited_by.is_empty());
        assert_eq!(links.unresolved, 0);

        // The network: a cites b (again), c and a stranger, by a's DOI; d
        // is cited by a, by a's arXiv id; c's references name a.
        let mut network = NetworkEdges::default();
        network.refs.insert(
            Doi("10.1/a".into()),
            vec![
                ArXiv("2401.00002".into()),
                Doi("10.1/c".into()),
                Doi("10.1/zzz".into()),
                Doi("10.1/a".into()), // itself, which never counts
            ],
        );
        network.cites.insert(
            ArXiv("2401.0000a".into()),
            vec![
                Doi("10.1/d".into()),
                Doi("10.1/a".into()),
                Doi("10.1/nobody".into()),
            ],
        );
        network
            .refs
            .insert(Doi("10.1/c".into()), vec![Doi("10.1/a".into())]);
        let links = store
            .citation_links(&ids[0], &entries, &DocRefs::new(), &network)
            .unwrap();
        let cited: Vec<&str> = links.cites.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            cited,
            vec![ids[1].as_str(), ids[2].as_str()],
            "b once, c from the network"
        );
        let by: Vec<&str> = links.cited_by.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            by,
            vec![ids[2].as_str(), ids[3].as_str()],
            "c through its references, d through a's citations"
        );
        assert_eq!(
            links.unresolved, 1,
            "the stranger a cites; not the stranger citing a"
        );

        // From b's side: cited by a through both the sidecar and the network, once.
        let links = store
            .citation_links(&ids[1], &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(links.cited_by.len(), 1);
        assert!(links.cites.is_empty());
        // From c's side: cites a (network), cited by a (network).
        let links = store
            .citation_links(&ids[2], &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(links.cites.len(), 1);
        assert_eq!(links.cited_by.len(), 1);
    }

    /// The bug this was built for: "Higher-order response theory in
    /// optimal stochastic thermodynamics" cites, as [48], a paper in the
    /// library that has no DOI yet, by journal reference and title only.
    #[test]
    fn a_reference_without_identifiers_resolves_by_its_title() {
        use super::super::references::{ProvidedRef, RefsCache, RefsProvider, StoreOutcome};
        use std::sync::Arc;
        let mut store = new_store();
        let mut citing = entry("/l/2512.24540v3.pdf", Some("citing"));
        citing.arxiv = Some("2512.24540".into());
        citing.title =
            Some("Higher-order response theory in optimal stochastic thermodynamics".into());
        citing.info_year = Some(2025);
        let mut zhong = entry("/l/2404.01286.pdf", Some("zhong"));
        zhong.arxiv = Some("2404.01286".into());
        zhong.title = Some(
            "Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport"
                .into(),
        );
        zhong.info_year = Some(2024);
        let mut generic = entry("/l/intro.pdf", Some("generic"));
        generic.title = Some("Optimal transport".into());
        let mut other = entry("/l/other.pdf", Some("other"));
        other.doi = Some("10.1000/other".into());
        other.title =
            Some("Thermodynamic geometry of minimum-dissipation driven barrier crossing".into());
        other.info_year = Some(2016);
        for e in [&citing, &zhong, &generic, &other] {
            store.reconcile_file(e);
        }
        let id_of = |path: &str| {
            store
                .papers
                .iter()
                .find(|p| p.files.iter().any(|f| f.path == path))
                .unwrap()
                .id
                .clone()
        };
        let (citing_id, zhong_id, other_id) = (
            id_of("/l/2512.24540v3.pdf"),
            id_of("/l/2404.01286.pdf"),
            id_of("/l/other.pdf"),
        );
        // What the References extension's parser returns for this list
        // (its tests read the same text), stored the way the extension's
        // answer is: through the provider's token, from the text sidecar.
        let dir = tempfile::tempdir().expect("dir");
        let text = "Body text cites [1, 2] and [3].";
        crate::search::librarytext::write_sidecar(
            dir.path(),
            &citing.path,
            citing.mtime_ms,
            citing.size,
            &[(0, text.to_owned())],
        )
        .expect("text sidecar");
        let provided = [
            (1, "A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).", 2024),
            (2, "T. Schmiedl and U. Seifert, Optimal finite-time processes in stochastic thermodynamics, Phys. Rev. Lett. 98, 108301 (2007).", 2007),
            (3, "Self, Higher-order response theory in optimal stochastic thermodynamics (2025).", 2025),
            (4, "X. Y, Optimal transport, Springer (2024).", 2024),
            (5, "D. A. Sivak and G. E. Crooks, Thermodynamic geometry of minimum-dissipation driven barrier crossing, doi:10.1000/other (2016).", 2016),
            (6, "Someone, Beyond linear response: equivalence between thermodynamic geometry and optimal transport, doi:10.1000/other (2024).", 2024),
        ]
        .map(|(label, text, year)| ProvidedRef {
            label: Some(label.to_string()),
            text: text.to_owned(),
            year: Some(year),
        });
        let mut entries = vec![
            citing.clone(),
            zhong.clone(),
            generic.clone(),
            other.clone(),
        ];
        entries[0].cites = Some(vec![CitationIdDto::Doi("10.1000/other".into())]);
        // The text pass has written the citing document's text.
        entries[0].text_indexed = true;
        for e in &mut entries[1..] {
            e.cites = Some(Vec::new());
        }
        let mut cache = RefsCache::default();
        let mut provider = RefsProvider::default();
        provider.set_active(Some("dev.speedyf.references@4.0.0".into()));
        let pending = provider.pending(
            dir.path(),
            &entries[..1],
            &mut cache,
            8,
            std::time::Instant::now(),
        );
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, text);
        let stored = provider
            .store(
                dir.path(),
                &pending[0].token,
                |_| Some(entries[0].clone()),
                Some(&provided),
                &mut cache,
            )
            .expect("store");
        assert_eq!(stored, StoreOutcome::Stored(6));
        let refs = cache.load(dir.path(), &entries, provider.active());
        assert_eq!(refs[&PathBuf::from("/l/2512.24540v3.pdf")].len(), 6);
        assert!(
            cache.load(dir.path(), &entries, None).is_empty(),
            "nothing is read with the provider off"
        );
        let network = NetworkEdges::default();

        let links = store
            .citation_links(&citing_id, &entries, &refs, &network)
            .unwrap();
        let mut cited: Vec<&str> = links.cites.iter().map(|r| r.id.as_str()).collect();
        cited.sort();
        let mut expected = vec![zhong_id.as_str(), other_id.as_str()];
        expected.sort();
        assert_eq!(
            cited, expected,
            "[48] by title, [5] by DOI; never itself or a short title"
        );
        // [2] names nothing here, [4]'s title is too generic, and [3]'s
        // title is the paper's own, which is never matched; [6]'s DOI
        // beats its title.
        assert_eq!(links.unresolved, 3);
        let back = store
            .citation_links(&zhong_id, &entries, &refs, &network)
            .unwrap();
        assert_eq!(back.cited_by.len(), 1);
        assert_eq!(back.cited_by[0].id, citing_id);

        // The identifier wins: [6] carries the Zhong title but other's DOI,
        // so with [1] gone nothing points at Zhong.
        let mut only_six = DocRefs::new();
        let six: Vec<_> = refs[&PathBuf::from("/l/2512.24540v3.pdf")]
            .iter()
            .filter(|r| r.label.as_deref() == Some("6"))
            .cloned()
            .collect();
        only_six.insert(PathBuf::from("/l/2512.24540v3.pdf"), Arc::new(six));
        let links = store
            .citation_links(&citing_id, &entries, &only_six, &network)
            .unwrap();
        let cited: Vec<&str> = links.cites.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(cited, [other_id.as_str()]);
        assert_eq!(links.unresolved, 0);

        // The graph draws the same edges and counts the same entries.
        let graph = store
            .citation_graph(None, &entries, &refs, &network)
            .unwrap();
        let mut edges: Vec<(String, String)> = graph
            .edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        edges.sort();
        let mut want = vec![
            (citing_id.clone(), zhong_id.clone()),
            (citing_id.clone(), other_id.clone()),
        ];
        want.sort();
        assert_eq!(edges, want);
        assert_eq!(graph.external, 3);
        assert_eq!(graph.unindexed, 0);

        // Without the refs sidecar, the old behaviour: identifiers only.
        let links = store
            .citation_links(&citing_id, &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(links.cites.len(), 1);
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.external, 0);
    }

    /// With the reference provider off, a document's refs sidecar is not
    /// read: the edges its identifiers make - DOI, arXiv, and the DOIs APS
    /// journal references stand for - are all drawn, and one only its
    /// title could make is not.
    #[test]
    fn with_no_provider_identifier_edges_stay_and_title_edges_go() {
        use super::super::references::{ProvidedRef, RefsCache, RefsProvider};
        let dir = tempfile::tempdir().expect("dir");
        let mut store = new_store();
        let citing = entry("/l/citing.pdf", Some("citing"));
        let mut b = entry("/l/b.pdf", Some("b"));
        b.arxiv = Some("2401.00002".into());
        let mut c = entry("/l/c.pdf", Some("c"));
        c.doi = Some("10.1000/c".into());
        let mut aps = entry("/l/aps.pdf", Some("aps"));
        aps.doi = Some("10.1103/physrevlett.133.057102".into());
        let mut titled = entry("/l/titled.pdf", Some("titled"));
        titled.title =
            Some("Thermodynamic geometry of minimum-dissipation driven barrier crossing".into());
        for e in [&citing, &b, &c, &aps, &titled] {
            store.reconcile_file(e);
        }
        let text = "References [1] B, arXiv:2401.00002. [2] C, doi:10.1000/c. \
            [3] Z, Phys. Rev. Lett. 133, 057102 (2024). \
            [4] D. A. Sivak and G. E. Crooks, Thermodynamic geometry of minimum-dissipation driven barrier crossing (2016).";
        let mut entries = vec![citing.clone(), b, c, aps, titled];
        entries[0].cites = Some(super::super::record_citations(&entries[0], text));
        // The text pass has written the citing document's text.
        entries[0].text_indexed = true;
        for e in &mut entries[1..] {
            e.cites = Some(Vec::new());
        }
        crate::search::librarytext::write_sidecar(
            dir.path(),
            &citing.path,
            citing.mtime_ms,
            citing.size,
            &[(0, text.to_owned())],
        )
        .expect("text sidecar");
        let mut cache = RefsCache::default();
        let mut provider = RefsProvider::default();
        provider.set_active(Some("dev.speedyf.references@4.0.0".into()));
        let token = provider.pending(
            dir.path(),
            &entries,
            &mut cache,
            8,
            std::time::Instant::now(),
        )[0]
        .token
        .clone();
        let provided = [ProvidedRef {
            label: Some("4".into()),
            text: "D. A. Sivak and G. E. Crooks, Thermodynamic geometry of minimum-dissipation driven barrier crossing (2016).".into(),
            year: Some(2016),
        }];
        provider
            .store(
                dir.path(),
                &token,
                |_| Some(entries[0].clone()),
                Some(&provided),
                &mut cache,
            )
            .expect("store");
        let network = NetworkEdges::default();
        let edges = |refs: &DocRefs| {
            store
                .citation_graph(None, &entries, refs, &network)
                .unwrap()
                .edges
                .len()
        };
        provider.set_active(None);
        let off = cache.load(dir.path(), &entries, provider.active());
        assert!(off.is_empty());
        assert_eq!(edges(&off), 3, "arXiv, DOI and APS journal reference");
        provider.set_active(Some("dev.speedyf.references@4.0.0".into()));
        let on = cache.load(dir.path(), &entries, provider.active());
        assert_eq!(edges(&on), 4, "and the title, with the provider back");
    }

    #[test]
    fn the_graph_draws_edges_between_nodes_and_counts_the_rest() {
        use CitationIdDto::{ArXiv, Doi};
        let mut store = new_store();
        let mut a = entry("/l/a.pdf", Some("a"));
        a.doi = Some("10.1/a".into());
        let mut b = entry("/l/b.pdf", Some("b"));
        b.arxiv = Some("2401.00002".into());
        let mut c = entry("/l/c.pdf", Some("c"));
        c.doi = Some("10.1/c".into());
        let mut d = entry("/l/d.pdf", Some("d"));
        d.doi = Some("10.1/d".into());
        for e in [&a, &b, &c, &d] {
            store.reconcile_file(e);
        }
        let ids: Vec<String> = store.papers.iter().map(|p| p.id.clone()).collect();
        // Compared as sets: the edges come out in list order, which the
        // test does not care about (B leads it, its arXiv id giving it a
        // year the others lack).
        let edges_of = |graph: &GraphDto| -> Vec<(String, String)> {
            let mut edges: Vec<(String, String)> = graph
                .edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone()))
                .collect();
            edges.sort();
            edges
        };
        let edges = |pairs: &[(usize, usize)]| -> Vec<(String, String)> {
            let mut edges: Vec<(String, String)> = pairs
                .iter()
                .map(|&(from, to)| (ids[from].clone(), ids[to].clone()))
                .collect();
            edges.sort();
            edges
        };
        let no_network = NetworkEdges::default();

        // The sidecars: A -> B (and a stranger, and itself), B -> C, C -> A
        // twice, D nothing - but read.
        let mut entries = vec![a.clone(), b.clone(), c.clone(), d.clone()];
        entries[0].cites = Some(vec![
            ArXiv("2401.00002".into()),
            Doi("10.1/zzz".into()),
            Doi("10.1/a".into()),
        ]);
        entries[1].cites = Some(vec![Doi("10.1/c".into())]);
        entries[2].cites = Some(vec![Doi("10.1/a".into()), Doi("10.1/a".into())]);
        entries[3].cites = Some(Vec::new());
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &no_network)
            .unwrap();
        let mut nodes: Vec<&str> = graph.nodes.iter().map(|row| row.id.as_str()).collect();
        nodes.sort();
        let mut all: Vec<&str> = ids.iter().map(String::as_str).collect();
        all.sort();
        assert_eq!(nodes, all);
        assert_eq!(
            edges_of(&graph),
            edges(&[(0, 1), (1, 2), (2, 0)]),
            "once each, never to itself"
        );
        assert_eq!(graph.external, 1, "the stranger");
        assert_eq!(graph.unindexed, 0);

        // A node whose cites were never read is unindexed, not uncited.
        entries[3].cites = None;
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &no_network)
            .unwrap();
        assert_eq!(graph.unindexed, 1);
        assert_eq!(graph.edges.len(), 3);

        // The network cache: D's references name A and a stranger; C is
        // said to be cited by D and by nobody known.
        let mut network = NetworkEdges::default();
        network.refs.insert(
            Doi("10.1/d".into()),
            vec![Doi("10.1/a".into()), Doi("10.1/yyy".into())],
        );
        network.cites.insert(
            Doi("10.1/c".into()),
            vec![Doi("10.1/d".into()), Doi("10.1/nobody".into())],
        );
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(
            edges_of(&graph),
            edges(&[(0, 1), (1, 2), (2, 0), (3, 0), (3, 2)]),
            "D → A from its references, D → C from C's citers"
        );
        assert_eq!(
            graph.external, 2,
            "the two strangers cited; not the stranger citing"
        );
        assert_eq!(graph.unindexed, 1, "the network does not index a file");

        // A project limits the nodes: an edge to a non-member becomes
        // external, and a non-paper member is still a node.
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&ids[0], &project, true).unwrap();
        store.set_membership(&ids[1], &project, true).unwrap();
        store
            .apply_patch(
                &ids[1],
                MetaPatchDto {
                    is_paper: Some(Some(false)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let graph = store
            .citation_graph(Some(&project), &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(edges_of(&graph), edges(&[(0, 1)]));
        assert_eq!(graph.external, 2, "the stranger A cites, and C");
        assert_eq!(graph.unindexed, 0);
        assert!(store
            .citation_graph(Some("nope"), &entries, &DocRefs::new(), &network)
            .is_err());

        // The whole library leaves the non-paper out, and what it cited
        // or was cited by with it.
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &network)
            .unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(edges_of(&graph), edges(&[(2, 0), (3, 0), (3, 2)]));
        assert_eq!(graph.external, 3, "the strangers and B");
    }

    #[test]
    fn copying_bibtex_freezes_keys_once() {
        let mut store = new_store();
        let mut a = entry("/l/a.pdf", Some("a"));
        a.info_author = Some("Abel Author".into());
        a.info_year = Some(1999);
        a.info_title = Some("Alpha Work".into());
        store.reconcile_file(&a);
        let mut b = entry("/l/b.pdf", Some("b"));
        b.info_author = Some("Baker Author".into());
        b.info_year = Some(2010);
        b.info_title = Some("Beta Work".into());
        store.reconcile_file(&b);
        let (a_id, b_id) = (store.papers[0].id.clone(), store.papers[1].id.clone());

        let (text, frozen) = store.bibtex_for(&[b_id.clone(), a_id.clone()]).unwrap();
        assert!(frozen);
        let b_at = text.find("@misc{author2010beta,").expect("b entry");
        let a_at = text.find("@misc{author1999alpha,").expect("a entry");
        assert!(b_at < a_at, "in the order given, not list order:\n{text}");
        assert!(
            !text.contains("Written by SpeedyF"),
            "no header on the clipboard"
        );
        assert!(store.papers.iter().all(|p| p.cite_key_pinned));

        // Frozen: the key no longer follows the metadata.
        store
            .apply_patch(
                &a_id,
                MetaPatchDto {
                    year: Some(Some(2030)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(store.papers[0].cite_key, "author1999alpha");
        let (_, frozen_again) = store.bibtex_for(&[a_id]).unwrap();
        assert!(!frozen_again, "nothing left to freeze");
        assert!(store.bibtex_for(&["nope".into()]).is_err());
    }

    #[test]
    fn citation_links_resolve_through_identifiers_and_skip_self() {
        use CitationIdDto::{ArXiv, Doi};
        let mut store = new_store();
        let mut a = entry("/l/a.pdf", Some("a"));
        a.doi = Some("10.1/a".into());
        a.info_author = Some("Abel Author".into());
        let mut b = entry("/l/b.pdf", Some("b"));
        b.arxiv = Some("2401.00002".into());
        b.info_author = Some("Baker Author".into());
        let mut c = entry("/l/c.pdf", Some("c"));
        c.doi = Some("10.1/c".into());
        c.info_author = Some("Charlie Author".into());
        let d = entry("/l/d.pdf", Some("d"));
        for e in [&a, &b, &c, &d] {
            store.reconcile_file(e);
        }
        let ids: Vec<String> = store.papers.iter().map(|p| p.id.clone()).collect();
        let cites = |mut e: LibraryEntry, ids: Option<Vec<CitationIdDto>>| {
            e.cites = ids;
            e
        };
        let entries = vec![
            cites(
                a,
                Some(vec![
                    Doi("10.1/a".into()), // itself, through a second file's header say
                    ArXiv("2401.00002".into()),
                    Doi("10.1/c".into()),
                    Doi("10.1/zzz".into()), // nobody in the library
                    Doi("10.1/c".into()),
                    Doi("10.1/zzz".into()),
                ]),
            ),
            cites(b, Some(vec![Doi("10.1/a".into())])),
            cites(c, None), // text not indexed yet
            cites(
                d,
                Some(vec![Doi("10.1/a".into()), ArXiv("2401.00002".into())]),
            ),
        ];

        let links = store
            .citation_links(&ids[0], &entries, &DocRefs::new(), &NetworkEdges::default())
            .unwrap();
        let names =
            |rows: &[PaperRowDto]| rows.iter().map(|r| r.authors.clone()).collect::<Vec<_>>();
        assert_eq!(names(&links.cites), vec!["Author", "Author"]);
        let cited: Vec<&str> = links.cites.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            cited,
            vec![ids[1].as_str(), ids[2].as_str()],
            "in list order, once each, never itself"
        );
        let by: Vec<&str> = links.cited_by.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(by, vec![ids[1].as_str(), ids[3].as_str()]);
        assert_eq!(
            links.unresolved, 1,
            "counted once however often it is cited"
        );

        let links = store
            .citation_links(&ids[1], &entries, &DocRefs::new(), &NetworkEdges::default())
            .unwrap();
        assert_eq!(
            links
                .cites
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec![ids[0].as_str()]
        );
        assert_eq!(
            links
                .cited_by
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec![ids[0].as_str(), ids[3].as_str()]
        );
        assert_eq!(links.unresolved, 0);

        // Not extracted yet: cites nothing, and is still cited.
        let links = store
            .citation_links(&ids[2], &entries, &DocRefs::new(), &NetworkEdges::default())
            .unwrap();
        assert!(links.cites.is_empty());
        assert_eq!(links.cited_by.len(), 1);

        // No identifier: nothing can cite it, by construction.
        let links = store
            .citation_links(&ids[3], &entries, &DocRefs::new(), &NetworkEdges::default())
            .unwrap();
        assert_eq!(links.cites.len(), 2);
        assert!(links.cited_by.is_empty());
        assert!(store
            .citation_links("nope", &entries, &DocRefs::new(), &NetworkEdges::default())
            .is_err());
    }

    #[test]
    fn the_scans_kind_reaches_the_record_and_is_retracted() {
        let mut store = new_store();
        let mut thesis = classified(
            "/l/t.pdf",
            "t",
            PapernessVerdict::Paper,
            "a dissertation title page",
        );
        thesis.doi = Some("10.1/t".into());
        thesis.paper_kind = Some(PaperKind::Thesis);
        assert!(store.reconcile_file(&thesis));
        let id = store.papers[0].id.clone();
        let kind = |store: &PaperStore| store.papers[0].meta.kind.clone();
        assert_eq!(
            kind(&store),
            Some(MetaField {
                value: PaperKind::Thesis,
                source: MetaSource::FirstPage,
            })
        );
        // The same again changes nothing.
        assert!(!store.reconcile_file(&thesis));

        // A classified rescan that no longer sees a kind takes it back.
        let mut plain = classified("/l/t.pdf", "t", PapernessVerdict::Likely, "has an abstract");
        plain.doi = Some("10.1/t".into());
        assert!(store.reconcile_file(&plain));
        assert_eq!(kind(&store), None);

        // An entry the scan has not classified says nothing about it.
        store.reconcile_file(&thesis);
        let mut unclassified = entry("/l/t.pdf", Some("t"));
        unclassified.doi = Some("10.1/t".into());
        store.reconcile_file(&unclassified);
        assert_eq!(kind(&store).map(|f| f.value), Some(PaperKind::Thesis));

        // A second file of the same paper does not speak for it.
        let mut copy = classified(
            "/l/t-copy.pdf",
            "t2",
            PapernessVerdict::Paper,
            "ISBN 9780306406157 on the copyright page",
        );
        copy.doi = Some("10.1/t".into());
        copy.paper_kind = Some(PaperKind::Book);
        store.reconcile_file(&copy);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2);
        assert_eq!(kind(&store).map(|f| f.value), Some(PaperKind::Thesis));

        // The user's kind beats the scan's, and a rescan does not touch it.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    kind: Some(Some(PaperKind::Book)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        store.reconcile_file(&thesis);
        assert_eq!(
            kind(&store),
            Some(MetaField {
                value: PaperKind::Book,
                source: MetaSource::User,
            })
        );
        store.reconcile_file(&plain);
        assert_eq!(kind(&store).map(|f| f.source), Some(MetaSource::User));
    }

    #[test]
    fn the_verdict_is_the_primary_files_and_the_override_wins() {
        let mut store = new_store();
        let mut a = classified(
            "/l/a.pdf",
            "a",
            PapernessVerdict::Paper,
            "arXiv:2401.00001 on the first page",
        );
        a.arxiv = Some("2401.00001".into());
        assert!(store.reconcile_file(&a));
        let id = store.papers[0].id.clone();
        let dto = store.paper_dto(&id).unwrap();
        assert!(dto.is_paper);
        assert_eq!(dto.paper_reason, "arXiv:2401.00001 on the first page");
        assert_eq!(dto.paper_override, None);
        assert_eq!(store.papers[0].verdict, Some(PapernessVerdict::Paper));
        // The same again changes nothing.
        assert!(!store.reconcile_file(&a));

        // A second file of the same paper, classified as nothing, does not
        // speak for the paper: the primary file does.
        let mut journal = classified("/l/a-journal.pdf", "aj", PapernessVerdict::Unknown, "");
        journal.arxiv = Some("2401.00001".into());
        store.reconcile_file(&journal);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2);
        assert!(store.papers[0].is_paper());
        assert_eq!(
            store.papers[0].verdict_reason,
            "arXiv:2401.00001 on the first page"
        );

        // A rescan of the primary file re-derives the verdict.
        let mut a2 = classified("/l/a.pdf", "a", PapernessVerdict::Likely, "has an abstract");
        a2.arxiv = Some("2401.00001".into());
        assert!(store.reconcile_file(&a2));
        assert!(store.papers[0].is_paper());
        assert_eq!(
            store.paper_dto(&id).unwrap().paper_reason,
            "has an abstract"
        );
        let mut a3 = classified("/l/a.pdf", "a", PapernessVerdict::Unknown, "");
        a3.arxiv = Some("2401.00001".into());
        assert!(store.reconcile_file(&a3));
        assert!(!store.papers[0].is_paper());
        assert_eq!(store.paper_dto(&id).unwrap().paper_reason, "");
        // An entry the scan has not classified says nothing.
        let mut a4 = entry("/l/a.pdf", Some("a"));
        a4.arxiv = Some("2401.00001".into());
        store.reconcile_file(&a4);
        assert_eq!(store.papers[0].verdict, Some(PapernessVerdict::Unknown));

        // The user's word beats the verdict either way, and can be taken
        // back.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    is_paper: Some(Some(true)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.is_paper);
        assert_eq!(dto.paper_reason, "marked by you");
        assert_eq!(dto.paper_override, Some(true));
        store.reconcile_file(&a3);
        assert!(store.papers[0].is_paper(), "a rescan does not unmark");
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    is_paper: Some(Some(false)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(!dto.is_paper);
        assert_eq!(dto.paper_reason, "marked as not a paper by you");
        store.reconcile_file(&a2);
        assert!(!store.papers[0].is_paper(), "the override still wins");
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    is_paper: Some(None),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.is_paper, "back to the verdict, which is Likely now");
        assert_eq!(dto.paper_reason, "has an abstract");
        assert_eq!(dto.paper_override, None);
        // A patch that says nothing about it leaves it alone.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    is_paper: Some(Some(false)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(Some(2024)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(dto.paper_override, Some(false));

        // A record the user marked is not a placeholder to be dropped.
        let mut store = new_store();
        store.reconcile_file(&entry("/l/d.pdf", Some("d")));
        let marked = store.papers[0].id.clone();
        store
            .apply_patch(
                &marked,
                MetaPatchDto {
                    is_paper: Some(Some(false)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let mut e = entry("/l/e.pdf", Some("e"));
        e.doi = Some("10.1/y".into());
        store.reconcile_file(&e);
        let mut d2 = entry("/l/d.pdf", Some("d2"));
        d2.doi = Some("10.1/y".into());
        store.reconcile_file(&d2);
        assert!(store
            .papers
            .iter()
            .any(|p| p.id == marked && p.files.is_empty()));
    }

    #[test]
    fn an_unscanned_record_is_given_the_benefit_of_the_doubt() {
        let mut store = new_store();
        store.reconcile_file(&entry("/l/old.pdf", Some("o")));
        let paper = &store.papers[0];
        assert_eq!(paper.verdict, None);
        assert!(paper.is_paper());
        assert_eq!(paper.paper_reason(), "");
        let dto = store.paper_dto(&paper.id).unwrap();
        assert!(dto.is_paper);
        assert_eq!(dto.paper_reason, "");
        assert_eq!(store.counts().all, 1);
        assert_eq!(store.counts().others, 0);
        assert_eq!(store.paper_rows(&PaperFilterDto::default()).total, 1);
    }

    #[test]
    fn scope_rules_and_counts_tell_papers_from_the_rest() {
        let mut store = new_store();
        let mut filed_paper = classified("/l/p1.pdf", "p1", PapernessVerdict::Paper, "IEEE");
        filed_paper.info_title = Some("Filed paper".into());
        let mut loose_paper = classified("/l/p2.pdf", "p2", PapernessVerdict::Likely, "shape");
        loose_paper.info_title = Some("Loose paper".into());
        let mut filed_other = classified("/l/n1.pdf", "n1", PapernessVerdict::Unknown, "");
        filed_other.info_title = Some("Filed manual".into());
        let mut loose_other = classified("/l/n2.pdf", "n2", PapernessVerdict::Unknown, "");
        loose_other.info_title = Some("Loose receipt".into());
        let mut unscanned = entry("/l/u.pdf", Some("u"));
        unscanned.info_title = Some("Unscanned thing".into());
        for e in [
            &filed_paper,
            &loose_paper,
            &filed_other,
            &loose_other,
            &unscanned,
        ] {
            store.reconcile_file(e);
        }
        let id_of = |store: &PaperStore, path: &str| {
            store
                .papers
                .iter()
                .find(|p| p.files[0].path == path)
                .unwrap()
                .id
                .clone()
        };
        let p1 = id_of(&store, "/l/p1.pdf");
        let n1 = id_of(&store, "/l/n1.pdf");
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&p1, &project, true).unwrap();
        store.set_membership(&n1, &project, true).unwrap();

        let counts = store.counts();
        assert_eq!(
            (counts.all, counts.unfiled, counts.others),
            (3, 2, 2),
            "papers (incl. the unscanned one), papers in no project, the rest"
        );

        let titles = |store: &PaperStore, filter: PaperFilterDto| {
            let list = store.paper_rows(&filter);
            let titles: Vec<String> = list.rows.iter().map(|r| r.title.clone()).collect();
            (titles, list.total)
        };
        assert_eq!(
            titles(&store, PaperFilterDto::default()),
            (
                vec![
                    "Filed paper".to_owned(),
                    "Loose paper".to_owned(),
                    "Unscanned thing".to_owned()
                ],
                3
            )
        );
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    unfiled: true,
                    ..PaperFilterDto::default()
                }
            ),
            (
                vec!["Loose paper".to_owned(), "Unscanned thing".to_owned()],
                2
            )
        );
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    others: true,
                    ..PaperFilterDto::default()
                }
            ),
            (
                vec!["Filed manual".to_owned(), "Loose receipt".to_owned()],
                2
            ),
            "non-papers, filed or not"
        );
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    others: true,
                    unfiled: true,
                    ..PaperFilterDto::default()
                }
            ),
            (
                vec!["Filed manual".to_owned(), "Loose receipt".to_owned()],
                2
            ),
            "others takes precedence over unfiled"
        );
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    project: Some(project.clone()),
                    others: true,
                    unfiled: true,
                    ..PaperFilterDto::default()
                }
            ),
            (vec!["Filed manual".to_owned(), "Filed paper".to_owned()], 2),
            "a project shows every member"
        );
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    others: true,
                    query: "receipt".into(),
                    ..PaperFilterDto::default()
                }
            ),
            (vec!["Loose receipt".to_owned()], 2)
        );
        let rows = store
            .paper_rows(&PaperFilterDto {
                project: Some(project.clone()),
                ..PaperFilterDto::default()
            })
            .rows;
        let flags: Vec<(String, bool)> =
            rows.iter().map(|r| (r.title.clone(), r.is_paper)).collect();
        assert_eq!(
            flags,
            vec![
                ("Filed manual".to_owned(), false),
                ("Filed paper".to_owned(), true)
            ]
        );
        assert_eq!(store.project_dto(&project).unwrap().paper_count, 2);

        // Marking the manual a paper moves it; marking a paper otherwise
        // moves it the other way.
        store
            .apply_patch(
                &n1,
                MetaPatchDto {
                    is_paper: Some(Some(true)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        store
            .apply_patch(
                &p1,
                MetaPatchDto {
                    is_paper: Some(Some(false)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let counts = store.counts();
        assert_eq!((counts.all, counts.unfiled, counts.others), (3, 2, 2));
        assert_eq!(
            titles(
                &store,
                PaperFilterDto {
                    others: true,
                    ..PaperFilterDto::default()
                }
            )
            .0,
            vec!["Filed paper".to_owned(), "Loose receipt".to_owned()]
        );
    }

    fn xmp_entry(path: &str, fingerprint: &str) -> LibraryEntry {
        let mut e = entry(path, Some(fingerprint));
        e.xmp = Some(crate::library::xmp::XmpFields {
            title: Some("Fast and Accurate Alignment".into()),
            creators: vec!["Heng Li".into(), "Durbin, Richard".into()],
            doi: Some("10.1186/S12859-019-2821-9".into()),
            isbn: None,
            pii: None,
            publication_name: Some("BMC Bioinformatics".into()),
            volume: Some("20".into()),
            number: Some("1".into()),
            start_page: Some("212".into()),
            end_page: Some("219".into()),
            year: Some(2019),
            publisher: Some("Springer".into()),
        });
        e
    }

    #[test]
    fn xmp_is_offered_at_its_rank_and_retracted_when_the_packet_goes() {
        let mut store = new_store();
        let mut e = xmp_entry("/l/a.pdf", "a");
        e.title = Some("Running header of the journal".into());
        e.info_title = Some("Info Title Here".into());
        e.info_author = Some("H. Li".into());
        e.info_year = Some(2021);
        assert!(store.reconcile_file(&e));
        let paper = &store.papers[0];
        let id = paper.id.clone();
        assert_eq!(
            meta_title(paper),
            (Some("Fast and Accurate Alignment"), Some(MetaSource::Xmp)),
            "XMP beats the Info dictionary"
        );
        assert_eq!(source_of(&paper.meta.authors), Some(MetaSource::Xmp));
        let names: Vec<String> = paper
            .authors()
            .iter()
            .map(|a| format!("{}|{}", a.given, a.family))
            .collect();
        assert_eq!(names, vec!["Heng|Li", "Richard|Durbin"]);
        assert_eq!(
            paper.meta.year.as_ref().map(|y| (y.value, y.source)),
            Some((2019, MetaSource::Xmp))
        );
        assert_eq!(paper.venue(), Some("BMC Bioinformatics"));
        assert_eq!(
            paper.meta.volume.as_ref().map(|v| v.value.as_str()),
            Some("20")
        );
        assert_eq!(
            paper.meta.issue.as_ref().map(|v| v.value.as_str()),
            Some("1")
        );
        assert_eq!(
            paper.meta.pages.as_ref().map(|v| v.value.as_str()),
            Some("212-219")
        );
        assert_eq!(
            paper.meta.publisher.as_ref().map(|v| v.value.as_str()),
            Some("Springer")
        );
        assert_eq!(
            paper.ids.doi.as_deref(),
            Some("10.1186/s12859-019-2821-9"),
            "the packet's DOI, normalized"
        );
        assert_eq!(paper.cite_key, "li2019fast");
        assert!(paper.incomplete().is_empty());

        // The DOI from the packet is an identity: a renamed, re-saved copy
        // with the same packet is the same paper.
        store.reconcile_file(&xmp_entry("/l/renamed.pdf", "other-bytes"));
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2);
        // ...and a second file without a packet does not retract what the
        // primary one supplies.
        let mut bare = entry("/l/renamed.pdf", Some("other-bytes"));
        bare.doi = Some("10.1186/s12859-019-2821-9".into());
        store.reconcile_file(&bare);
        assert_eq!(store.papers[0].venue(), Some("BMC Bioinformatics"));

        // A lookup outranks it; a user edit outranks everything.
        let crossref = candidate(
            MetaSource::Crossref,
            "Fast and accurate long-read alignment",
            &[("Li", "Heng")],
            Some(2019),
        );
        store.apply_candidate(&id, &crossref).unwrap();
        assert_eq!(
            source_of(&store.papers[0].meta.title),
            Some(MetaSource::Crossref)
        );
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    venue: Some(Some("My venue".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();

        // The packet goes (the file was replaced): its fields go with it,
        // except what a lookup or the user set.
        let mut gone = entry("/l/a.pdf", Some("a"));
        gone.doi = Some("10.1186/s12859-019-2821-9".into());
        gone.title = Some("Running header of the journal".into());
        gone.info_title = Some("Info Title Here".into());
        assert!(store.reconcile_file(&gone));
        let paper = &store.papers[0];
        assert_eq!(source_of(&paper.meta.title), Some(MetaSource::Crossref));
        assert_eq!(source_of(&paper.meta.authors), Some(MetaSource::Crossref));
        assert_eq!(paper.venue(), Some("My venue"));
        assert!(paper.meta.volume.is_none());
        assert!(paper.meta.issue.is_none());
        assert!(paper.meta.pages.is_none());
        assert!(paper.meta.publisher.is_none());
        assert_eq!(
            paper.ids.doi.as_deref(),
            Some("10.1186/s12859-019-2821-9"),
            "the page still carries it"
        );
        assert_eq!(
            paper.ids.doi_source,
            Some(MetaSource::FirstPage),
            "now at the page's say rather than the packet's"
        );

        // Without the lookup, the title falls back to the Info dictionary.
        let mut store = new_store();
        store.reconcile_file(&e);
        store.reconcile_file(&gone);
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("Info Title Here"), Some(MetaSource::PdfInfo))
        );
        assert!(store.papers[0].meta.year.is_none());

        // A packet title that is a file name is no title.
        let mut store = new_store();
        let mut named = xmp_entry("/l/b.pdf", "b");
        named.xmp.as_mut().unwrap().title = Some("main.tex".into());
        named.title = Some("The first line of the page".into());
        store.reconcile_file(&named);
        assert_eq!(
            meta_title(&store.papers[0]),
            (
                Some("The first line of the page"),
                Some(MetaSource::FirstPage)
            )
        );
    }

    #[test]
    fn isbns_come_from_the_page_then_the_packet_and_can_be_edited() {
        let mut store = new_store();
        let mut e = xmp_entry("/l/book.pdf", "b");
        e.xmp.as_mut().unwrap().isbn = Some("9780306406157".into());
        e.isbn = Some("0306406152".into());
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();
        assert_eq!(
            store.paper_dto(&id).unwrap().isbn.as_deref(),
            Some("0306406152"),
            "the page's first"
        );
        assert_eq!(store.papers[0].ids.isbn_source, Some(MetaSource::FirstPage));
        // The page no longer prints one: the primary file's rescan takes
        // the packet's instead - what the file says is re-derived.
        let mut again = e.clone();
        again.isbn = None;
        assert!(store.reconcile_file(&again));
        assert_eq!(store.papers[0].ids.isbn.as_deref(), Some("9780306406157"));
        assert_eq!(store.papers[0].ids.isbn_source, Some(MetaSource::Xmp));

        let mut only_packet = xmp_entry("/l/book2.pdf", "b2");
        only_packet.xmp.as_mut().unwrap().doi = None;
        only_packet.xmp.as_mut().unwrap().isbn = Some("9780306406157".into());
        // Another work: the same title, author and year would be merged.
        only_packet.xmp.as_mut().unwrap().title = Some("Another Book Entirely Here".into());
        store.reconcile_file(&only_packet);
        assert_eq!(store.papers[1].ids.isbn.as_deref(), Some("9780306406157"));

        // Patched: normalized, validated, cleared - and the user's outlives
        // any rescan.
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    isbn: Some(Some(" 978-0-13-468599-1 ".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(dto.isbn.as_deref(), Some("9780134685991"));
        assert_eq!(store.papers[0].ids.isbn_source, Some(MetaSource::User));
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].ids.isbn.as_deref(), Some("9780134685991"));
        let error = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    isbn: Some(Some("978-0-13-468599-2".into())),
                    ..MetaPatchDto::default()
                },
            )
            .expect_err("bad check digit");
        assert!(error.to_string().contains("not a valid ISBN"), "{error}");
        assert_eq!(
            store.papers[0].ids.isbn.as_deref(),
            Some("9780134685991"),
            "a refused patch changes nothing"
        );
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    isbn: Some(None),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.isbn.is_none());
        let dto = store
            .apply_patch(
                &id,
                MetaPatchDto {
                    isbn: Some(Some("  ".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert!(dto.isbn.is_none(), "blank clears too");

        // A lookup's ISBN lands when the paper has none, and the scan
        // never replaces it.
        let mut found = candidate(MetaSource::OpenLibrary, "A Book", &[("Author", "A.")], None);
        found.isbn = Some("978-0-13-468599-1".into());
        let dto = store.apply_candidate(&id, &found).unwrap();
        assert_eq!(dto.isbn.as_deref(), Some("9780134685991"));
        assert_eq!(
            store.papers[0].ids.isbn_source,
            Some(MetaSource::OpenLibrary)
        );
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].ids.isbn.as_deref(), Some("9780134685991"));
    }

    #[test]
    fn parked_candidates_are_counted_discarded_or_cleared_by_applying_one() {
        let mut store = new_store();
        let mut e = entry("/l/a.pdf", Some("a"));
        e.title = Some("Some paper title here".into());
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();
        let parked = vec![
            candidate(
                MetaSource::Openalex,
                "Some Paper Title",
                &[("A", "B")],
                Some(2020),
            ),
            candidate(
                MetaSource::Crossref,
                "Some Paper Title Here",
                &[("C", "D")],
                Some(2021),
            ),
        ];
        store.papers[0].candidates = parked.clone();
        let dto = store.paper_dto(&id).unwrap();
        assert_eq!(dto.candidates, parked);
        let row = &store.paper_rows(&PaperFilterDto::default()).rows[0];
        assert_eq!(row.candidate_count, 2);

        let dto = store.discard_candidates(&id).unwrap();
        assert!(dto.candidates.is_empty());
        assert_eq!(
            store.paper_rows(&PaperFilterDto::default()).rows[0].candidate_count,
            0
        );
        assert!(store.discard_candidates("nope").is_err());

        store.papers[0].candidates = parked.clone();
        let dto = store.apply_candidate(&id, &parked[1]).unwrap();
        assert!(
            dto.candidates.is_empty(),
            "applying one answers the question"
        );
        assert_eq!(dto.cite_key, "c2021paper", "\"some\" is a stop word");
    }

    #[test]
    fn an_xmp_only_doi_never_steals_a_file_from_a_filed_or_edited_paper() {
        let mut store = new_store();
        let mut article = entry("/l/article.pdf", Some("art"));
        article.doi = Some("10.1/a".into());
        article.title = Some("The Article".into());
        store.reconcile_file(&article);
        let supplement = entry("/l/supplement.pdf", Some("sup"));
        store.reconcile_file(&supplement);
        let (article_id, supplement_id) = (store.papers[0].id.clone(), store.papers[1].id.clone());
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&article_id, &project, true).unwrap();
        store
            .set_membership(&supplement_id, &project, true)
            .unwrap();

        // The supplement's packet carries the article's DOI: it stays its
        // own paper, without the DOI, and the article keeps its one file.
        let mut packet = xmp_entry("/l/supplement.pdf", "sup");
        packet.xmp.as_mut().unwrap().doi = Some("10.1/a".into());
        store.reconcile_file(&packet);
        assert_eq!(store.papers.len(), 2);
        assert_eq!(store.papers[0].files.len(), 1);
        assert_eq!(store.papers[1].files.len(), 1);
        assert_eq!(
            store.papers[1].ids.doi, None,
            "two papers never share a DOI"
        );
        assert_eq!(store.papers[1].id, supplement_id);
        // And again, so nothing flips between scans.
        assert!(!store.reconcile_file(&packet));

        // A user edit protects the same way.
        store
            .set_membership(&supplement_id, &project, false)
            .unwrap();
        store
            .apply_patch(
                &supplement_id,
                MetaPatchDto {
                    title: Some(Some("Supplementary material".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        store.reconcile_file(&packet);
        assert_eq!(store.papers.len(), 2);
        assert_eq!(store.papers[1].ids.doi, None);

        // A placeholder - not filed, not edited - does move to the paper
        // whose DOI its packet carries, and goes.
        let mut store = new_store();
        store.reconcile_file(&article);
        store.reconcile_file(&supplement);
        assert_eq!(store.papers.len(), 2);
        store.reconcile_file(&packet);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2);

        // A brand-new file known by nothing but its packet's DOI joins the
        // paper that holds it - a renamed, re-saved copy is the same paper.
        let mut copy = xmp_entry("/l/copy.pdf", "copy");
        copy.xmp.as_mut().unwrap().doi = Some("10.1/a".into());
        store.reconcile_file(&copy);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 3);

        // A DOI read off the page is still the strong identity it was.
        let mut store = new_store();
        store.reconcile_file(&article);
        store.reconcile_file(&supplement);
        let mut page = entry("/l/supplement.pdf", Some("sup"));
        page.doi = Some("10.1/a".into());
        store.reconcile_file(&page);
        assert_eq!(store.papers.len(), 1, "re-identified by the page's DOI");
    }

    #[test]
    fn only_the_primary_file_supplies_fields_and_an_unchanged_rescan_touches_nothing() {
        let mut store = new_store();
        let mut preprint = entry("/l/preprint.pdf", Some("pre"));
        preprint.arxiv = Some("2401.00001".into());
        preprint.title = Some("The preprint title line".into());
        preprint.info_year = Some(2023);
        store.reconcile_file(&preprint);
        // The journal version joins by its arXiv id, with a richer packet.
        let mut journal = xmp_entry("/l/journal.pdf", "jour");
        journal.arxiv = Some("2401.00001".into());
        journal.info_year = Some(2024);
        journal.info_title = Some("Journal Info Title".into());
        store.reconcile_file(&journal);
        assert_eq!(store.papers.len(), 1);
        let paper = &store.papers[0];
        assert_eq!(paper.files.len(), 2);
        assert_eq!(
            meta_title(paper),
            (Some("The preprint title line"), Some(MetaSource::FirstPage)),
            "the second file's title does not land"
        );
        assert_eq!(
            paper.meta.year.as_ref().map(|y| (y.value, y.source)),
            Some((2024, MetaSource::Identifier)),
            "the shared arXiv id's year outranks both Info dictionaries"
        );
        assert_eq!(paper.venue(), None);
        assert!(paper.meta.volume.is_none() && paper.meta.pages.is_none());
        assert!(paper.authors().is_empty());
        assert_eq!(
            paper.ids.doi.as_deref(),
            Some("10.1186/s12859-019-2821-9"),
            "identifiers still come from any file"
        );

        // Rescans of either file, unchanged, change and touch nothing: the
        // preprint's rescan does not take back the DOI the journal version
        // supplies, though the preprint carries none.
        let before = store.papers[0].updated_at.clone();
        assert!(!store.reconcile_file(&journal));
        assert!(!store.reconcile_file(&preprint));
        assert!(!store.reconcile_file(&journal));
        assert_eq!(store.papers[0].updated_at, before);
        assert_eq!(
            store.papers[0].ids.doi.as_deref(),
            Some("10.1186/s12859-019-2821-9")
        );
        assert_eq!(
            meta_title(&store.papers[0]),
            (Some("The preprint title line"), Some(MetaSource::FirstPage))
        );
    }

    fn year_of(paper: &PaperRecord) -> Option<(i32, MetaSource)> {
        paper.meta.year.as_ref().map(|y| (y.value, y.source))
    }

    #[test]
    fn an_identifiers_year_is_offered_at_its_rank_and_retracted_with_it() {
        let mut store = new_store();
        // An arXiv id alone: its submission year, above the Info
        // dictionary's CreationDate.
        let mut e = entry("/l/a.pdf", Some("a"));
        e.arxiv = Some("1706.03762".into());
        e.info_title = Some("Attention Is All You Need".into());
        e.info_year = Some(2023);
        assert!(store.reconcile_file(&e));
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2017, MetaSource::Identifier)),
            "the id beats the Info dictionary"
        );
        // The file no longer carries the id: the id and its year go.
        e.arxiv = None;
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].ids.arxiv, None);
        assert_eq!(year_of(&store.papers[0]), Some((2023, MetaSource::PdfInfo)));
        // A DOI of a known family says its year too.
        e.doi = Some("10.1016/j.physa.2020.124578".into());
        store.reconcile_file(&e);
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2020, MetaSource::Identifier))
        );
        // One of an unknown family - the page now reads differently, so
        // it replaces the other - says nothing, and the Info year returns.
        e.doi = Some("10.1103/PhysRevLett.78.2690".into());
        store.reconcile_file(&e);
        assert_eq!(
            store.papers[0].ids.doi.as_deref(),
            Some("10.1103/physrevlett.78.2690")
        );
        assert_eq!(year_of(&store.papers[0]), Some((2023, MetaSource::PdfInfo)));

        // The PII case: a scan with no DOI anywhere. The scan derived the
        // DOI; the record takes the year out of it.
        let mut old = entry("/l/old.pdf", Some("old"));
        old.doi = Some("10.1016/0022-0000(88)90028-1".into());
        old.title = Some("Some Old Theorem And Its Proof".into());
        store.reconcile_file(&old);
        let paper = store
            .papers
            .iter()
            .find(|p| p.files[0].path == "/l/old.pdf")
            .unwrap();
        assert_eq!(
            paper.ids.doi.as_deref(),
            Some("10.1016/0022-0000(88)90028-1")
        );
        assert_eq!(year_of(paper), Some((1988, MetaSource::Identifier)));
        assert!(
            paper.has_lookup_identifier(),
            "a PII makes the paper lookable-up"
        );
    }

    #[test]
    fn an_identifiers_year_loses_to_xmp_lookups_and_the_user() {
        let mut store = new_store();
        let mut e = xmp_entry("/l/a.pdf", "a");
        e.arxiv = Some("1706.03762".into());
        store.reconcile_file(&e);
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2019, MetaSource::Xmp)),
            "the packet's publication date outranks the id"
        );
        // Without the packet's year the id's stands.
        e.xmp.as_mut().unwrap().year = None;
        store.reconcile_file(&e);
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2017, MetaSource::Identifier))
        );
        // The user's year is never touched by a rescan.
        let id = store.papers[0].id.clone();
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(Some(2018)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        store.reconcile_file(&e);
        assert_eq!(year_of(&store.papers[0]), Some((2018, MetaSource::User)));
        e.arxiv = None;
        store.reconcile_file(&e);
        assert_eq!(year_of(&store.papers[0]), Some((2018, MetaSource::User)));
    }

    #[test]
    fn an_identifiers_year_is_the_primary_files_only() {
        let mut store = new_store();
        let mut preprint = entry("/l/preprint.pdf", Some("pre"));
        preprint.arxiv = Some("2401.00001".into());
        preprint.title = Some("The preprint title line".into());
        store.reconcile_file(&preprint);
        let mut journal = entry("/l/journal.pdf", Some("jour"));
        journal.arxiv = Some("2401.00001".into());
        journal.doi = Some("10.1016/j.physa.2025.124578".into());
        store.reconcile_file(&journal);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2024, MetaSource::Identifier)),
            "the second file's DOI year does not land"
        );
    }

    #[test]
    fn a_packets_pii_stands_in_for_its_doi_at_the_packets_strength() {
        let mut store = new_store();
        let mut e = entry("/l/a.pdf", Some("a"));
        e.xmp = Some(crate::library::xmp::XmpFields {
            pii: Some("S0022000088900281".into()),
            ..crate::library::xmp::XmpFields::default()
        });
        // The packet's compact form came through `find_pii` expanded.
        e.xmp.as_mut().unwrap().pii = identifiers::find_pii("S0022000088900281");
        store.reconcile_file(&e);
        assert_eq!(
            store.papers[0].ids.doi.as_deref(),
            Some("10.1016/s0022-0000(88)90028-1")
        );
        assert_eq!(
            year_of(&store.papers[0]),
            Some((1988, MetaSource::Identifier))
        );
        // A packet PII that fails its ISSN check is no identity at all.
        let mut bad = entry("/l/b.pdf", Some("b"));
        bad.xmp = Some(crate::library::xmp::XmpFields {
            pii: Some("S0022-0001(88)90028-1".into()),
            ..crate::library::xmp::XmpFields::default()
        });
        store.reconcile_file(&bad);
        assert_eq!(store.papers.len(), 2);
        assert!(store.papers[1].ids.doi.is_none());
        assert!(store.papers[1].meta.year.is_none());
    }

    #[test]
    fn the_stamps_category_reaches_the_record_and_the_lookup_target() {
        let mut store = new_store();
        let mut e = entry("/l/a.pdf", Some("a"));
        e.arxiv = Some("1706.03762".into());
        e.arxiv_category = Some("cs.CL".into());
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();
        assert_eq!(store.papers[0].arxiv_category.as_deref(), Some("cs.CL"));
        let target = store.lookup_target(&id).unwrap();
        assert_eq!(target.arxiv_category.as_deref(), Some("cs.CL"));
        // The primary file's rescan re-derives it with the id, as it does
        // the identifiers: replaced when the stamp now says otherwise,
        // taken back when there is no stamp; and a second file supplies
        // one when the first had none.
        e.arxiv_category = Some("stat.ML".into());
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].arxiv_category.as_deref(), Some("stat.ML"));
        e.arxiv_category = None;
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].arxiv_category, None);
        e.arxiv_category = Some("cs.CL".into());
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].arxiv_category.as_deref(), Some("cs.CL"));
        let mut physics = entry("/l/p.pdf", Some("p"));
        physics.arxiv = Some("2401.00002".into());
        store.reconcile_file(&physics);
        let mut again = entry("/l/p-v2.pdf", Some("p2"));
        again.arxiv = Some("2401.00002".into());
        again.arxiv_category = Some("hep-th".into());
        store.reconcile_file(&again);
        assert_eq!(store.papers[1].arxiv_category.as_deref(), Some("hep-th"));
        // A new-style id says nothing about its subject; the stamp does.
        let plan = crate::library::chain::plan_identifier_sources(
            &store.lookup_target(&store.papers[1].id).unwrap(),
        );
        assert!(plan
            .iter()
            .any(|step| step.source == crate::library::chain::Source::Inspire));
        let plan = crate::library::chain::plan_identifier_sources(&target);
        assert!(!plan
            .iter()
            .any(|step| step.source == crate::library::chain::Source::Inspire));
    }

    #[test]
    fn xmp_creators_parse_one_person_per_entry() {
        let names = |creators: &[&str]| -> Vec<String> {
            parse_xmp_creators(&creators.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>())
                .into_iter()
                .map(|a| format!("{}|{}", a.given, a.family))
                .collect()
        };
        assert_eq!(
            names(&["Heng Li", "Durbin, Richard"]),
            vec!["Heng|Li", "Richard|Durbin"]
        );
        assert_eq!(names(&["Jarzynski, C."]), vec!["C.|Jarzynski"]);
        assert_eq!(
            names(&["A. Author and B. Author"]),
            vec!["A.|Author", "B.|Author"]
        );
        assert_eq!(
            names(&["Smith, J.; Jones, K."]),
            vec!["J.|Smith", "K.|Jones"]
        );
        assert_eq!(
            names(&["Evans, D., Cohen, E."]),
            vec!["D.|Evans", "E.|Cohen"],
            "two people in one entry"
        );
        assert!(names(&["", "  ", "Unknown", ", "]).is_empty());
    }

    /// The demo library's bug, at the record: a one-page paper whose own
    /// DOI is in the head and whose reference list cites a preprint by
    /// its arXiv id. The old scan made the cited id the record's, the
    /// id's year beat the Info dictionary's, and a lookup would have
    /// fetched the preprint's metadata for it.
    #[test]
    fn a_cited_arxiv_id_in_the_reference_list_is_not_the_papers_own() {
        use crate::library::{cited_identifiers, entry_from_first_page, ScanCandidate};
        let page = "A Demonstration of Library Scanning\nJ. Doe\nDOI: 10.5555/speedyf.001\nAbstract\nWe cite things.\nReferences\n[1] A. Vaswani et al. Attention is all you need. arXiv:1706.03762, 2017.";
        let candidate = ScanCandidate {
            path: PathBuf::from("/l/demo.pdf"),
            mtime_ms: 1,
            size: 100,
            epoch: 0,
            unchanged: false,
        };
        let mut e = entry_from_first_page(&candidate, page);
        e.fingerprint = Some("demo".into());
        e.info_year = Some(2019);
        assert_eq!(e.doi.as_deref(), Some("10.5555/speedyf.001"));
        assert_eq!(e.doi_source, Some(MetaSource::FirstPage));
        assert_eq!(e.arxiv, None, "a cited id is not the paper's");
        let cites = cited_identifiers(page, e.doi.as_deref(), e.arxiv.as_deref());
        assert_eq!(cites, vec![CitationIdDto::ArXiv("1706.03762".into())]);
        e.cites = Some(cites);

        let mut store = new_store();
        assert!(store.reconcile_file(&e));
        let paper = &store.papers[0];
        assert_eq!(paper.ids.doi.as_deref(), Some("10.5555/speedyf.001"));
        assert_eq!(paper.ids.doi_source, Some(MetaSource::FirstPage));
        assert_eq!(paper.ids.arxiv, None);
        assert_eq!(year_of(paper), Some((2019, MetaSource::PdfInfo)));
        let links = store
            .citation_links(
                &paper.id,
                &[e.clone()],
                &DocRefs::new(),
                &NetworkEdges::default(),
            )
            .unwrap();
        assert_eq!(links.unresolved, 1, "the preprint is cited, not self");

        // A record the old scan built - the cited id as its own, the id's
        // year over the Info dictionary's, no sources kept - is put right
        // by the first rescan.
        let mut store = new_store();
        let mut wrong = e.clone();
        wrong.arxiv = Some("1706.03762".into());
        store.reconcile_file(&wrong);
        assert_eq!(
            year_of(&store.papers[0]),
            Some((2017, MetaSource::Identifier))
        );
        store.papers[0].ids.doi_source = None;
        store.papers[0].ids.arxiv_source = None;
        assert!(store.reconcile_file(&e));
        let paper = &store.papers[0];
        assert_eq!(paper.ids.arxiv, None, "cleared: the file never carried it");
        assert_eq!(paper.ids.arxiv_source, None);
        assert_eq!(paper.ids.doi.as_deref(), Some("10.5555/speedyf.001"));
        assert_eq!(paper.ids.doi_source, Some(MetaSource::FirstPage));
        assert_eq!(
            year_of(paper),
            Some((2019, MetaSource::PdfInfo)),
            "the id's year went with it"
        );
    }

    #[test]
    fn a_scanned_identifier_is_rederived_and_a_lookups_or_the_users_is_kept() {
        let mut store = new_store();
        let mut e = entry("/l/a.pdf", Some("a"));
        e.doi = Some("10.1/first".into());
        e.doi_source = Some(MetaSource::FirstPage);
        store.reconcile_file(&e);
        let id = store.papers[0].id.clone();
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::FirstPage));
        // The page now reads differently: replaced.
        e.doi = Some("10.1/second".into());
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/second"));
        // The page no longer carries one: cleared.
        e.doi = None;
        e.doi_source = None;
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].ids.doi, None);
        assert_eq!(store.papers[0].ids.doi_source, None);

        // A lookup's DOI survives a rescan that finds none, and one that
        // finds another.
        let mut crossref = candidate(MetaSource::Crossref, "A Title", &[("A", "B")], Some(2020));
        crossref.doi = Some("10.1/crossref".into());
        store.apply_candidate(&id, &crossref).unwrap();
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::Crossref));
        assert!(!store.reconcile_file(&e));
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/crossref"));
        e.doi = Some("10.1/page".into());
        e.doi_source = Some(MetaSource::FirstPage);
        // The file's own DOI is noted on the file (its traits for choosing
        // the main file), and nothing else changes.
        assert!(store.reconcile_file(&e));
        assert_eq!(store.papers[0].files[0].traits.dois, ["10.1/page"]);
        assert!(!store.reconcile_file(&e));
        assert_eq!(
            store.papers[0].ids.doi.as_deref(),
            Some("10.1/crossref"),
            "a lookup's is never overwritten by the scan"
        );

        // The user's survives both as well.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    doi: Some(Some("10.1/typed".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::User));
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/typed"));
        e.doi = None;
        e.doi_source = None;
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/typed"));
        // Cleared by the user it is simply absent, and the scan fills it.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    doi: Some(None),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(store.papers[0].ids.doi, None);
        assert_eq!(store.papers[0].ids.doi_source, None);
        e.doi = Some("10.1/page".into());
        e.doi_source = Some(MetaSource::FirstPage);
        store.reconcile_file(&e);
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/page"));
        assert_eq!(store.papers[0].ids.doi_source, Some(MetaSource::FirstPage));

        // An old record with no sources kept is the scan's to correct; a
        // second file only fills what is absent.
        let mut store = new_store();
        let mut old = entry("/l/old.pdf", Some("o"));
        old.arxiv = Some("1706.03762".into());
        store.reconcile_file(&old);
        store.papers[0].ids.arxiv_source = None;
        let mut rescanned = entry("/l/old.pdf", Some("o"));
        rescanned.arxiv = Some("2401.00001".into());
        assert!(store.reconcile_file(&rescanned));
        assert_eq!(store.papers[0].ids.arxiv.as_deref(), Some("2401.00001"));
        assert_eq!(
            store.papers[0].ids.arxiv_source,
            Some(MetaSource::FirstPage)
        );
        let mut second = entry("/l/old-v2.pdf", Some("o2"));
        second.arxiv = Some("2401.00001".into());
        second.doi = Some("10.1/journal".into());
        store.reconcile_file(&second);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].ids.doi.as_deref(), Some("10.1/journal"));
        let mut other = entry("/l/old-v2.pdf", Some("o2"));
        other.arxiv = Some("2401.00001".into());
        other.doi = Some("10.1000/other".into());
        store.reconcile_file(&other);
        assert_eq!(
            store.papers[0].ids.doi.as_deref(),
            Some("10.1/journal"),
            "not the primary file: no re-derivation"
        );

        // The file name's id is recorded as the name's.
        let mut named = entry("/l/2401.12345.pdf", Some("n"));
        named.arxiv = Some("2401.12345".into());
        named.arxiv_source = Some(MetaSource::Filename);
        store.reconcile_file(&named);
        assert_eq!(store.papers[1].ids.arxiv_source, Some(MetaSource::Filename));

        // A typed identifier is a user edit: its record is no placeholder,
        // so a file re-identified elsewhere leaves it standing.
        let mut store = new_store();
        store.reconcile_file(&entry("/l/d.pdf", Some("d")));
        let typed = store.papers[0].id.clone();
        store
            .apply_patch(
                &typed,
                MetaPatchDto {
                    arxiv: Some(Some("2401.99999".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        let mut y = entry("/l/y.pdf", Some("y"));
        y.doi = Some("10.1/y".into());
        store.reconcile_file(&y);
        let mut d2 = entry("/l/d.pdf", Some("d2"));
        d2.doi = Some("10.1/y".into());
        store.reconcile_file(&d2);
        assert!(store
            .papers
            .iter()
            .any(|p| p.id == typed && p.files.is_empty()));
    }

    #[test]
    fn a_file_reidentified_as_another_paper_moves_and_drops_its_placeholder() {
        let mut store = new_store();
        let mut a = entry("/l/a.pdf", Some("a"));
        a.doi = Some("10.1/a".into());
        store.reconcile_file(&a);
        store.reconcile_file(&entry("/l/b.pdf", Some("b")));
        assert_eq!(store.papers.len(), 2);
        // The scan now reads a DOI out of b.pdf that is a.pdf's.
        let mut b = entry("/l/b.pdf", Some("b"));
        b.doi = Some("10.1/a".into());
        store.reconcile_file(&b);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(store.papers[0].files.len(), 2);
    }

    /// A file on disk in `dir`, so the copies of one PDF are told from a
    /// rename (`attach_file` asks the disk), scanned as `fingerprint`.
    fn on_disk(
        dir: &Path,
        name: &str,
        fingerprint: &str,
        mtime_ms: u64,
        authors: bool,
        arxiv: Option<&str>,
    ) -> LibraryEntry {
        let path = dir.join(name);
        std::fs::write(&path, fingerprint).unwrap();
        let mut e = entry(path.to_str().unwrap(), Some(fingerprint));
        e.mtime_ms = mtime_ms;
        e.arxiv = arxiv.map(str::to_owned);
        e.arxiv_source = arxiv.map(|_| MetaSource::FirstPage);
        if authors {
            e.first_page_authors = vec![Author {
                given: "Ada".into(),
                family: "Lovelace".into(),
            }];
        }
        e
    }

    fn names(store: &PaperStore, index: usize) -> Vec<String> {
        store.papers[index]
            .files
            .iter()
            .map(|file| file.name.clone())
            .collect()
    }

    /// The case that asked for the rules. One work, five files:
    /// - `2606.13614v1.pdf` and `2606.13614v1 (1).pdf`: the same bytes, the
    ///   arXiv v1 (version from the name), authors on the first page,
    ///   downloaded first (mtime 90 and 100);
    /// - `mot.pdf` and `mot (1).pdf`: the same bytes, a later version with
    ///   the arXiv stamp (its version is not in the name, so unknown) and
    ///   authors (mtime 200 and 300);
    /// - `mot_tmlr.pdf`: the TMLR review copy, anonymous, no stamp, newest
    ///   (mtime 400), filed as its own record and merged in by hand.
    ///
    /// Nothing is published (rule 1). v1 is the newest known version, and
    /// the unknown versions stay in (rule 2). The anonymous TMLR copy goes
    /// (rule 3), then the browser duplicates (rule 4); of `2606.13614v1.pdf`
    /// and `mot.pdf` the newer wins (rule 5): **`mot.pdf`**.
    #[test]
    fn the_main_file_of_the_real_case_is_the_later_version_with_authors() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut store = new_store();
        let arxiv = Some("2606.13614");
        for e in [
            on_disk(d, "2606.13614v1 (1).pdf", "fp-v1", 100, true, arxiv),
            on_disk(d, "2606.13614v1.pdf", "fp-v1", 90, true, arxiv),
            on_disk(d, "mot (1).pdf", "fp-mot", 300, true, arxiv),
            on_disk(d, "mot.pdf", "fp-mot", 200, true, arxiv),
        ] {
            store.reconcile_file(&e);
        }
        store.reconcile_file(&on_disk(d, "mot_tmlr.pdf", "fp-tmlr", 400, false, None));
        assert_eq!(store.papers.len(), 2);
        assert_eq!(names(&store, 0)[0], "mot.pdf", "chosen as the files attach");
        let keep = store.papers[0].id.clone();
        let tmlr = store.papers[1].id.clone();
        let dto = store.merge_papers(&keep, &[tmlr]).unwrap();
        assert_eq!(dto.files.len(), 5, "every file stays");
        assert_eq!(dto.files[0].name, "mot.pdf");

        // Two byte-identical pairs: each shows as one row.
        let copy_of = |name: &str| {
            dto.files
                .iter()
                .find(|file| file.name == name)
                .and_then(|file| file.duplicate_of.clone())
        };
        let path = |name: &str| Some(d.join(name).to_string_lossy().into_owned());
        assert_eq!(copy_of("mot (1).pdf"), path("mot.pdf"));
        assert_eq!(copy_of("2606.13614v1 (1).pdf"), path("2606.13614v1.pdf"));
        assert_eq!(copy_of("mot.pdf"), None);
        assert_eq!(copy_of("2606.13614v1.pdf"), None);
        assert_eq!(copy_of("mot_tmlr.pdf"), None);
        let rows = store.paper_rows(&PaperFilterDto::default()).rows;
        assert_eq!(rows[0].file_count, 3, "distinct contents");
        // The DTO's grouping is never stored.
        assert!(store.papers[0]
            .files
            .iter()
            .all(|f| f.duplicate_of.is_none()));
    }

    #[test]
    fn the_published_version_becomes_main_when_it_attaches() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut store = new_store();
        let mut pre = on_disk(d, "2401.00001v3.pdf", "fp-pre", 900, true, None);
        pre.doi = Some("10.1103/physrevlett.1.1".into());
        pre.doi_source = Some(MetaSource::Filename);
        store.reconcile_file(&pre);
        // An older file whose page names its publisher.
        let mut journal = on_disk(d, "prl.pdf", "fp-prl", 10, false, None);
        journal.doi = Some("10.1103/physrevlett.1.1".into());
        journal.doi_source = Some(MetaSource::FirstPage);
        store.reconcile_file(&journal);
        assert_eq!(store.papers.len(), 1);
        assert_eq!(names(&store, 0), ["prl.pdf", "2401.00001v3.pdf"]);
        assert!(
            store.papers[0].files[1].traits.dois.is_empty(),
            "the name's DOI is not the file's own"
        );

        // The classifier naming a publisher does it too, without a DOI.
        let mut store = new_store();
        let mut v2 = on_disk(d, "a_v2.pdf", "fp-a2", 900, true, Some("2401.00002"));
        v2.paper_reason = Some("arXiv:2401.00002 on the first page".into());
        store.reconcile_file(&v2);
        let mut published = on_disk(d, "a.pdf", "fp-a", 10, false, Some("2401.00002"));
        published.arxiv_source = Some(MetaSource::Filename);
        published.paper_reason = Some("an APS journal on the first page".into());
        store.reconcile_file(&published);
        assert_eq!(names(&store, 0), ["a.pdf", "a_v2.pdf"]);
    }

    #[test]
    fn the_users_main_file_is_never_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut store = new_store();
        let arxiv = Some("2401.00003");
        store.reconcile_file(&on_disk(d, "b_v1.pdf", "fp-1", 1, true, arxiv));
        store.reconcile_file(&on_disk(d, "b_v2.pdf", "fp-2", 2, true, arxiv));
        assert_eq!(names(&store, 0), ["b_v2.pdf", "b_v1.pdf"]);
        let id = store.papers[0].id.clone();
        let v1 = d.join("b_v1.pdf").to_string_lossy().into_owned();
        store.set_primary_file(&id, &v1).unwrap();
        // A later version arrives, a rescan runs, the load sweep runs, the
        // record takes part in a merge: the user's choice stands.
        store.reconcile_file(&on_disk(d, "b_v3.pdf", "fp-3", 3, true, arxiv));
        store.reconcile_file(&on_disk(d, "b_v2.pdf", "fp-2", 2, true, arxiv));
        store.load_sweep(1_800_000_000);
        let other = store.reconcile_file(&on_disk(d, "c.pdf", "fp-c", 9, true, None));
        assert!(other);
        let c = store.papers[1].id.clone();
        store.merge_papers(&id, &[c]).unwrap();
        assert_eq!(names(&store, 0)[0], "b_v1.pdf");

        // Merged into a record without a choice, the choice comes along.
        let mut store = new_store();
        store.reconcile_file(&on_disk(d, "b_v1.pdf", "fp-1", 1, true, arxiv));
        store.reconcile_file(&on_disk(d, "c.pdf", "fp-c", 9, true, None));
        let (b, c) = (store.papers[0].id.clone(), store.papers[1].id.clone());
        store.set_primary_file(&b, &v1).unwrap();
        let dto = store.merge_papers(&c, &[b]).unwrap();
        assert_eq!(dto.files[0].name, "b_v1.pdf");
        // Split back, the record it went into chooses by the rules again.
        let back = store.unmerge(&c).unwrap();
        let survivor = back.iter().find(|p| p.id == c).unwrap();
        assert_eq!(survivor.files.len(), 1);
        assert_eq!(store.papers[store.index_of(&c).unwrap()].primary_path, None);
    }

    #[test]
    fn the_load_sweep_chooses_the_main_file_of_records_without_a_choice() {
        let mut store = new_store();
        let mut paper = PaperRecord::new();
        let mut old = PaperFile {
            path: "/l/x (1).pdf".into(),
            name: "x (1).pdf".into(),
            mtime_ms: 5,
            fingerprint: Some("x".into()),
            ..Default::default()
        };
        old.traits.authors = true;
        let mut good = old.clone();
        good.path = "/l/x.pdf".into();
        good.name = "x.pdf".into();
        good.fingerprint = Some("y".into());
        paper.files = vec![old, good];
        paper.cite_key = "x".into();
        store.papers.push(paper);
        assert!(store.load_sweep(1_800_000_000));
        assert_eq!(names(&store, 0), ["x.pdf", "x (1).pdf"]);
        assert!(!store.load_sweep(1_800_000_000), "settled");
    }

    fn missing_store(now: u64, days_ago: u64) -> PaperStore {
        let mut store = new_store();
        let mut paper = PaperRecord::new();
        paper.files.push(PaperFile {
            path: "/l/gone.pdf".into(),
            name: "gone.pdf".into(),
            missing: true,
            missing_since: Some(format_rfc3339(now - days_ago * 24 * 60 * 60)),
            ..Default::default()
        });
        paper.cite_key = "gone".into();
        store.papers.push(paper);
        store
    }

    #[test]
    fn the_missing_clock_starts_when_a_file_goes_and_stops_when_it_is_back() {
        let mut store = new_store();
        store.reconcile_file(&entry("/l/a.pdf", Some("fp-a")));
        assert!(store.mark_missing(&HashSet::new()));
        let since = store.papers[0].files[0].missing_since.clone();
        assert!(since.is_some());
        // Still gone: the clock keeps its start.
        store.mark_missing(&HashSet::new());
        assert_eq!(store.papers[0].files[0].missing_since, since);
        // Back: cleared, and so on a reattach.
        let present: HashSet<PathBuf> = [PathBuf::from("/l/a.pdf")].into();
        assert!(store.mark_missing(&present));
        assert_eq!(store.papers[0].files[0].missing_since, None);
        store.mark_missing(&HashSet::new());
        store.reconcile_file(&entry("/l/a.pdf", Some("fp-a")));
        assert!(!store.papers[0].files[0].missing);
        assert_eq!(store.papers[0].files[0].missing_since, None);
        // A file missing from before the clock was kept starts it at load.
        store.papers[0].files[0].missing = true;
        assert!(store.load_sweep(1_800_000_000));
        assert_eq!(
            store.papers[0].files[0].missing_since.as_deref(),
            Some(format_rfc3339(1_800_000_000).as_str())
        );
    }

    #[test]
    fn an_item_gone_for_a_week_is_removed_unless_the_user_has_a_stake() {
        let now = 1_800_000_000;
        // Six days: kept. Seven: removed, by the sweep or after a scan.
        assert!(!missing_store(now, 6).remove_long_missing(now));
        let mut store = missing_store(now, 7);
        assert!(store.remove_long_missing(now));
        assert!(store.papers.is_empty());
        let mut store = missing_store(now, 30);
        assert!(store.load_sweep(now));
        assert!(store.papers.is_empty());
        let mut store = missing_store(now, 0);
        store.papers[0].files[0].missing_since =
            Some(format_rfc3339(now_secs_for_tests() - 8 * 24 * 60 * 60));
        assert!(store.mark_missing(&HashSet::new()));
        assert!(store.papers.is_empty(), "the scan's sweep removes it too");

        // One file still on disk keeps the item.
        let mut store = missing_store(now, 30);
        store.papers[0].files.push(PaperFile {
            path: "/l/here.pdf".into(),
            name: "here.pdf".into(),
            ..Default::default()
        });
        assert!(!store.remove_long_missing(now));

        // Every stake the user can have keeps it.
        type Stake = fn(&mut PaperStore);
        let stakes: [(&str, Stake); 7] = [
            ("in a project", |store| {
                let id = store.papers[0].id.clone();
                let project = store.create_project("P").unwrap().id;
                store.set_membership(&id, &project, true).unwrap();
            }),
            ("a user edit", |store| {
                store.papers[0].meta.title = Some(MetaField {
                    value: "Mine".into(),
                    source: MetaSource::User,
                });
            }),
            ("a pinned key", |store| {
                store.papers[0].cite_key_pinned = true
            }),
            ("an alias", |store| {
                store.papers[0]
                    .alias_ids
                    .push(CitationIdDto::ArXiv("2401.00001".into()));
            }),
            ("a merged record", |store| {
                let snapshot = PaperRecord::new();
                store.papers[0].merged_from.push(snapshot);
            }),
            ("parked candidates", |store| {
                let found = candidate(MetaSource::Crossref, "T", &[("A", "B")], Some(2020));
                store.papers[0].candidates.push(found);
            }),
            ("a file on an unmounted volume", |store| {
                store.papers[0].files[0].path = "/Volumes/speedyf-not-mounted-x/gone.pdf".into();
            }),
        ];
        for (what, stake) in stakes {
            let mut store = missing_store(now, 30);
            stake(&mut store);
            assert!(!store.remove_long_missing(now), "{what}");
            assert_eq!(store.papers.len(), 1, "{what}");
        }

        // A read-only store removes nothing.
        let mut store = missing_store(now, 30);
        store.block_writes_for_tests();
        assert!(!store.remove_long_missing(now));
    }

    fn now_secs_for_tests() -> u64 {
        parse_rfc3339(&now_rfc3339()).unwrap()
    }
}
