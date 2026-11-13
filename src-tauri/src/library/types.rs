//! Wire types for paper records and projects.
//!
//! Mirrors `src/types/engine.ts` field for field; change both or neither.
//! Everything here crosses the IPC boundary as camelCase JSON. The on-disk
//! shape of `library.json` lives in `store.rs` and may differ.

use serde::{Deserialize, Deserializer, Serialize};

/// Where a metadata value came from. Ordered weakest to strongest: a value
/// never loses to a weaker source (see `rank`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum MetaSource {
    Filename,
    FirstPage,
    PdfInfo,
    /// Derived from an identifier the file carries: the year in an arXiv id
    /// or a DOI, the year and DOI in an Elsevier PII. Certain as far as it
    /// goes, which is never further than a year.
    Identifier,
    /// The XMP packet embedded in the PDF: what the publisher's typesetter
    /// wrote, when there is one.
    Xmp,
    OpenLibrary,
    Openalex,
    Dblp,
    SemanticScholar,
    Arxiv,
    Datacite,
    Inspire,
    Crossref,
    User,
}

impl MetaSource {
    /// Higher wins. `User` beats every lookup; every lookup beats every local
    /// source; among the local ones XMP beats the Info dictionary beats the
    /// first-page guess. Among lookups the registration agencies (Crossref,
    /// DataCite) and INSPIRE's curated records rank above aggregators.
    pub fn rank(self) -> u8 {
        match self {
            MetaSource::Filename => 0,
            MetaSource::FirstPage => 1,
            MetaSource::PdfInfo => 2,
            MetaSource::Identifier => 3,
            MetaSource::Xmp => 4,
            MetaSource::OpenLibrary => 5,
            MetaSource::Openalex => 6,
            MetaSource::Dblp => 7,
            MetaSource::SemanticScholar => 8,
            MetaSource::Arxiv => 9,
            MetaSource::Datacite => 10,
            MetaSource::Inspire => 11,
            MetaSource::Crossref => 12,
            MetaSource::User => 13,
        }
    }

    /// Whether this is something fetched rather than read from the file.
    pub fn is_lookup(self) -> bool {
        self.rank() >= MetaSource::OpenLibrary.rank() && self != MetaSource::User
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MetaField<T> {
    pub value: T,
    pub source: MetaSource,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Author {
    pub family: String,
    #[serde(default)]
    pub given: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PaperKind {
    Article,
    Inproceedings,
    Book,
    Thesis,
    Misc,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaperMeta {
    #[serde(default)]
    pub title: Option<MetaField<String>>,
    #[serde(default)]
    pub authors: Option<MetaField<Vec<Author>>>,
    #[serde(default)]
    pub year: Option<MetaField<i32>>,
    #[serde(default)]
    pub venue: Option<MetaField<String>>,
    #[serde(default)]
    pub volume: Option<MetaField<String>>,
    #[serde(default)]
    pub issue: Option<MetaField<String>>,
    #[serde(default)]
    pub pages: Option<MetaField<String>>,
    #[serde(default)]
    pub publisher: Option<MetaField<String>>,
    #[serde(default, rename = "abstract")]
    pub abstract_: Option<MetaField<String>>,
    #[serde(default)]
    pub kind: Option<MetaField<PaperKind>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaperFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: u64,
    /// Size plus a hash of the first and last 64 KiB. Absent for a file the
    /// scan has not fingerprinted yet.
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub missing: bool,
}

/// The full record, for the detail column and the Paper panel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaperDto {
    pub id: String,
    pub files: Vec<PaperFile>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    /// For books: the ISBN found on the first pages, normalized to digits
    /// (and a trailing X), 10 or 13 long.
    #[serde(default)]
    pub isbn: Option<String>,
    pub meta: PaperMeta,
    pub cite_key: String,
    pub cite_key_pinned: bool,
    pub project_ids: Vec<String>,
    pub added_at: String,
    pub updated_at: String,
    /// Metadata a BibTeX entry needs and this paper lacks: "authors", "year",
    /// "title", "venue". Empty when the entry would be complete.
    pub incomplete: Vec<String>,
    /// Whether this record is treated as a paper: the user's override when
    /// there is one, else the scan's verdict (`paper` or `likely`).
    pub is_paper: bool,
    /// Why, in a few words: the scan's reason, or "marked by you".
    pub paper_reason: String,
    /// The user's say, when they had one. Null means the scan decides.
    pub paper_override: Option<bool>,
    /// Matches a title search found but did not apply, for the user to pick
    /// from. Empty once one is applied or they are discarded.
    #[serde(default)]
    pub candidates: Vec<LookupCandidateDto>,
    /// How many records were merged into this one. Non-zero means the item
    /// can be split back.
    #[serde(default)]
    pub merged_count: u32,
}

/// One row of the paper list. Display strings, already resolved, so the list
/// never has to know the precedence rules.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaperRowDto {
    pub id: String,
    /// The title, or the file name when there is none.
    pub title: String,
    /// "Jarzynski", "Evans, Cohen & Morriss", "Marchetti et al." — or "".
    pub authors: String,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub cite_key: String,
    /// A BibTeX `alpha`-style label from the full author list and year:
    /// "Jar97", "ZD24", "ABC+19". Not unique; a view suffixes repeats.
    pub alpha_label: String,
    pub project_ids: Vec<String>,
    /// Every file is missing.
    pub missing: bool,
    pub has_identifier: bool,
    pub incomplete_count: u32,
    pub is_paper: bool,
    /// Parked title-search matches waiting for the user.
    pub candidate_count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub order: u32,
    pub paper_count: u32,
    pub bib_path: Option<String>,
    pub incomplete_count: u32,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaperFilterDto {
    /// Only papers in this project.
    #[serde(default)]
    pub project: Option<String>,
    /// Only papers in no project. Ignored when `project` is set.
    #[serde(default)]
    pub unfiled: bool,
    /// Only records that are not papers. Ignored when `project` is set.
    /// Without it, and without `project`, non-papers are left out.
    #[serde(default)]
    pub others: bool,
    /// Matched against title, authors, year and cite key; empty matches all.
    #[serde(default)]
    pub query: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaperListDto {
    pub rows: Vec<PaperRowDto>,
    pub total: u32,
}

fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

/// A user edit. A field left out is untouched; `null` clears it. Every field
/// set here is recorded with source `user` and is never overwritten by a scan
/// or a lookup afterwards.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaPatchDto {
    #[serde(default, deserialize_with = "double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub authors: Option<Option<Vec<Author>>>,
    #[serde(default, deserialize_with = "double_option")]
    pub year: Option<Option<i32>>,
    #[serde(default, deserialize_with = "double_option")]
    pub venue: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub volume: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub issue: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub pages: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub publisher: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option", rename = "abstract")]
    pub abstract_: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub kind: Option<Option<PaperKind>>,
    #[serde(default, deserialize_with = "double_option")]
    pub doi: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub arxiv: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub isbn: Option<Option<String>>,
    /// Setting a key pins it. Clearing (`null`) unpins and regenerates.
    #[serde(default, deserialize_with = "double_option")]
    pub cite_key: Option<Option<String>>,
    /// `true`/`false` overrides the scan's verdict; `null` hands it back.
    #[serde(default, deserialize_with = "double_option")]
    pub is_paper: Option<Option<bool>>,
}

/// What an online lookup found.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LookupCandidateDto {
    pub source: MetaSource,
    pub title: String,
    pub authors: Vec<Author>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub publisher: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_: Option<String>,
    pub kind: Option<PaperKind>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
// A result is built once per lookup and serialized straight away; the size
// gap between an applied paper and `Nothing` costs nothing that matters.
#[allow(clippy::large_enum_variant)]
pub enum LookupResultDto {
    /// An identifier lookup resolved to one work and it has been applied.
    Applied { paper: PaperDto },
    /// A title lookup; the user picks one or none.
    Candidates { candidates: Vec<LookupCandidateDto> },
    /// Nothing found, or the paper has nothing to look up with.
    Nothing,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LookupMode {
    /// By DOI or arXiv id. Applies automatically.
    Id,
    /// By title. Returns candidates.
    Title,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CitationLinksDto {
    pub cites: Vec<PaperRowDto>,
    pub cited_by: Vec<PaperRowDto>,
    /// Identifiers this paper cites that match no paper in the library.
    pub unresolved: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDataInfoDto {
    pub dir: String,
    pub is_default: bool,
}

/// Counts for the two fixed rows of the projects column.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCountsDto {
    /// every paper
    pub all: u32,
    /// papers in no project
    pub unfiled: u32,
    /// records the scan does not take for papers (and the user has not)
    pub others: u32,
}

/// What a whole-project lookup did. Papers without an identifier are
/// skipped, never searched by title (projects.md §4).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LookupBatchDto {
    pub attempted: u32,
    pub applied: u32,
    pub skipped: u32,
    pub failed: u32,
    /// Title searches that found something but nothing sure enough to apply;
    /// the candidates wait on the paper.
    pub parked: u32,
}

/// Whether an open PDF looks like a paper (projects.md: SpeedyF is a viewer
/// first; the reader only offers to file what looks like a paper).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PapernessVerdict {
    /// An identifier or a publisher's name on the first page.
    Paper,
    /// The shape of a paper with nobody's name on it.
    Likely,
    /// Nothing says so. The default for a manual, a form, a scan.
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PapernessDto {
    pub verdict: PapernessVerdict,
    /// Why, in a few words, for the panel to show. Empty for `Unknown`.
    pub reason: String,
    /// What kind of item it looks like, when the signals say: a thesis, a
    /// book, an article. None when only "some kind of paper" is known.
    #[serde(default)]
    pub kind: Option<PaperKind>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
}

/// One citation between two items of a graph: `from` cites `to`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeDto {
    pub from: String,
    pub to: String,
}

/// The citation graph of a project, or of the whole library.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphDto {
    pub nodes: Vec<PaperRowDto>,
    /// Edges between nodes only; a citation to something outside the set
    /// is counted in `external`, never drawn.
    pub edges: Vec<GraphEdgeDto>,
    pub external: u32,
    /// Nodes whose text has not been indexed yet, so their outgoing
    /// citations are not known. The view says so rather than drawing them
    /// as uncited.
    pub unindexed: u32,
}

/// Records the engine suspects are one work: a preprint and its journal
/// version, or the same file saved twice under different bytes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroupDto {
    pub paper_ids: Vec<String>,
    /// Why, in a few words: "same title", "arXiv 2404.01286 is the preprint of this DOI".
    pub reason: String,
}

/// A work cited by items in the view but not in the library.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MissingWorkDto {
    /// Stable within one answer: the identifier, or a key built from the
    /// reference text. Used as the ghost node's id.
    pub key: String,
    /// The reference as printed, from one of the citing items.
    pub text: Option<String>,
    pub year: Option<i32>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    /// The items in the view that cite it, in list order.
    pub cited_by: Vec<String>,
}
