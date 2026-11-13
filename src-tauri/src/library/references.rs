//! A paper's reference list, as the reference provider extension read it
//! (projects.md sec. 8, EXTENSIONS.md "referenceProvider").
//!
//! Most physics references print neither a DOI nor an arXiv id, so the
//! identifiers alone miss most of a bibliography. Cutting a reference list
//! into entries is parsing, and parsing lives in an extension on the
//! permissioned API: the built-in References extension is handed each
//! library document's stored text (never its path) at idle and answers with
//! its entries - label, text, year. What the engine keeps, per entry, is
//! what a citation can be resolved by without a network: the identifiers
//! printed in it, the DOI an APS journal reference spells out mechanically
//! (an identifier rule, not parsing, so it also runs over the whole text
//! for `cites` with no extension at all), its year, and its text in a
//! compact form a library title can be found in.
//!
//! Each refs sidecar records the provider (`<extension id>@<version>`) that
//! produced it. Only sidecars from the active provider are read, so turning
//! the extension off turns title matching off (the sidecars are ignored,
//! not deleted, and count again when it comes back), and a new version of
//! it reads the library again.

use crate::engine::citation::{find_explicit_citations, fold_arxiv_doi, parse_citation_id};
use crate::engine::types::CitationIdDto;
use crate::errors::{AppError, AppResult};
use crate::search::librarytext::{read_sidecar_text, refs_path};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// No paper has more; a provider that claims more is misreading something.
pub const MAX_ENTRIES: usize = 1000;
/// An entry's text is cut here. A real entry, even one listing thirty
/// authors, is shorter.
pub const MAX_ENTRY_CHARS: usize = 1500;
/// The most of a document's text a provider is handed. Reference lists are
/// at the end, so a longer text keeps its tail.
pub const MAX_PROVIDER_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// A label longer than this is not a label.
const MAX_LABEL_CHARS: usize = 32;

/// One reference as a provider extension returns it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvidedRef {
    #[serde(default)]
    pub label: Option<String>,
    pub text: String,
    #[serde(default)]
    pub year: Option<i32>,
}

/// One reference as kept in the refs sidecar: what resolving it needs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StoredRef {
    pub label: Option<String>,
    /// `compact` of the entry's text, which a library title is looked for in.
    pub compact: String,
    pub year: Option<i32>,
    /// The DOIs and arXiv ids printed in the entry, and the DOI its APS
    /// journal reference stands for; never the citing paper's own.
    pub ids: Vec<CitationIdDto>,
    /// The entry as printed, cut at [`MAX_STORED_TEXT_CHARS`], for the
    /// missing-works list to show. Empty in a sidecar from before it was
    /// kept (which a version bump retires).
    #[serde(default)]
    pub text: String,
}

/// How much of an entry's printed text the sidecar keeps for display.
pub const MAX_STORED_TEXT_CHARS: usize = 400;

/// A document's parsed reference lists, by path, for one citation query.
pub type DocRefs = HashMap<PathBuf, Arc<Vec<StoredRef>>>;

/// A provider's entries, ready to store: each one's compact text, year and
/// identifiers, the citing paper's own identifiers left out. An entry with
/// no text is dropped, an over-long one cut, an implausible year or label
/// dropped; at most `MAX_ENTRIES`.
pub fn stored_refs(
    provided: &[ProvidedRef],
    own_doi: Option<&str>,
    own_arxiv: Option<&str>,
) -> Vec<StoredRef> {
    let own_doi = own_doi.map(str::to_ascii_lowercase);
    let own = |id: &CitationIdDto| match id {
        CitationIdDto::Doi(value) => own_doi.as_deref() == Some(value.as_str()),
        CitationIdDto::ArXiv(value) => own_arxiv == Some(value.as_str()),
    };
    provided
        .iter()
        .filter_map(|entry| {
            let text: String = entry.text.trim().chars().take(MAX_ENTRY_CHARS).collect();
            if text.is_empty() {
                return None;
            }
            let mut ids: Vec<CitationIdDto> = Vec::new();
            let explicit = find_explicit_citations(&text)
                .into_iter()
                .map(|occurrence| fold_arxiv_doi(occurrence.id));
            for id in explicit
                .chain(bare_dois(&text))
                .chain(journal_doi(&text).map(CitationIdDto::Doi))
            {
                if !own(&id) && !ids.contains(&id) {
                    ids.push(id);
                }
            }
            let label = entry
                .label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty() && label.chars().count() <= MAX_LABEL_CHARS)
                .map(str::to_owned);
            Some(StoredRef {
                label,
                compact: compact(&text),
                year: entry.year.filter(|year| (1400..=2100).contains(year)),
                ids,
                text: display_text(&text),
            })
        })
        .take(MAX_ENTRIES)
        .collect()
}

/// An entry's text for display: whitespace collapsed, at most
/// [`MAX_STORED_TEXT_CHARS`] characters.
fn display_text(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_STORED_TEXT_CHARS {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(MAX_STORED_TEXT_CHARS - 3).collect();
    cut.push_str("...");
    cut
}

/* ---- Identifiers ---- */

/// APS journals, as printed (lower-case, periods as spaces), and the code
/// their DOIs use: `10.1103/<code>.<volume>.<article>`. Longer names first,
/// so "phys rev lett" is not read as "phys rev".
const APS_JOURNALS: &[(&str, &str)] = &[
    ("physical review letters", "physrevlett"),
    ("physical review research", "physrevresearch"),
    ("physical review applied", "physrevapplied"),
    ("physical review fluids", "physrevfluids"),
    ("physical review materials", "physrevmaterials"),
    ("physical review x", "physrevx"),
    ("physical review a", "physreva"),
    ("physical review b", "physrevb"),
    ("physical review c", "physrevc"),
    ("physical review d", "physrevd"),
    ("physical review e", "physreve"),
    ("physical review", "physrev"),
    ("reviews of modern physics", "revmodphys"),
    ("phys rev lett", "physrevlett"),
    ("phys rev research", "physrevresearch"),
    ("phys rev res", "physrevresearch"),
    ("phys rev applied", "physrevapplied"),
    ("phys rev appl", "physrevapplied"),
    ("phys rev fluids", "physrevfluids"),
    ("phys rev materials", "physrevmaterials"),
    ("phys rev mater", "physrevmaterials"),
    ("phys rev x", "physrevx"),
    ("phys rev a", "physreva"),
    ("phys rev b", "physrevb"),
    ("phys rev c", "physrevc"),
    ("phys rev d", "physrevd"),
    ("phys rev e", "physreve"),
    ("phys rev", "physrev"),
    ("rev mod phys", "revmodphys"),
    ("prx quantum", "prxquantum"),
];

/// The DOI an APS journal reference stands for, which APS assigns
/// mechanically: "Phys. Rev. Lett. 133, 057102" is
/// `10.1103/physrevlett.133.057102`, lower-case as the library keeps DOIs.
/// The volume may be followed by an issue or year in parentheses and the
/// article id may carry a letter ("L041301"). Nothing for any other
/// publisher, whose DOIs cannot be spelled out from a citation.
pub fn journal_doi(text: &str) -> Option<String> {
    journal_dois(text).into_iter().next()
}

/// Every DOI the APS journal references in a text stand for, in order,
/// each once. Run over a whole document's text for `cites`, so the rule
/// works with no reference provider at all.
pub fn journal_dois(text: &str) -> Vec<String> {
    // Lower-case, periods as spaces, whitespace collapsed: "Phys. Rev.
    // Lett. 133, 057102" is "phys rev lett 133, 057102". A period between
    // digits stays, so "105, 10.1103/..." is not read as article 10.
    let mut flat = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut previous = ' ';
    while let Some(c) = chars.next() {
        let between_digits =
            c == '.' && previous.is_ascii_digit() && chars.peek().is_some_and(char::is_ascii_digit);
        previous = c;
        let c = if (c == '.' && !between_digits) || c.is_whitespace() {
            ' '
        } else {
            c.to_ascii_lowercase()
        };
        if c == ' ' && (flat.is_empty() || flat.ends_with(' ')) {
            continue;
        }
        flat.push(c);
    }
    let mut out: Vec<String> = Vec::new();
    let mut previous = ' ';
    for (at, c) in flat.char_indices() {
        let word_start = !previous.is_ascii_alphanumeric();
        previous = c;
        // Every journal name starts with "p" or "r".
        if !word_start || !matches!(c, 'p' | 'r') {
            continue;
        }
        for (name, code) in APS_JOURNALS {
            let Some(rest) = flat[at..].strip_prefix(name) else {
                continue;
            };
            if rest
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            {
                continue;
            }
            if let Some((volume, article)) = volume_and_article(rest) {
                let doi = format!("10.1103/{code}.{volume}.{article}");
                if !out.contains(&doi) {
                    out.push(doi);
                }
            }
            break;
        }
    }
    out
}

/// "133, 057102" or "133 (2024) 057102" or "105, L052103" after a journal
/// name: the volume and the article id, the id lower-cased.
fn volume_and_article(rest: &str) -> Option<(&str, String)> {
    let rest = rest.trim_start();
    let volume_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=4).contains(&volume_len) {
        return None;
    }
    let volume = &rest[..volume_len];
    let mut rest = rest[volume_len..].trim_start_matches([' ', ',', ':']);
    // An issue or a year in parentheses: "94(5)", "133 (2024)".
    if let Some(inner) = rest.strip_prefix('(') {
        let close = inner.find(')')?;
        if close == 0 || close > 4 || !inner[..close].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest = inner[close + 1..].trim_start_matches([' ', ',', ':']);
    }
    let letter = rest
        .chars()
        .next()
        .filter(|c| c.is_ascii_lowercase())
        .map_or(0, char::len_utf8);
    let digits = rest[letter..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if !(1..=7).contains(&digits)
        || rest[letter + digits..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '/'))
    {
        return None;
    }
    Some((volume, rest[..letter + digits].to_owned()))
}

/// Text as references are matched: lower-case, accents stripped, and
/// nothing but ASCII letters and digits, so line breaks, punctuation and
/// "higher-order" against "higherorder" never matter.
pub fn compact(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// DOIs printed without a `doi:` or URL prefix ("Physical Review E 105,
/// 10.1103/physreve.105.034130"). Not trusted in running text, where
/// `find_explicit_citations` insists on the prefix; inside a reference
/// entry a word starting `10.NNNN/` is a DOI.
fn bare_dois(text: &str) -> impl Iterator<Item = CitationIdDto> + '_ {
    text.split_whitespace()
        .map(|word| word.trim_start_matches(['(', '[', '<', ',', ';']))
        .filter(|word| word.starts_with("10."))
        .filter_map(parse_citation_id)
        .filter(|id| matches!(id, CitationIdDto::Doi(_)))
}

/* ---- The sidecar ---- */

/// v2: entries come from a provider extension, which the sidecar names.
/// A v1 sidecar (the engine's own parser) is never read. v3: each entry
/// keeps its printed text, so an older sidecar is read again.
const REFS_VERSION: u32 = 3;
/// Refuse to load something implausible as one document's references.
const MAX_REFS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct RefsSidecar {
    version: u32,
    /// Checked on load, as the text sidecar's are: these references are
    /// valid only for the bytes they were read from.
    path: PathBuf,
    mtime_ms: u64,
    size: u64,
    /// `<extension id>@<version>` of the provider that read them.
    provider: String,
    refs: Vec<StoredRef>,
}

/// Write a document's references beside its text sidecar.
pub fn write_refs(
    app_data_dir: &Path,
    document: &Path,
    mtime_ms: u64,
    size: u64,
    provider: &str,
    refs: &[StoredRef],
) -> AppResult<()> {
    let target = refs_path(app_data_dir, document);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Io(format!("cannot create text index directory: {e}")))?;
    }
    let sidecar = RefsSidecar {
        version: REFS_VERSION,
        path: document.to_path_buf(),
        mtime_ms,
        size,
        provider: provider.to_owned(),
        refs: refs.to_vec(),
    };
    let bytes = serde_json::to_vec(&sidecar)
        .map_err(|e| AppError::Internal(format!("cannot serialize references: {e}")))?;
    std::fs::write(&target, bytes)
        .map_err(|e| AppError::Io(format!("cannot write references: {e}")))
}

/// A document's references, if a sidecar for exactly these bytes, read by
/// exactly this provider, exists.
pub fn read_refs(
    app_data_dir: &Path,
    document: &Path,
    mtime_ms: u64,
    size: u64,
    provider: &str,
) -> Option<Vec<StoredRef>> {
    let target = refs_path(app_data_dir, document);
    if std::fs::metadata(&target).ok()?.len() > MAX_REFS_BYTES {
        return None;
    }
    let sidecar: RefsSidecar = serde_json::from_slice(&std::fs::read(&target).ok()?).ok()?;
    (sidecar.version == REFS_VERSION
        && sidecar.path == document
        && sidecar.mtime_ms == mtime_ms
        && sidecar.size == size
        && sidecar.provider == provider)
        .then_some(sidecar.refs)
}

/// The refs sidecars already read, so a citation query does not re-read
/// a few hundred files each time. Keyed by path and checked against the
/// document's stamp and the active provider; storing a provider's answer
/// and the text pass `forget` a document. A missing sidecar is remembered
/// too, and is what makes a document pending for the provider.
#[derive(Default)]
pub struct RefsCache {
    provider: Option<String>,
    entries: HashMap<PathBuf, CachedRefs>,
}

/// A document's stamp `(mtime_ms, size)` and its references, if a sidecar
/// exists (an empty list when the provider found none).
type CachedRefs = ((u64, u64), Option<Arc<Vec<StoredRef>>>);

impl RefsCache {
    /// Bring the cache up to date for `entries` and `provider`, reading only
    /// what is not cached for its current stamp. Documents no longer in
    /// `entries`, and everything when the provider changed, are dropped.
    /// With no provider nothing is read and nothing is kept.
    fn sync(
        &mut self,
        app_data_dir: &Path,
        entries: &[super::LibraryEntry],
        provider: Option<&str>,
    ) {
        if self.provider.as_deref() != provider {
            self.provider = provider.map(str::to_owned);
            self.entries.clear();
        }
        let Some(provider) = provider else {
            return;
        };
        let mut keep: HashMap<PathBuf, CachedRefs> = HashMap::with_capacity(entries.len());
        for entry in entries {
            if entry.cites.is_none() {
                continue;
            }
            let stamp = (entry.mtime_ms, entry.size);
            let refs = match self.entries.remove(&entry.path) {
                Some((cached, refs)) if cached == stamp => refs,
                _ => read_refs(
                    app_data_dir,
                    &entry.path,
                    entry.mtime_ms,
                    entry.size,
                    provider,
                )
                .map(Arc::new),
            };
            keep.insert(entry.path.clone(), (stamp, refs));
        }
        self.entries = keep;
    }

    /// The references of every document whose cites are known and whose
    /// list the active provider has read and found entries in. Empty with
    /// no provider: then nothing is matched by title.
    pub fn load(
        &mut self,
        app_data_dir: &Path,
        entries: &[super::LibraryEntry],
        provider: Option<&str>,
    ) -> DocRefs {
        self.sync(app_data_dir, entries, provider);
        self.entries
            .iter()
            .filter_map(|(path, (_, refs))| {
                let refs = refs.as_ref().filter(|refs| !refs.is_empty())?;
                Some((path.clone(), refs.clone()))
            })
            .collect()
    }

    /// The documents whose text has been read but whose reference list the
    /// provider has not, in `entries` order.
    pub fn unread<'a>(
        &mut self,
        app_data_dir: &Path,
        entries: &'a [super::LibraryEntry],
        provider: &str,
    ) -> Vec<&'a super::LibraryEntry> {
        self.sync(app_data_dir, entries, Some(provider));
        entries
            .iter()
            .filter(|entry| {
                self.entries
                    .get(&entry.path)
                    .is_some_and(|(_, refs)| refs.is_none())
            })
            .collect()
    }

    pub fn forget(&mut self, document: &Path) {
        self.entries.remove(document);
    }
}

/* ---- The provider ---- */

/// A document handed to the provider: an opaque token and its text. The
/// token is all the extension ever sees of which document it is.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingRefsDto {
    pub token: String,
    pub text: String,
}

/// What a token stands for: one document's bytes, for one provider.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Issued {
    path: PathBuf,
    mtime_ms: u64,
    size: u64,
    provider: String,
}

/// A token is honoured, and its document kept from being handed out again,
/// this long: well past the extension's own 20 s, short enough that a
/// window closed mid-document does not hold it for the session.
const TOKEN_LIFETIME: Duration = Duration::from_secs(120);
/// Tokens outstanding at once; the oldest go first.
const MAX_TOKENS: usize = 256;

/// The engine's side of the reference provider: which provider is active,
/// and which documents it has been handed. Tokens map back to paths here
/// and nowhere else.
pub struct RefsProvider {
    active: Option<String>,
    issued: HashMap<String, (Issued, Instant)>,
    /// Documents the provider failed on (or whose text is gone), not
    /// offered again this session for the same bytes and provider.
    given_up: HashSet<Issued>,
    counter: u64,
    salt: u64,
}

impl Default for RefsProvider {
    fn default() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(0);
        Self {
            active: None,
            issued: HashMap::new(),
            given_up: HashSet::new(),
            counter: 0,
            salt: nanos ^ 0x9e37_79b9_7f4a_7c15,
        }
    }
}

/// What became of a provider's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreOutcome {
    /// Written to the refs sidecar, with this many entries.
    Stored(usize),
    /// Not written: the document changed, left the library, or the provider
    /// changed while it was being read. It is offered again when current.
    Stale,
    /// The provider gave up on it; not offered again this session.
    GivenUp,
}

impl RefsProvider {
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Set the active provider, `<extension id>@<version>`, or none. A
    /// change forgets every outstanding token and every give-up.
    pub fn set_active(&mut self, provider: Option<String>) {
        if self.active != provider {
            self.active = provider;
            self.issued.clear();
            self.given_up.clear();
        }
    }

    fn mint(&mut self) -> String {
        self.counter += 1;
        // Opaque, unguessable enough, and derived from nothing about the
        // document: a counter mixed with a per-session salt.
        let mut x = self.counter.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ self.salt;
        x ^= x >> 31;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^= x >> 29;
        format!("r{:x}{x:016x}", self.counter)
    }

    /// Up to `limit` documents for the provider to read: text indexed, no
    /// refs sidecar for these bytes from this provider, not already handed
    /// out and not given up on. Each comes with a fresh token and its
    /// normalized text (the last `MAX_PROVIDER_TEXT_BYTES` of a longer
    /// one). Nothing without an active provider.
    pub fn pending(
        &mut self,
        app_data_dir: &Path,
        entries: &[super::LibraryEntry],
        cache: &mut RefsCache,
        limit: usize,
        now: Instant,
    ) -> Vec<PendingRefsDto> {
        let Some(provider) = self.active.clone() else {
            return Vec::new();
        };
        self.issued
            .retain(|_, (_, at)| now.saturating_duration_since(*at) < TOKEN_LIFETIME);
        let mut out = Vec::new();
        for entry in cache.unread(app_data_dir, entries, &provider) {
            if out.len() >= limit {
                break;
            }
            let issued = Issued {
                path: entry.path.clone(),
                mtime_ms: entry.mtime_ms,
                size: entry.size,
                provider: provider.clone(),
            };
            if self.given_up.contains(&issued)
                || self.issued.values().any(|(other, _)| *other == issued)
            {
                continue;
            }
            // No text yet is not a failure: the text pass is still writing
            // it (every document's, after an upgrade re-extracts the
            // library). Giving up here left whole libraries without
            // reference lists until the next launch. Skip, and the next
            // poll finds it once the sidecar lands.
            if !entry.text_indexed {
                continue;
            }
            let Some(text) =
                read_sidecar_text(app_data_dir, &entry.path, entry.mtime_ms, entry.size)
            else {
                continue;
            };
            if self.issued.len() >= MAX_TOKENS {
                let oldest = self
                    .issued
                    .iter()
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(token, _)| token.clone());
                if let Some(oldest) = oldest {
                    self.issued.remove(&oldest);
                }
            }
            let token = self.mint();
            self.issued.insert(token.clone(), (issued, now));
            out.push(PendingRefsDto {
                token,
                text: tail(text, MAX_PROVIDER_TEXT_BYTES),
            });
        }
        out
    }

    /// Store the provider's answer for a token: its entries written to the
    /// refs sidecar (with `stored_refs`), or, for `None`, the document given
    /// up on for the session. `current` is the library's entry for the
    /// token's path now; an answer for bytes that are no longer the file's
    /// is not written. An unknown or expired token is an error.
    pub fn store(
        &mut self,
        app_data_dir: &Path,
        token: &str,
        current: impl FnOnce(&Path) -> Option<super::LibraryEntry>,
        provided: Option<&[ProvidedRef]>,
        cache: &mut RefsCache,
    ) -> AppResult<StoreOutcome> {
        let (issued, _) = self
            .issued
            .remove(token)
            .ok_or_else(|| AppError::Unsupported("unknown or expired document token".into()))?;
        if self.active.as_deref() != Some(issued.provider.as_str()) {
            return Ok(StoreOutcome::Stale);
        }
        let Some(entry) = current(&issued.path)
            .filter(|entry| entry.mtime_ms == issued.mtime_ms && entry.size == issued.size)
        else {
            return Ok(StoreOutcome::Stale);
        };
        let Some(provided) = provided else {
            self.given_up.insert(issued);
            return Ok(StoreOutcome::GivenUp);
        };
        let refs = stored_refs(provided, entry.doi.as_deref(), entry.arxiv.as_deref());
        write_refs(
            app_data_dir,
            &issued.path,
            issued.mtime_ms,
            issued.size,
            &issued.provider,
            &refs,
        )?;
        cache.forget(&issued.path);
        Ok(StoreOutcome::Stored(refs.len()))
    }
}

/// The last `max` bytes of a text, cut at a character boundary.
fn tail(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

/* ---- Matching titles ---- */

/// A title shorter than this, compacted, never matches: short titles are
/// generic ("Optimal transport") and would be found in unrelated entries.
const MIN_TITLE_CHARS: usize = 24;
/// Nor does one of fewer words.
const MIN_TITLE_WORDS: usize = 4;
/// A cited year and a paper's year further apart than this are two works.
const YEAR_SLACK: i32 = 2;
const PREFIX: usize = 12;

struct IndexedTitle {
    compact: String,
    paper: usize,
    year: Option<i32>,
}

/// Library titles, compacted and indexed by their first twelve characters,
/// for finding which paper a reference entry names.
pub struct TitleIndex {
    titles: Vec<IndexedTitle>,
    by_prefix: HashMap<[u8; PREFIX], Vec<usize>>,
}

impl TitleIndex {
    /// From `(paper index, title, year)`. Each title is indexed whole and,
    /// when it has a subtitle, by its main title before the colon too - a
    /// reference may drop or shorten the subtitle - each only if it is long
    /// enough to be specific (24 characters, 4 words).
    pub fn new<'a>(papers: impl IntoIterator<Item = (usize, &'a str, Option<i32>)>) -> Self {
        let mut titles = Vec::new();
        let mut by_prefix: HashMap<[u8; PREFIX], Vec<usize>> = HashMap::new();
        for (paper, title, year) in papers {
            let whole = matchable(title);
            let main = title
                .split_once(':')
                .and_then(|(main, _)| matchable(main))
                .filter(|main| Some(main) != whole.as_ref());
            for compact in [whole, main].into_iter().flatten() {
                let Ok(key) = <[u8; PREFIX]>::try_from(&compact.as_bytes()[..PREFIX]) else {
                    continue;
                };
                by_prefix.entry(key).or_default().push(titles.len());
                titles.push(IndexedTitle {
                    compact,
                    paper,
                    year,
                });
            }
        }
        Self { titles, by_prefix }
    }

    /// The paper whose title the entry contains: not `citing` itself nor
    /// another record under the same title, not a year more than two off when both are known, and the longest such
    /// title when several are found. One hash lookup per position of the
    /// entry's text.
    pub fn find(&self, entry: &StoredRef, citing: usize) -> Option<usize> {
        let text = entry.compact.as_bytes();
        // Another record under the citing paper's own title is a duplicate
        // or a running header taken for a title ("Under review as
        // submission to TMLR"), never what the entry cites.
        let own: Vec<&str> = self
            .titles
            .iter()
            .filter(|title| title.paper == citing)
            .map(|title| title.compact.as_str())
            .collect();
        let mut best: Option<&IndexedTitle> = None;
        for at in 0..text.len().saturating_sub(PREFIX - 1) {
            let Ok(key) = <[u8; PREFIX]>::try_from(&text[at..at + PREFIX]) else {
                continue;
            };
            let Some(candidates) = self.by_prefix.get(&key) else {
                continue;
            };
            for &index in candidates {
                let title = &self.titles[index];
                if title.paper == citing
                    || own.contains(&title.compact.as_str())
                    || !text[at..].starts_with(title.compact.as_bytes())
                {
                    continue;
                }
                if let (Some(cited), Some(own)) = (entry.year, title.year) {
                    if (cited - own).abs() > YEAR_SLACK {
                        continue;
                    }
                }
                let better = best.is_none_or(|current| {
                    title.compact.len() > current.compact.len()
                        || (title.compact.len() == current.compact.len()
                            && title.paper < current.paper)
                });
                if better {
                    best = Some(title);
                }
            }
        }
        best.map(|title| title.paper)
    }
}

/// A title's compact form, if it is specific enough to match on.
fn matchable(title: &str) -> Option<String> {
    let compact = compact(title);
    let words = super::titlematch::normalize(title).split(' ').count();
    (compact.len() >= MIN_TITLE_CHARS && words >= MIN_TITLE_WORDS).then_some(compact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::indexed_entry;

    const ZHONG: &str = "A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).";
    const PROVIDER: &str = "dev.speedyf.references@4.0.0";

    fn provided(label: Option<&str>, text: &str, year: Option<i32>) -> ProvidedRef {
        ProvidedRef {
            label: label.map(str::to_owned),
            text: text.to_owned(),
            year,
        }
    }

    fn reference(text: &str, year: Option<i32>) -> StoredRef {
        StoredRef {
            label: None,
            compact: compact(text),
            year,
            ids: Vec::new(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn aps_references_spell_out_their_dois() {
        let cases = [
            (
                "Phys. Rev. Lett. 133, 057102 (2024)",
                "10.1103/physrevlett.133.057102",
            ),
            (
                "Physical review letters 106, 250601 (2011)",
                "10.1103/physrevlett.106.250601",
            ),
            (
                "Physical Review E 105, L052103 (2022)",
                "10.1103/physreve.105.l052103",
            ),
            ("Phys. Rev. E 94(5), 052106", "10.1103/physreve.94.052106"),
            (
                "Rev. Mod. Phys. 81, 1665 (2009)",
                "10.1103/revmodphys.81.1665",
            ),
            (
                "Reviews of Modern Physics 81, 1665",
                "10.1103/revmodphys.81.1665",
            ),
            (
                "Phys. Rev. X 14, 011012 (2024)",
                "10.1103/physrevx.14.011012",
            ),
            (
                "PRX Quantum 2, 030339 (2021)",
                "10.1103/prxquantum.2.030339",
            ),
            (
                "Phys. Rev. Research 2, 033187",
                "10.1103/physrevresearch.2.033187",
            ),
            (
                "Phys. Rev. Lett. 133 (2024) 057102",
                "10.1103/physrevlett.133.057102",
            ),
            ("Phys. Rev. 47, 777 (1935)", "10.1103/physrev.47.777"),
            (
                "Phys. Rev. B 99, 041301 (2019)",
                "10.1103/physrevb.99.041301",
            ),
        ];
        for (text, doi) in cases {
            assert_eq!(journal_doi(text).as_deref(), Some(doi), "{text}");
        }
        for text in [
            "J. Stat. Mech. (2020) 083207",
            "Nature 17, 261 (2020)",
            "Europhysics Letters 121, 60005 (2018)",
            "Physical Review E 105, 10.1103/physreve.105.034130 (2022)",
            "Phys. Rev. B: Condens. Matter",
        ] {
            assert_eq!(journal_doi(text), None, "{text}");
        }
    }

    #[test]
    fn compact_ignores_case_accents_spacing_and_hyphens() {
        assert_eq!(
            compact("Higher-order  Response\ntheory"),
            "higherorderresponsetheory"
        );
        assert_eq!(
            compact("higherorder response"),
            compact("Higher-Order Response")
        );
        assert_eq!(compact("Schrödinger’s ﬁeld: 2"), "schrodingersfield2");
    }

    #[test]
    fn titles_match_long_specific_and_year_close_and_not_self() {
        let index = TitleIndex::new([
            (0, "Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport", Some(2024)),
            (1, "Introduction", Some(2024)),
            (2, "Optimal transport", Some(2024)),
            (3, "Beyond linear response", Some(2024)),
            (4, "Optimal finite-time Brownian Carnot engine: a long subtitle nobody cites", Some(2022)),
        ]);
        let zhong = reference(ZHONG, Some(2024));
        assert_eq!(index.find(&zhong, 9), Some(0));
        assert_eq!(index.find(&zhong, 0), None, "a paper never cites itself");
        // Short and generic titles never match, though their words are there.
        let generic = reference(
            "X. Y, Introduction to optimal transport (2024).",
            Some(2024),
        );
        assert_eq!(index.find(&generic, 9), None);
        // A main title before the colon matches a reference that drops the subtitle.
        let short = reference(
            "A. G. Frim, Optimal finite-time Brownian Carnot engine, PRE (2022).",
            Some(2022),
        );
        assert_eq!(index.find(&short, 9), Some(4));
        // A year more than two off is another work.
        let old = reference("A. Zhong, Beyond linear response: equivalence between thermodynamic geometry and optimal transport (2019).", Some(2019));
        assert_eq!(index.find(&old, 9), None);
        let near = reference("A. Zhong, Beyond linear response: equivalence between thermodynamic geometry and optimal transport (2022).", Some(2022));
        assert_eq!(index.find(&near, 9), Some(0));
        // Another record under the citing paper's own title is not cited.
        let dup = TitleIndex::new([
            (0, "Under review as submission to TMLR", None),
            (1, "Under review as submission to TMLR", None),
        ]);
        assert_eq!(
            dup.find(
                &reference("Under review as submission to TMLR Smith, J.", None),
                0
            ),
            None
        );
    }

    #[test]
    fn journal_references_are_found_anywhere_in_a_text() {
        let text = "As in Phys. Rev. Lett. 133, 057102 (2024) and\nPhysical Review E 105, L052103; again Phys. Rev. Lett. 133, 057102.";
        assert_eq!(
            journal_dois(text),
            [
                "10.1103/physrevlett.133.057102",
                "10.1103/physreve.105.l052103"
            ]
        );
        assert!(journal_dois("Nature 17, 261 (2020)").is_empty());
    }

    #[test]
    fn provided_entries_keep_ids_years_and_never_the_papers_own() {
        let entries = [
            provided(Some("1"), "Self, arXiv:2512.24540 (2025).", Some(2025)),
            provided(Some("2"), "B. Two, doi:10.1000/x (2020).", Some(2020)),
            provided(Some("3"), ZHONG, Some(2024)),
            provided(
                None,
                "C. Three, Physical Review E 105, 10.1103/physreve.105.034130 (2022).",
                None,
            ),
        ];
        let refs = stored_refs(&entries, None, Some("2512.24540"));
        assert_eq!(refs.len(), 4);
        assert!(
            refs[0].ids.is_empty(),
            "the paper's own id is not a citation"
        );
        assert_eq!(refs[1].ids, [CitationIdDto::Doi("10.1000/x".into())]);
        assert_eq!(refs[2].label.as_deref(), Some("3"));
        assert_eq!(refs[2].year, Some(2024));
        assert_eq!(
            refs[2].ids,
            [CitationIdDto::Doi("10.1103/physrevlett.133.057102".into())],
            "the APS rule"
        );
        assert!(refs[2].compact.contains(&compact(
            "Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport"
        )));
        assert_eq!(
            refs[3].ids,
            [CitationIdDto::Doi("10.1103/physreve.105.034130".into())],
            "a bare DOI inside an entry"
        );
        assert_eq!(refs[3].year, None, "the engine does not guess a year");
    }

    #[test]
    fn provided_entries_are_capped_and_cleaned() {
        let long = "x".repeat(MAX_ENTRY_CHARS + 500);
        let mut entries = vec![
            provided(Some("  "), "   ", Some(2020)),
            provided(Some(&"9".repeat(40)), &long, Some(99_999)),
        ];
        entries.extend((0..MAX_ENTRIES + 10).map(|n| provided(None, &format!("e{n}"), None)));
        let refs = stored_refs(&entries, None, None);
        assert_eq!(refs.len(), MAX_ENTRIES, "empty entry dropped, then capped");
        assert_eq!(refs[0].compact.len(), MAX_ENTRY_CHARS);
        assert_eq!(refs[0].label, None, "an over-long label is dropped");
        assert_eq!(refs[0].year, None, "an implausible year is dropped");
    }

    #[test]
    fn the_refs_sidecar_round_trips_for_the_same_bytes_and_provider_only() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = dir.path().join("a.pdf");
        let refs = vec![reference(ZHONG, Some(2024))];
        write_refs(dir.path(), &doc, 5, 6, PROVIDER, &refs).expect("write");
        assert_eq!(
            read_refs(dir.path(), &doc, 5, 6, PROVIDER),
            Some(refs.clone())
        );
        assert_eq!(read_refs(dir.path(), &doc, 5, 7, PROVIDER), None);
        assert_eq!(read_refs(dir.path(), &doc, 5, 6, "other@1"), None);
        assert_eq!(
            read_refs(dir.path(), &dir.path().join("b.pdf"), 5, 6, PROVIDER),
            None
        );

        let mut entry = indexed_entry(&doc, 5, 6);
        entry.cites = Some(Vec::new());
        let entries = std::slice::from_ref(&entry);
        let mut cache = RefsCache::default();
        assert_eq!(
            cache.load(dir.path(), entries, Some(PROVIDER))[&doc].len(),
            1
        );
        assert!(
            cache.load(dir.path(), entries, None).is_empty(),
            "no provider, no references"
        );
        assert!(cache.load(dir.path(), entries, Some("other@1")).is_empty());
        // Cached: a rewrite is not seen until it is forgotten.
        assert_eq!(
            cache.load(dir.path(), entries, Some(PROVIDER))[&doc].len(),
            1
        );
        write_refs(dir.path(), &doc, 5, 6, PROVIDER, &[]).expect("rewrite");
        assert_eq!(
            cache.load(dir.path(), entries, Some(PROVIDER))[&doc].len(),
            1
        );
        cache.forget(&doc);
        assert!(
            cache.load(dir.path(), entries, Some(PROVIDER)).is_empty(),
            "an empty list is read but matches nothing"
        );
        assert!(cache.unread(dir.path(), entries, PROVIDER).is_empty());
        std::fs::remove_file(refs_path(dir.path(), &doc)).expect("remove");
        cache.forget(&doc);
        assert_eq!(cache.unread(dir.path(), entries, PROVIDER).len(), 1);
    }

    /// A library of one document with a text sidecar, for the provider
    /// tests.
    fn library_with_text(dir: &Path, text: &str) -> super::super::LibraryEntry {
        let doc = dir.join("secret-name.pdf");
        crate::search::librarytext::write_sidecar(dir, &doc, 5, 6, &[(0, text.to_owned())])
            .expect("sidecar");
        let mut entry = indexed_entry(&doc, 5, 6);
        entry.cites = Some(Vec::new());
        entry.arxiv = Some("2512.24540".into());
        entry
    }

    #[test]
    fn a_document_whose_text_is_not_ready_is_waited_for_not_given_up() {
        // After an upgrade the text pass rewrites every document's text while
        // the provider is already asking. A document caught mid-rewrite must
        // be offered again once its text lands, not dropped for the session.
        let dir = tempfile::tempdir().expect("dir");
        let doc = dir.path().join("secret-name.pdf");
        let mut entry = indexed_entry(&doc, 5, 6);
        entry.text_indexed = false;
        let mut cache = RefsCache::default();
        let mut provider = RefsProvider::default();
        provider.set_active(Some(PROVIDER.into()));
        let now = Instant::now();
        assert!(provider
            .pending(dir.path(), &[entry.clone()], &mut cache, 10, now)
            .is_empty());
        // Marked indexed but the sidecar not written yet: still not a failure.
        entry.text_indexed = true;
        assert!(provider
            .pending(dir.path(), &[entry.clone()], &mut cache, 10, now)
            .is_empty());
        // The text lands; the document is offered.
        let ready = library_with_text(dir.path(), "Body. References [1] A. B, X (2001).");
        let pending = provider.pending(dir.path(), &[ready], &mut cache, 10, now);
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn pending_documents_are_handed_out_by_token_never_by_path() {
        let dir = tempfile::tempdir().expect("dir");
        let entry = library_with_text(dir.path(), "Body. References [1] A. B, X (2001).");
        let entries = vec![entry.clone()];
        let mut cache = RefsCache::default();
        let mut provider = RefsProvider::default();
        let now = Instant::now();
        assert!(
            provider
                .pending(dir.path(), &entries, &mut cache, 10, now)
                .is_empty(),
            "nothing without a provider"
        );
        provider.set_active(Some(PROVIDER.into()));
        let pending = provider.pending(dir.path(), &entries, &mut cache, 10, now);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "Body. References [1] A. B, X (2001).");
        let wire = serde_json::to_string(&pending).expect("json");
        assert!(!wire.contains("secret-name"), "{wire}");
        assert!(!wire.contains(&dir.path().to_string_lossy().into_owned()));
        assert!(!pending[0].token.contains("secret"));
        // In flight: not handed out twice.
        assert!(provider
            .pending(dir.path(), &entries, &mut cache, 10, now)
            .is_empty());

        // The answer is written as the sidecar, with the engine's fields.
        let outcome = provider
            .store(
                dir.path(),
                &pending[0].token,
                |_| Some(entry.clone()),
                Some(&[
                    provided(Some("1"), ZHONG, Some(2024)),
                    provided(Some("2"), "Self, arXiv:2512.24540.", None),
                ]),
                &mut cache,
            )
            .expect("store");
        assert_eq!(outcome, StoreOutcome::Stored(2));
        let refs = read_refs(dir.path(), &entry.path, 5, 6, PROVIDER).expect("sidecar");
        assert_eq!(refs[0].year, Some(2024));
        assert_eq!(
            refs[0].ids,
            [CitationIdDto::Doi("10.1103/physrevlett.133.057102".into())]
        );
        assert!(refs[0].compact.starts_with("azhongandmrdeweese"));
        assert!(refs[1].ids.is_empty(), "its own arXiv id");
        assert_eq!(cache.load(dir.path(), &entries, Some(PROVIDER)).len(), 1);
        assert!(provider
            .pending(dir.path(), &entries, &mut cache, 10, now)
            .is_empty());

        // A token is used once.
        assert!(provider
            .store(
                dir.path(),
                &pending[0].token,
                |_| Some(entry.clone()),
                Some(&[]),
                &mut cache
            )
            .is_err());

        // A new provider version reads it again; the old answer is not used.
        provider.set_active(Some("dev.speedyf.references@4.0.1".into()));
        assert!(cache
            .load(dir.path(), &entries, Some("dev.speedyf.references@4.0.1"))
            .is_empty());
        assert_eq!(
            provider
                .pending(dir.path(), &entries, &mut cache, 10, now)
                .len(),
            1
        );
    }

    #[test]
    fn an_answer_for_changed_bytes_or_a_changed_provider_is_not_written() {
        let dir = tempfile::tempdir().expect("dir");
        let entry = library_with_text(dir.path(), "Text.");
        let entries = vec![entry.clone()];
        let mut cache = RefsCache::default();
        let mut provider = RefsProvider::default();
        provider.set_active(Some(PROVIDER.into()));
        let now = Instant::now();
        let token = provider.pending(dir.path(), &entries, &mut cache, 1, now)[0]
            .token
            .clone();
        let mut changed = entry.clone();
        changed.size += 1;
        let answer = [provided(None, ZHONG, None)];
        assert_eq!(
            provider
                .store(
                    dir.path(),
                    &token,
                    |_| Some(changed),
                    Some(&answer),
                    &mut cache
                )
                .expect("store"),
            StoreOutcome::Stale
        );
        assert!(read_refs(dir.path(), &entry.path, 5, 6, PROVIDER).is_none());

        // Stale tokens are gone; a fresh one, then the provider changes.
        let token = provider.pending(dir.path(), &entries, &mut cache, 1, now)[0]
            .token
            .clone();
        provider.set_active(None);
        assert!(provider
            .store(
                dir.path(),
                &token,
                |_| Some(entry.clone()),
                Some(&answer),
                &mut cache
            )
            .is_err());
        provider.set_active(Some(PROVIDER.into()));

        // A failure is remembered for the session; a token expires.
        let token = provider.pending(dir.path(), &entries, &mut cache, 1, now)[0]
            .token
            .clone();
        assert_eq!(
            provider
                .store(
                    dir.path(),
                    &token,
                    |_| Some(entry.clone()),
                    None,
                    &mut cache
                )
                .expect("give up"),
            StoreOutcome::GivenUp
        );
        assert!(provider
            .pending(dir.path(), &entries, &mut cache, 1, now)
            .is_empty());
        provider.set_active(Some("x@2".into()));
        let token = provider.pending(dir.path(), &entries, &mut cache, 1, now)[0]
            .token
            .clone();
        let later = now + TOKEN_LIFETIME + Duration::from_secs(1);
        let again = provider.pending(dir.path(), &entries, &mut cache, 1, later);
        assert_eq!(again.len(), 1, "an expired token frees its document");
        assert!(provider
            .store(
                dir.path(),
                &token,
                |_| Some(entry.clone()),
                Some(&answer),
                &mut cache
            )
            .is_err());
    }

    #[test]
    fn a_long_text_keeps_its_tail() {
        let text = format!("{}é References", "a".repeat(10));
        assert_eq!(tail(text.clone(), 100), text);
        assert_eq!(tail(text.clone(), 12), " References");
        assert_eq!(tail(text, 13), "é References");
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn an_entry_keeps_its_printed_text_collapsed_and_cut() {
        let long = format!("A. Author,\n  {}", "word ".repeat(200));
        let provided = [
            ProvidedRef {
                label: None,
                text: "A. Zhong and  M. DeWeese,\nBeyond linear response (2024).".into(),
                year: Some(2024),
            },
            ProvidedRef {
                label: None,
                text: long,
                year: None,
            },
        ];
        let stored = stored_refs(&provided, None, None);
        assert_eq!(
            stored[0].text,
            "A. Zhong and M. DeWeese, Beyond linear response (2024)."
        );
        assert_eq!(stored[1].text.chars().count(), MAX_STORED_TEXT_CHARS);
        assert!(stored[1].text.ends_with("..."));
    }
}
