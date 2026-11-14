//! Metadata from the papers that cite an item, and a date from the papers
//! it cites (projects.md sec. 4, "From citing papers", and sec. 8, "Dating").
//! Offline: everything here reads the refs sidecars the reference provider
//! already wrote.
//!
//! **Consensus.** When items in the library cite another item (a reference
//! entry that resolves to it, by identifier or by title), each citing
//! item's entry is an independent witness of the cited work's authors,
//! year, title, venue, volume and first page (`witness::parse`). A field
//! is taken when at least [`MIN_WITNESSES`] independent citing items agree
//! on it and they are a strict majority of the citing items that say
//! anything about it. Citing items that are versions of one another (a
//! preprint and its journal version, spotted by `versions` or by an equal
//! title) count once, and a version of the item itself never vouches for
//! it. The value is offered at [`MetaSource::Citations`]: above the
//! first page, the file name and the Info dictionary, below the
//! identifier, XMP, every lookup and the user, through the same `offer` as
//! every other source, so a user edit is never touched. A value the
//! record already holds that agrees with the consensus is left as it is,
//! under its own source. When the consensus goes (a sidecar changed), a
//! value that came from it is dropped and the primary file is reconciled
//! again, so what the file says comes back.
//!
//! **Identifiers.** An item without a DOI (or arXiv id) takes the one its
//! citing papers print, when at least [`MIN_WITNESSES`] independent citing
//! items name it by title (`TitleIndex::find`) and print that identifier in
//! the same entry, no citing item names it with another one, and each
//! entry's first author and year (when the entry and the item both have
//! them) fit the item: the same family name (`persons::NameKey`), years at
//! most [`ID_YEAR_SLACK`] apart. It lands with source `Citations`, below the
//! file's own identifier, XMP, every lookup and the user: a scan of the
//! file that finds one replaces it, a lookup that answers with one replaces
//! it, and it goes when the consensus does. An identifier another item
//! already holds is never taken; the two are suggested as duplicates
//! instead ("cited as the same DOI by 3 papers", `CitedAs`).
//!
//! **Dating.** A paper is at least as new as what it cites:
//! [`year_floor`] is the latest year its references give, made robust.

use super::papers::{fold, normalize_arxiv, offer, CitationTables, PaperRecord};
use super::persons;
use super::references::{DocRefs, StoredRef};
use super::store::{format_rfc3339, PaperStore};
use super::types::{Author, MetaField, MetaSource};
use super::witness::{self, Witness};
use super::{citekey, titlematch, versions, LibraryEntry};
use crate::engine::citation::fold_arxiv_doi;
use crate::engine::types::CitationIdDto;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Independent citing items that must agree before a field is taken.
pub const MIN_WITNESSES: usize = 2;
/// A recorded year this much below the floor is still plausible: a
/// journal version can cite work that appeared while it was in review.
pub const YEAR_LAG: i32 = 1;
/// The newest cited year is ignored when only one reference gives it and
/// it is more than this far above the next: a parse error, most likely.
pub const OUTLIER_GAP: i32 = 3;
/// A citing entry's year and the item's may differ this much before the
/// entry is taken for another work's (identifiers only).
pub const ID_YEAR_SLACK: i32 = 1;

/// What the citation graph says about one record, kept on it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CitationFacts {
    /// For each field the citing papers supplied ("authors", "year", ...),
    /// the ids of the citing items that vouched for it, one per
    /// independent group.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vouched: BTreeMap<String, Vec<String>>,
    /// The year its own reference list says it is at least as new as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_floor: Option<i32>,
    /// An identifier citing papers print for this item that another item
    /// holds: offered as a duplicate suggestion, never taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_as: Option<CitedAs>,
}

/// "cited as the same DOI by 3 papers": the other item and the evidence.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CitedAs {
    /// The item holding the identifier.
    pub paper_id: String,
    /// "DOI" or "arXiv id".
    pub kind: String,
    /// Independent citing items that print it.
    pub count: u32,
}

impl CitedAs {
    pub fn reason(&self) -> String {
        format!("cited as the same {} by {} papers", self.kind, self.count)
    }
}

impl CitationFacts {
    pub fn is_empty(&self) -> bool {
        self.vouched.is_empty() && self.year_floor.is_none() && self.cited_as.is_none()
    }
}

impl PaperRecord {
    /// How many citing items vouched for each field whose value is theirs.
    pub fn vouched_counts(&self) -> BTreeMap<String, u32> {
        self.citation_facts
            .vouched
            .iter()
            .filter(|(field, _)| field_source(self, field) == Some(MetaSource::Citations))
            .map(|(field, ids)| (field.clone(), ids.len().min(u32::MAX as usize) as u32))
            .collect()
    }

    /// The floor, for an item with no year to show instead.
    pub fn shown_year_floor(&self) -> Option<i32> {
        self.year()
            .is_none()
            .then_some(self.citation_facts.year_floor)
            .flatten()
    }

    /// The year the list sorts by: the year, else the floor.
    pub fn sort_year(&self) -> Option<i32> {
        self.year().or(self.citation_facts.year_floor)
    }

    /// "cites work from 2021; year may be wrong", when the recorded year is
    /// more than [`YEAR_LAG`] below the floor.
    pub fn year_hint(&self) -> Option<String> {
        let (year, floor) = (self.year()?, self.citation_facts.year_floor?);
        (year < floor - YEAR_LAG).then(|| format!("cites work from {floor}; year may be wrong"))
    }
}

fn field_source(paper: &PaperRecord, field: &str) -> Option<MetaSource> {
    let meta = &paper.meta;
    match field {
        "title" => meta.title.as_ref().map(|f| f.source),
        "authors" => meta.authors.as_ref().map(|f| f.source),
        "year" => meta.year.as_ref().map(|f| f.source),
        "venue" => meta.venue.as_ref().map(|f| f.source),
        "volume" => meta.volume.as_ref().map(|f| f.source),
        "pages" => meta.pages.as_ref().map(|f| f.source),
        "doi" => paper.ids.doi_source,
        "arxiv" => paper.ids.arxiv_source,
        _ => None,
    }
}

/// This year, for telling a cited year from a typo in the future.
pub fn current_year() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
        .get(..4)
        .and_then(|year| year.parse().ok())
        .unwrap_or(2026)
}

/* ---- Dating ---- */

/// The latest year a reference list cites, made robust: years after
/// `this_year` are ignored; the newest year is ignored when one reference
/// alone gives it and it is more than [`OUTLIER_GAP`] years above the next
/// newest; and the floor must be supported by the references at or after
/// it - one that prints an identifier, or two that do not. With less, the
/// next older year is tried.
pub fn year_floor(refs: &[StoredRef], this_year: i32) -> Option<i32> {
    let mut dated: Vec<(i32, bool)> = refs
        .iter()
        .filter_map(|r| Some((r.year?, !r.ids.is_empty())))
        .filter(|(year, _)| (1400..=this_year).contains(year))
        .collect();
    dated.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    if dated.len() >= 2 {
        let top = dated[0].0;
        let alone = dated[1].0 != top;
        if alone && top - dated[1].0 > OUTLIER_GAP {
            dated.remove(0);
        }
    }
    let (mut with_id, mut without) = (0, 0);
    for (year, has_id) in dated {
        if has_id {
            with_id += 1;
        } else {
            without += 1;
        }
        if with_id >= 1 || without >= 2 {
            return Some(year);
        }
    }
    None
}

/* ---- Consensus ---- */

/// Fields the citing papers agreed on, each with the citing items (store
/// indices, one per independent group) that vouched for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agreed {
    pub title: Option<(String, Vec<usize>)>,
    pub authors: Option<(Vec<Author>, Vec<usize>)>,
    pub year: Option<(i32, Vec<usize>)>,
    pub venue: Option<(String, Vec<usize>)>,
    pub volume: Option<(String, Vec<usize>)>,
    pub pages: Option<(String, Vec<usize>)>,
}

/// One vote per independent citing item: (the item, what its entry says).
pub type Vote = (usize, Witness);

/// The indices of the votes that agree, when they are at least
/// [`MIN_WITNESSES`] and a strict majority of `votes`.
fn majority<T>(votes: &[(usize, T)], same: impl Fn(&T, &T) -> bool) -> Option<Vec<usize>> {
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (at, (_, value)) in votes.iter().enumerate() {
        match clusters
            .iter_mut()
            .find(|cluster| same(&votes[cluster[0]].1, value))
        {
            Some(cluster) => cluster.push(at),
            None => clusters.push(vec![at]),
        }
    }
    let best = clusters.into_iter().max_by_key(Vec::len)?;
    (best.len() >= MIN_WITNESSES && best.len() * 2 > votes.len()).then_some(best)
}

/// The form most votes print; a tie goes to the longer, then the first.
fn most_common(forms: &[&str]) -> String {
    let mut best: Option<(&str, usize)> = None;
    for form in forms {
        let count = forms.iter().filter(|other| *other == form).count();
        let better = best.is_none_or(|(current, n)| {
            count > n || (count == n && form.chars().count() > current.chars().count())
        });
        if better {
            best = Some((form, count));
        }
    }
    best.map(|(form, _)| form.to_owned()).unwrap_or_default()
}

fn agree_text(
    votes: &[(usize, Option<&str>)],
    key: impl Fn(&str) -> String,
    same: impl Fn(&str, &str) -> bool,
) -> Option<(String, Vec<usize>)> {
    let said: Vec<(usize, (&str, String))> = votes
        .iter()
        .filter_map(|(voter, value)| {
            let value = (*value)?;
            let k = key(value);
            (!k.is_empty()).then_some((*voter, (value, k)))
        })
        .collect();
    let agreeing = majority(&said, |a, b| same(&a.1, &b.1))?;
    let forms: Vec<&str> = agreeing.iter().map(|&i| said[i].1 .0).collect();
    Some((
        most_common(&forms),
        agreeing.iter().map(|&i| said[i].0).collect(),
    ))
}

/// What independent witnesses agree on (see the module doc).
pub fn agree(votes: &[Vote]) -> Agreed {
    let pick = |f: fn(&Witness) -> Option<&str>| -> Vec<(usize, Option<&str>)> {
        votes.iter().map(|(voter, w)| (*voter, f(w))).collect()
    };
    let years: Vec<(usize, i32)> = votes
        .iter()
        .filter_map(|(voter, w)| Some((*voter, w.year?)))
        .collect();
    let year = majority(&years, |a, b| a == b).map(|agreeing| {
        (
            years[agreeing[0]].1,
            agreeing.iter().map(|&i| years[i].0).collect(),
        )
    });
    Agreed {
        title: agree_text(
            &pick(|w| w.title.as_deref()),
            |t| {
                let n = titlematch::normalize(t);
                if n.split(' ').count() < 3 {
                    String::new()
                } else {
                    n.replace(' ', "")
                }
            },
            |a, b| a == b,
        ),
        authors: agree_authors(votes),
        year,
        venue: agree_text(&pick(|w| w.venue.as_deref()), venue_key, venues_match),
        volume: agree_text(
            &pick(|w| w.volume.as_deref()),
            |v| v.trim_start_matches('0').to_owned(),
            |a, b| a == b,
        ),
        pages: agree_text(&pick(|w| w.pages.as_deref()), first_page, |a, b| a == b),
    }
}

/// A family name as compared: folded, letters only.
pub(super) fn family_key(family: &str) -> String {
    fold(family).chars().filter(|c| c.is_alphabetic()).collect()
}

fn initial(given: &str) -> Option<char> {
    fold(given).chars().find(|c| c.is_alphabetic())
}

/// One person as far as citing papers go: the same family and first
/// initial, or two forms the person rule (`persons::same_name`) calls one
/// person - "van der Waals" and "Waals", "J. A." and "John".
fn same_person(a: &Author, b: &Author) -> bool {
    (family_key(&a.family) == family_key(&b.family)
        && match (initial(&a.given), initial(&b.given)) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        })
        || persons::same_name(a, b)
}

/// Two author lists name the same people in the same order.
pub fn same_authors(a: &[Author], b: &[Author]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_person(x, y))
}

/// `short` (an "et al." list) opens `long`.
fn opens(short: &[Author], long: &[Author]) -> bool {
    !short.is_empty()
        && short.len() <= long.len()
        && short.iter().zip(long).all(|(x, y)| same_person(x, y))
}

/// Complete lists vote for themselves; an "et al." list supports every
/// complete list it opens and counts among the witnesses. The list taken
/// has each family as most witnesses print it (not in capitals when some
/// do not) and each given name in its fullest printed form.
fn agree_authors(votes: &[Vote]) -> Option<(Vec<Author>, Vec<usize>)> {
    let said: Vec<&Vote> = votes
        .iter()
        .filter(|(_, w)| !w.authors.is_empty())
        .collect();
    let complete: Vec<(usize, &Vote)> = said
        .iter()
        .enumerate()
        .filter(|(_, (_, w))| !w.et_al)
        .map(|(at, vote)| (at, *vote))
        .collect();
    let mut best: Option<Vec<usize>> = None;
    for (_, (_, candidate)) in &complete {
        let support: Vec<usize> = said
            .iter()
            .enumerate()
            .filter(|(_, (_, w))| {
                if w.et_al {
                    opens(&w.authors, &candidate.authors)
                } else {
                    same_authors(&w.authors, &candidate.authors)
                }
            })
            .map(|(at, _)| at)
            .collect();
        if best.as_ref().is_none_or(|b| support.len() > b.len()) {
            best = Some(support);
        }
    }
    let support = best?;
    if support.len() < MIN_WITNESSES || support.len() * 2 <= said.len() {
        return None;
    }
    let full: Vec<&Witness> = support
        .iter()
        .map(|&at| &said[at].1)
        .filter(|w| !w.et_al)
        .collect();
    let length = full.first()?.authors.len();
    let mut authors = Vec::with_capacity(length);
    for position in 0..length {
        let printed: Vec<&Author> = support
            .iter()
            .filter_map(|&at| said[at].1.authors.get(position))
            .collect();
        let families: Vec<&str> = printed.iter().map(|a| a.family.as_str()).collect();
        let cased: Vec<&str> = families
            .iter()
            .copied()
            .filter(|f| f.chars().any(char::is_lowercase))
            .collect();
        let family = most_common(if cased.is_empty() { &families } else { &cased });
        let given = printed
            .iter()
            .map(|a| a.given.as_str())
            .max_by_key(|g| g.chars().filter(|c| c.is_alphabetic()).count())
            .unwrap_or_default()
            .to_owned();
        authors.push(Author { family, given });
    }
    Some((authors, support.iter().map(|&at| said[at].0).collect()))
}

/// A venue's words as compared: folded, joining words dropped.
fn venue_words(venue: &str) -> Vec<String> {
    fold(venue)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| {
            !matches!(
                *w,
                "of" | "the" | "and" | "in" | "on" | "for" | "de" | "der" | "und"
            )
        })
        .map(str::to_owned)
        .collect()
}

fn venue_key(venue: &str) -> String {
    venue_words(venue).join(" ")
}

/// `short` abbreviates `long`: the same first letter and its letters in
/// order ("Phys" -> "Physical", "Natl" -> "National", "J" -> "Journal").
fn abbreviates(short: &str, long: &str) -> bool {
    let mut letters = long.chars();
    short.chars().next() == long.chars().next() && short.chars().all(|c| letters.any(|l| l == c))
}

/// "Phys. Rev. Lett." is "Physical Review Letters"; "J. Chem. Phys." is
/// "Journal of Chemical Physics". Word for word, each abbreviating the
/// other; a one-letter series at the end must be the same letter, so
/// "Phys. Rev. A" is not "Phys. Rev. Applied".
fn venues_match(a: &str, b: &str) -> bool {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    a.len() == b.len()
        && a.iter().zip(&b).enumerate().all(|(at, (x, y))| {
            let last = at + 1 == a.len();
            if last && (x.len() == 1 || y.len() == 1) {
                return x == y;
            }
            abbreviates(x, y) || abbreviates(y, x)
        })
}

/// "436-444" -> "436"; "L052103" -> "l052103".
fn first_page(pages: &str) -> String {
    pages
        .split('-')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/* ---- The pass over the library ---- */

/// Each record's reference list, for records whose files have one: the
/// primary file's when it has one, else the first found. Indexed like
/// `PaperStore::papers`.
pub(super) fn lists_by_record<'r>(
    store: &PaperStore,
    tables: &CitationTables<'_>,
    entries: &[LibraryEntry],
    refs: &'r DocRefs,
) -> Vec<Option<&'r Arc<Vec<StoredRef>>>> {
    let mut lists: Vec<Option<&Arc<Vec<StoredRef>>>> = vec![None; store.papers.len()];
    for entry in entries {
        let Some(list) = refs.get(&entry.path) else {
            continue;
        };
        let path = entry.path.to_string_lossy();
        let Some(&holder) = tables.by_path.get(path.as_ref()) else {
            continue;
        };
        let primary = store.papers[holder]
            .files
            .first()
            .is_some_and(|file| file.path == path);
        if lists[holder].is_none() || primary {
            lists[holder] = Some(list);
        }
    }
    lists
}

/// Union-find over record indices.
pub(super) struct Groups(Vec<usize>);

impl Groups {
    pub(super) fn new(n: usize) -> Self {
        Self((0..n).collect())
    }

    pub(super) fn find(&mut self, mut at: usize) -> usize {
        while self.0[at] != at {
            self.0[at] = self.0[self.0[at]];
            at = self.0[at];
        }
        at
    }

    pub(super) fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

/// What the pass decided for one record.
struct Plan {
    agreed: Agreed,
    year_floor: Option<i32>,
    doi: Option<CitedId>,
    arxiv: Option<CitedId>,
}

/// An identifier the citing papers agree on for one record: its value, the
/// citing items that print it (one per independent group), and the record
/// that already holds it, when one does.
#[derive(Clone, Debug, PartialEq)]
struct CitedId {
    value: String,
    voters: Vec<usize>,
    owner: Option<usize>,
}

/// What one independent citing group's entries say of a record's
/// identifiers.
#[derive(Default)]
struct IdVote {
    voter: usize,
    dois: Vec<String>,
    arxivs: Vec<String>,
}

/// Whether a citing entry's first author and year fit the record, where
/// both say something: the same family name, years at most
/// [`ID_YEAR_SLACK`] apart.
fn entry_fits(read: &Witness, paper: &PaperRecord) -> bool {
    if let (Some(cited), Some(own)) = (read.authors.first(), paper.authors().first()) {
        if let (Some(x), Some(y)) = (persons::NameKey::of(cited), persons::NameKey::of(own)) {
            if x.family != y.family {
                return false;
            }
        }
    }
    match (read.year, paper.year()) {
        (Some(x), Some(y)) => (x - y).abs() <= ID_YEAR_SLACK,
        _ => true,
    }
}

/// The one value every vote that prints any agrees on, with at least
/// [`MIN_WITNESSES`] votes printing it; none when two differ.
fn agreed_id(
    votes: &[IdVote],
    pick: impl Fn(&IdVote) -> &[String],
) -> Option<(String, Vec<usize>)> {
    let mut value: Option<&String> = None;
    let mut voters = Vec::new();
    for vote in votes {
        for printed in pick(vote) {
            match value {
                Some(v) if v != printed => return None,
                _ => value = Some(printed),
            }
        }
        if !pick(vote).is_empty() {
            voters.push(vote.voter);
        }
    }
    let value = value?.clone();
    (voters.len() >= MIN_WITNESSES).then_some((value, voters))
}

impl PaperStore {
    /// Records that are versions of one another as far as vouching goes:
    /// a version pair (`versions::spot`), or two records under one
    /// normalized title of four words or more.
    fn version_groups(
        &self,
        tables: &CitationTables<'_>,
        lists: &[Option<&Arc<Vec<StoredRef>>>],
    ) -> Groups {
        let mut groups = Groups::new(self.papers.len());
        for pair in versions::spot(self, tables, lists) {
            groups.join(pair.a, pair.b);
        }
        let mut by_title: HashMap<String, usize> = HashMap::new();
        for (index, paper) in self.papers.iter().enumerate() {
            let Some(title) = paper.title().map(titlematch::normalize) else {
                continue;
            };
            if title.split(' ').count() < 4 {
                continue;
            }
            match by_title.get(&title.replace(' ', "")) {
                Some(&first) => groups.join(first, index),
                None => {
                    by_title.insert(title.replace(' ', ""), index);
                }
            }
        }
        groups
    }

    /// Pairs of records (ids, smaller first) for author identity
    /// (`persons`): one cites the other, their lists share at least
    /// [`persons::MIN_SHARED_REFS`] references (`versions::reference_keys`),
    /// and their author lists hold a candidate pair of names.
    fn topic_links(
        &self,
        tables: &CitationTables<'_>,
        lists: &[Option<&Arc<Vec<StoredRef>>>],
    ) -> Vec<(String, String)> {
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (holder, list) in lists.iter().enumerate() {
            let Some(list) = list else {
                continue;
            };
            for reference in list.iter() {
                let Some(target) = tables
                    .resolve_reference(reference, holder)
                    .filter(|&t| t != holder)
                else {
                    continue;
                };
                let pair = (holder.min(target), holder.max(target));
                if pairs.contains(&pair) {
                    continue;
                }
                let (a, b) = (self.papers[pair.0].authors(), self.papers[pair.1].authors());
                if a.iter().any(|x| b.iter().any(|y| persons::same_name(x, y))) {
                    pairs.push(pair);
                }
            }
        }
        let mut keys: HashMap<usize, Vec<Vec<String>>> = HashMap::new();
        let mut keyed = |at: usize| -> Vec<Vec<String>> {
            keys.entry(at)
                .or_insert_with(|| {
                    lists[at]
                        .map(|list| {
                            list.iter()
                                .map(|r| versions::reference_keys(r, at, tables))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .clone()
        };
        let mut out = Vec::new();
        for (a, b) in pairs {
            let (ka, kb) = (keyed(a), keyed(b));
            let theirs: std::collections::HashSet<&String> = kb.iter().flatten().collect();
            let shared = ka
                .iter()
                .filter(|keys| keys.iter().any(|k| theirs.contains(k)))
                .count();
            if shared >= persons::MIN_SHARED_REFS {
                let (x, y) = (self.papers[a].id.clone(), self.papers[b].id.clone());
                out.push(if x < y { (x, y) } else { (y, x) });
            }
        }
        out
    }

    /// Every record's votes, consensus and floor, read-only; and the topic
    /// links for author identity.
    fn citation_plans(
        &self,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        this_year: i32,
    ) -> (Vec<Plan>, Vec<(String, String)>) {
        let tables = self.citation_tables();
        let lists = lists_by_record(self, &tables, entries, refs);
        let links = self.topic_links(&tables, &lists);
        let mut groups = self.version_groups(&tables, &lists);
        let mut votes: Vec<Vec<(usize, Vote)>> = vec![Vec::new(); self.papers.len()];
        let mut id_votes: Vec<Vec<(usize, IdVote)>> =
            (0..self.papers.len()).map(|_| Vec::new()).collect();
        for (holder, list) in lists.iter().enumerate() {
            let Some(list) = list else {
                continue;
            };
            let group = groups.find(holder);
            for reference in list.iter() {
                if reference.text.is_empty() {
                    continue;
                }
                // Identifiers: by title only, so an entry printing an id
                // another record holds still names the record it cites.
                if let Some(target) = tables
                    .titles
                    .find(reference, holder)
                    .filter(|&t| t != holder && groups.find(t) != group)
                {
                    let read = witness::parse(&reference.text, reference.year);
                    if entry_fits(&read, &self.papers[target]) {
                        let at = match id_votes[target].iter().position(|(g, _)| *g == group) {
                            Some(at) => at,
                            None => {
                                id_votes[target].push((
                                    group,
                                    IdVote {
                                        voter: holder,
                                        ..IdVote::default()
                                    },
                                ));
                                id_votes[target].len() - 1
                            }
                        };
                        let vote = &mut id_votes[target][at].1;
                        for id in &reference.ids {
                            match fold_arxiv_doi(id.clone()) {
                                CitationIdDto::Doi(doi) => push_unique(&mut vote.dois, doi),
                                CitationIdDto::ArXiv(arxiv) => {
                                    if let Some(arxiv) = normalize_arxiv(&arxiv) {
                                        push_unique(&mut vote.arxivs, arxiv);
                                    }
                                }
                            }
                        }
                    }
                }
                let Some(target) = tables
                    .resolve_reference(reference, holder)
                    .filter(|&t| t != holder)
                else {
                    continue;
                };
                // A version of the item never vouches for it, and a group
                // of versions vouches once.
                if groups.find(target) == group || votes[target].iter().any(|(g, _)| *g == group) {
                    continue;
                }
                votes[target].push((
                    group,
                    (holder, witness::parse(&reference.text, reference.year)),
                ));
            }
        }
        let owner = |id: CitationIdDto| -> Option<usize> {
            let folded = fold_arxiv_doi(id.clone());
            let arxiv_doi = match &folded {
                CitationIdDto::ArXiv(arxiv) => {
                    Some(CitationIdDto::Doi(format!("10.48550/arxiv.{arxiv}")))
                }
                CitationIdDto::Doi(_) => None,
            };
            [Some(id), Some(folded), arxiv_doi]
                .into_iter()
                .flatten()
                .find_map(|id| tables.by_id.get(&id).copied())
        };
        let plans = votes
            .into_iter()
            .zip(id_votes)
            .enumerate()
            .map(|(index, (votes, id_votes))| {
                let votes: Vec<Vote> = votes.into_iter().map(|(_, vote)| vote).collect();
                let id_votes: Vec<IdVote> = id_votes.into_iter().map(|(_, vote)| vote).collect();
                let cited = |pick: fn(&IdVote) -> &[String], make: fn(String) -> CitationIdDto| {
                    agreed_id(&id_votes, pick).map(|(value, voters)| CitedId {
                        owner: owner(make(value.clone())),
                        value,
                        voters,
                    })
                };
                Plan {
                    agreed: if votes.len() >= MIN_WITNESSES {
                        agree(&votes)
                    } else {
                        Agreed::default()
                    },
                    year_floor: lists[index].and_then(|list| year_floor(list, this_year)),
                    doi: cited(|v| &v.dois, CitationIdDto::Doi),
                    arxiv: cited(|v| &v.arxivs, CitationIdDto::ArXiv),
                }
            })
            .collect();
        (plans, links)
    }

    /// Bring every record's citation-derived metadata and year floor up to
    /// date with the reference lists (see the module doc). Returns whether
    /// anything changed. Nothing in a read-only store.
    pub fn apply_citation_facts(
        &mut self,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        this_year: i32,
    ) -> bool {
        if !self.writable() {
            return false;
        }
        let (plans, links) = self.citation_plans(entries, refs, this_year);
        let mut changed_any = self.persons.set_topic_links(links);
        let ids: Vec<String> = self.papers.iter().map(|p| p.id.clone()).collect();
        let named = |voters: Vec<usize>| -> Vec<String> {
            voters.into_iter().map(|v| ids[v].clone()).collect()
        };
        let mut reconcile: Vec<String> = Vec::new();
        for (index, plan) in plans.into_iter().enumerate() {
            let base_before = citekey::base_key(&self.papers[index].meta);
            let paper = &mut self.papers[index];
            let mut vouched: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut changed = false;
            let mut retracted = false;
            let agreed = plan.agreed;
            let mut settle = |name: &str, outcome: Settled| match outcome {
                Settled::Vouched(voters, moved) => {
                    vouched.insert(name.to_owned(), voters);
                    changed |= moved;
                }
                Settled::Retracted => {
                    changed = true;
                    retracted = true;
                }
                Settled::Untouched => {}
            };
            let authors = agreed.authors.map(|(mut list, voters)| {
                enrich_given_names(&mut list, paper.authors());
                (list, named(voters))
            });
            settle(
                "authors",
                settle_field(&mut paper.meta.authors, authors, |a, b| same_authors(a, b)),
            );
            settle(
                "title",
                settle_field(
                    &mut paper.meta.title,
                    agreed.title.map(|(v, w)| (v, named(w))),
                    |a, b| {
                        titlematch::normalize(a).replace(' ', "")
                            == titlematch::normalize(b).replace(' ', "")
                    },
                ),
            );
            settle(
                "year",
                settle_field(
                    &mut paper.meta.year,
                    agreed.year.map(|(v, w)| (v, named(w))),
                    |a, b| a == b,
                ),
            );
            settle(
                "venue",
                settle_field(
                    &mut paper.meta.venue,
                    agreed.venue.map(|(v, w)| (v, named(w))),
                    |a, b| venues_match(&venue_key(a), &venue_key(b)),
                ),
            );
            settle(
                "volume",
                settle_field(
                    &mut paper.meta.volume,
                    agreed.volume.map(|(v, w)| (v, named(w))),
                    |a, b| a.trim_start_matches('0') == b.trim_start_matches('0'),
                ),
            );
            settle(
                "pages",
                settle_field(
                    &mut paper.meta.pages,
                    agreed.pages.map(|(v, w)| (v, named(w))),
                    |a, b| first_page(a) == first_page(b),
                ),
            );
            // Identifiers, each settled on its own.
            let mut cited_as = None;
            for (kind, cited) in [("DOI", plan.doi), ("arXiv id", plan.arxiv)] {
                let doi = kind == "DOI";
                let alias = |value: &str| {
                    let id = if doi {
                        CitationIdDto::Doi(value.to_owned())
                    } else {
                        CitationIdDto::ArXiv(value.to_owned())
                    };
                    paper.alias_ids.contains(&id)
                };
                let (held, source) = if doi {
                    (&paper.ids.doi, paper.ids.doi_source)
                } else {
                    (&paper.ids.arxiv, paper.ids.arxiv_source)
                };
                let ours = source == Some(MetaSource::Citations);
                if held.is_some() && !ours {
                    continue;
                }
                let held = held.clone();
                let take = match cited {
                    Some(cited) if cited.owner.is_some_and(|o| o != index) => {
                        if cited_as.is_none() {
                            cited_as = Some(CitedAs {
                                paper_id: ids[cited.owner.unwrap_or(index)].clone(),
                                kind: kind.to_owned(),
                                count: cited.voters.len().min(u32::MAX as usize) as u32,
                            });
                        }
                        None
                    }
                    Some(cited) if !alias(&cited.value) => Some(cited),
                    _ => None,
                };
                let (slot, slot_source) = if doi {
                    (&mut paper.ids.doi, &mut paper.ids.doi_source)
                } else {
                    (&mut paper.ids.arxiv, &mut paper.ids.arxiv_source)
                };
                match take {
                    Some(cited) => {
                        if held.as_deref() != Some(cited.value.as_str()) {
                            *slot = Some(cited.value);
                            *slot_source = Some(MetaSource::Citations);
                            changed = true;
                        }
                        let field = if doi { "doi" } else { "arxiv" };
                        vouched.insert(field.to_owned(), named(cited.voters));
                    }
                    None if ours => {
                        *slot = None;
                        *slot_source = None;
                        changed = true;
                    }
                    None => {}
                }
            }
            let facts = CitationFacts {
                vouched,
                year_floor: plan.year_floor,
                cited_as,
            };
            if paper.citation_facts != facts {
                paper.citation_facts = facts;
                changed_any = true;
            }
            if retracted {
                reconcile.push(paper.id.clone());
            }
            if changed {
                changed_any = true;
                paper.updated_at = super::store::now_rfc3339();
                let base_after = citekey::base_key(&paper.meta);
                if !paper.cite_key_pinned && base_before != base_after {
                    let meta = paper.meta.clone();
                    let key = self.free_key(&meta, Some(index));
                    self.papers[index].set_cite_key(key);
                }
            }
        }
        // What the file says comes back where the consensus went.
        for id in reconcile {
            let Some(primary) = self
                .papers
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.files.first())
                .map(|f| f.path.clone())
            else {
                continue;
            };
            if let Some(entry) = entries.iter().find(|e| e.path.to_string_lossy() == primary) {
                self.reconcile_file(entry);
            }
        }
        if changed_any {
            self.mark_dirty();
        }
        changed_any
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

enum Settled {
    /// The consensus stands behind the field's value: these voters, and
    /// whether the value changed.
    Vouched(Vec<String>, bool),
    /// A value that came from citing papers is gone with their consensus.
    Retracted,
    Untouched,
}

/// One field against the consensus: a value that agrees is left under its
/// own source; one that does not is offered the consensus at
/// `Citations` rank (so it replaces only weaker sources); a field that had
/// the consensus's value loses it when the consensus goes.
fn settle_field<T: PartialEq + Clone>(
    field: &mut Option<MetaField<T>>,
    agreed: Option<(T, Vec<String>)>,
    same: impl Fn(&T, &T) -> bool,
) -> Settled {
    let from_citations = field
        .as_ref()
        .is_some_and(|f| f.source == MetaSource::Citations);
    match agreed {
        Some((value, voters)) => {
            if field.as_ref().is_some_and(|f| same(&f.value, &value)) {
                return if from_citations {
                    Settled::Vouched(voters, false)
                } else {
                    Settled::Untouched
                };
            }
            if offer(field, value, MetaSource::Citations) {
                Settled::Vouched(voters, true)
            } else {
                Settled::Untouched
            }
        }
        None if from_citations => {
            *field = None;
            Settled::Retracted
        }
        None => Settled::Untouched,
    }
}

/// A given name the record already has in fuller form than the citing
/// papers print ("Adrianne" for "A.") is kept, person by person.
fn enrich_given_names(list: &mut [Author], current: &[Author]) {
    for (author, known) in list.iter_mut().zip(current) {
        let letters = |g: &str| g.chars().filter(|c| c.is_alphabetic()).count();
        if same_person(author, known) && letters(&known.given) > letters(&author.given) {
            author.given = known.given.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::CitationIdDto;

    fn reference(year: Option<i32>, id: bool) -> StoredRef {
        StoredRef {
            label: None,
            compact: String::new(),
            year,
            ids: if id {
                vec![CitationIdDto::Doi("10.1/x".into())]
            } else {
                Vec::new()
            },
            text: String::new(),
        }
    }

    #[test]
    fn the_floor_is_the_latest_supported_cited_year() {
        let refs = [
            reference(Some(2015), false),
            reference(Some(2019), true),
            reference(Some(2018), false),
        ];
        assert_eq!(year_floor(&refs, 2026), Some(2019), "one with an id");
        let refs = [
            reference(Some(2015), false),
            reference(Some(2019), false),
            reference(Some(2018), false),
        ];
        assert_eq!(
            year_floor(&refs, 2026),
            Some(2018),
            "two without an id: 2019 and 2018 are at or after 2018"
        );
        let refs = [reference(Some(2019), false), reference(Some(2019), false)];
        assert_eq!(year_floor(&refs, 2026), Some(2019));
    }

    #[test]
    fn the_floor_ignores_the_future_an_outlier_and_thin_support() {
        let refs = [reference(Some(2031), true), reference(Some(2019), true)];
        assert_eq!(year_floor(&refs, 2026), Some(2019), "the future is a typo");
        let refs = [
            reference(Some(2024), true),
            reference(Some(2017), true),
            reference(Some(2016), false),
        ];
        assert_eq!(year_floor(&refs, 2026), Some(2017), "a lone outlier");
        let refs = [
            reference(Some(2024), true),
            reference(Some(2024), false),
            reference(Some(2017), true),
        ];
        assert_eq!(
            year_floor(&refs, 2026),
            Some(2024),
            "two say so: not an outlier"
        );
        let refs = [reference(Some(2020), true), reference(Some(2017), true)];
        assert_eq!(
            year_floor(&refs, 2026),
            Some(2020),
            "three years is not an outlier"
        );
        assert_eq!(year_floor(&[reference(Some(2020), false)], 2026), None);
        assert_eq!(year_floor(&[reference(None, true)], 2026), None);
        assert_eq!(year_floor(&[], 2026), None);
    }

    fn record(year: Option<i32>, floor: Option<i32>) -> PaperRecord {
        let mut paper = PaperRecord::new();
        paper.meta.year = year.map(|value| MetaField {
            value,
            source: MetaSource::FirstPage,
        });
        paper.citation_facts.year_floor = floor;
        paper
    }

    #[test]
    fn the_floor_shows_without_a_year_and_hints_against_an_old_one() {
        let undated = record(None, Some(2019));
        assert_eq!(undated.shown_year_floor(), Some(2019));
        assert_eq!(undated.sort_year(), Some(2019));
        assert_eq!(undated.year_hint(), None);
        let dated = record(Some(2015), Some(2019));
        assert_eq!(dated.shown_year_floor(), None);
        assert_eq!(dated.sort_year(), Some(2015));
        assert_eq!(
            dated.year_hint().as_deref(),
            Some("cites work from 2019; year may be wrong")
        );
        assert_eq!(
            record(Some(2018), Some(2019)).year_hint(),
            None,
            "a year of lag"
        );
        assert_eq!(
            record(Some(2017), Some(2019)).year_hint().as_deref(),
            Some("cites work from 2019; year may be wrong")
        );
        assert_eq!(record(Some(2020), Some(2019)).year_hint(), None);
        assert_eq!(record(Some(2010), None).year_hint(), None);
    }

    fn vote(voter: usize, text: &str, year: Option<i32>) -> Vote {
        (voter, witness::parse(text, year))
    }

    const APS: &str = "A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).";
    const SPRINGER: &str = "Zhong, A., DeWeese, M.R.: Beyond linear response: equivalence between thermodynamic geometry and optimal transport. Physical Review Letters 133, 057102 (2024)";
    const WRONG: &str = "A. Zhang and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2023).";

    #[test]
    fn two_agreeing_witnesses_in_a_majority_carry_a_field() {
        let agreed = agree(&[vote(1, APS, Some(2024)), vote(2, SPRINGER, Some(2024))]);
        let (authors, voters) = agreed.authors.expect("authors");
        assert_eq!(voters, [1, 2]);
        assert_eq!(authors[0].family, "Zhong");
        assert_eq!(authors[1].family, "DeWeese");
        assert_eq!(authors[1].given, "M. R.");
        assert_eq!(agreed.year.map(|(y, _)| y), Some(2024));
        let venue = agreed.venue.expect("venue").0;
        assert!(
            venue == "Physical Review Letters" || venue == "Phys. Rev. Lett.",
            "{venue}"
        );
        assert_eq!(agreed.volume.map(|(v, _)| v).as_deref(), Some("133"));
        assert_eq!(agreed.pages.map(|(v, _)| v).as_deref(), Some("057102"));
        assert!(agreed.title.is_some());

        // One witness is not enough.
        assert_eq!(agree(&[vote(1, APS, Some(2024))]), Agreed::default());

        // Two against one is a majority; one against one is not.
        let agreed = agree(&[
            vote(1, APS, Some(2024)),
            vote(2, SPRINGER, Some(2024)),
            vote(3, WRONG, Some(2023)),
        ]);
        assert_eq!(agreed.authors.map(|(_, v)| v), Some(vec![1, 2]));
        assert_eq!(agreed.year, Some((2024, vec![1, 2])));
        let agreed = agree(&[vote(1, APS, Some(2024)), vote(3, WRONG, Some(2023))]);
        assert!(agreed.authors.is_none());
        assert!(agreed.year.is_none());
        // Two of four is not a strict majority.
        let agreed = agree(&[
            vote(1, APS, Some(2024)),
            vote(2, SPRINGER, Some(2024)),
            vote(3, WRONG, Some(2023)),
            vote(4, WRONG, Some(2023)),
        ]);
        assert!(agreed.authors.is_none());
        assert!(agreed.year.is_none());
    }

    #[test]
    fn an_et_al_list_supports_the_full_list_it_opens() {
        let agreed = agree(&[
            vote(1, APS, Some(2024)),
            vote(
                2,
                "A. Zhong et al., Phys. Rev. Lett. 133, 057102 (2024).",
                Some(2024),
            ),
        ]);
        let (authors, voters) = agreed.authors.expect("authors");
        assert_eq!(authors.len(), 2);
        assert_eq!(voters, [1, 2]);
        // Two truncated lists name no full list.
        let agreed = agree(&[
            vote(
                1,
                "A. Zhong et al., Phys. Rev. Lett. 133, 057102 (2024).",
                None,
            ),
            vote(
                2,
                "A. Zhong et al., Phys. Rev. Lett. 133, 057102 (2024).",
                None,
            ),
        ]);
        assert!(agreed.authors.is_none());
    }

    #[test]
    fn venues_match_across_abbreviation_but_not_across_series() {
        let same = |a: &str, b: &str| venues_match(&venue_key(a), &venue_key(b));
        assert!(same("Phys. Rev. Lett.", "Physical Review Letters"));
        assert!(same("J. Chem. Phys.", "Journal of Chemical Physics"));
        assert!(same(
            "Proc. Natl. Acad. Sci. USA",
            "Proceedings of the National Academy of Sciences USA"
        ));
        assert!(!same("Phys. Rev. A", "Phys. Rev. Applied"));
        assert!(!same("Phys. Rev. A", "Phys. Rev. B"));
        assert!(!same("Nature", "Nature Physics"));
        assert!(!same("Phys. Rev. Lett.", "Phys. Lett."));
    }
}

/// Shared by the store tests here and in `versions`.
#[cfg(test)]
pub(super) mod fixtures {
    use super::super::references::{stored_refs, DocRefs, ProvidedRef};
    use super::super::store::PaperStore;
    use super::super::types::Author;
    use super::super::LibraryEntry;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    /// A scanned file with a confident first-page title and authors.
    pub fn entry(path: &str, title: &str, authors: &[(&str, &str)]) -> LibraryEntry {
        let mut entry = crate::library::tests::indexed_entry(std::path::Path::new(path), 1, 100);
        entry.fingerprint = Some(format!("fp-{path}"));
        entry.cites = Some(Vec::new());
        entry.title = Some(title.to_owned());
        entry.first_page_title_confident = true;
        entry.first_page_authors_confident = true;
        entry.first_page_authors = authors
            .iter()
            .map(|(given, family)| Author {
                given: (*given).to_owned(),
                family: (*family).to_owned(),
            })
            .collect();
        entry
    }

    /// A reference list as the provider returns it: (text, year) each.
    pub type Listed = Vec<(String, Option<i32>)>;

    /// Reference lists by path, as the provider would have stored them.
    pub fn refs(lists: &[(&str, Listed)]) -> DocRefs {
        lists
            .iter()
            .map(|(path, entries)| {
                let provided: Vec<ProvidedRef> = entries
                    .iter()
                    .map(|(text, year)| ProvidedRef {
                        label: None,
                        text: text.clone(),
                        year: *year,
                    })
                    .collect();
                (
                    PathBuf::from(path),
                    Arc::new(stored_refs(&provided, None, None)),
                )
            })
            .collect()
    }

    pub fn id_of(store: &PaperStore, path: &str) -> String {
        store
            .papers
            .iter()
            .find(|p| p.files.iter().any(|f| f.path == path))
            .expect("record")
            .id
            .clone()
    }
}

#[cfg(test)]
mod store_tests {
    use super::fixtures::{entry, id_of, refs, store};
    use super::*;
    use crate::library::types::MetaPatchDto;

    const TARGET: &str =
        "Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport";
    const APS: &str = "A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).";
    const SPRINGER: &str = "Zhong, A., DeWeese, M.R.: Beyond linear response: equivalence between thermodynamic geometry and optimal transport. Phys. Rev. Lett. 133, 057102 (2024)";
    const WRONG: &str = "A. Zhang and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2023).";

    fn library() -> (PaperStore, Vec<LibraryEntry>) {
        let mut store = store();
        let entries = vec![
            entry("/l/target.pdf", TARGET, &[("", "Zhong")]),
            entry(
                "/l/c1.pdf",
                "Higher-order response theory in optimal stochastic thermodynamics",
                &[("Jordan", "Sawchuk")],
            ),
            entry(
                "/l/c2.pdf",
                "Optimal control of overdamped systems at finite time",
                &[("Some", "One")],
            ),
            entry(
                "/l/c3.pdf",
                "Thermodynamic geometry of minimum dissipation driven barrier crossing",
                &[("Other", "Person")],
            ),
        ];
        for e in &entries {
            store.reconcile_file(e);
        }
        (store, entries)
    }

    fn witnesses(aps: bool, springer: bool, wrong: bool) -> DocRefs {
        let mut lists = Vec::new();
        if aps {
            lists.push(("/l/c1.pdf", vec![(APS.to_owned(), Some(2024))]));
        }
        if springer {
            lists.push(("/l/c2.pdf", vec![(SPRINGER.to_owned(), Some(2024))]));
        }
        if wrong {
            lists.push(("/l/c3.pdf", vec![(WRONG.to_owned(), Some(2023))]));
        }
        refs(&lists)
    }

    fn target(store: &PaperStore) -> &PaperRecord {
        let id = id_of(store, "/l/target.pdf");
        store.papers.iter().find(|p| p.id == id).expect("target")
    }

    #[test]
    fn citing_papers_fill_authors_and_say_who_vouched() {
        let (mut store, entries) = library();
        let refs = witnesses(true, true, true);
        assert!(store.apply_citation_facts(&entries, &refs, 2026));
        let paper = target(&store);
        let authors = paper.meta.authors.as_ref().expect("authors");
        assert_eq!(authors.source, MetaSource::Citations);
        let names: Vec<(&str, &str)> = authors
            .value
            .iter()
            .map(|a| (a.given.as_str(), a.family.as_str()))
            .collect();
        assert_eq!(names, [("A.", "Zhong"), ("M. R.", "DeWeese")]);
        assert_eq!(
            paper.meta.year.as_ref().map(|f| (f.value, f.source)),
            Some((2024, MetaSource::Citations)),
            "two of three"
        );
        assert_eq!(
            paper.meta.volume.as_ref().map(|f| f.value.as_str()),
            Some("133")
        );
        assert_eq!(
            paper.meta.venue.as_ref().map(|f| f.value.as_str()),
            Some("Phys. Rev. Lett.")
        );
        let (c1, c2) = (id_of(&store, "/l/c1.pdf"), id_of(&store, "/l/c2.pdf"));
        assert_eq!(paper.citation_facts.vouched["authors"], [c1, c2]);
        // The first-page title agrees, so it stays the first page's.
        assert_eq!(
            paper.meta.title.as_ref().map(|f| f.source),
            Some(MetaSource::FirstPage)
        );
        assert!(!paper.citation_facts.vouched.contains_key("title"));
        let id = paper.id.clone();
        let dto = store.paper_dto(&id).expect("dto");
        assert_eq!(dto.vouched.get("authors"), Some(&2));
        assert_eq!(dto.vouched.get("year"), Some(&2));
        assert!(dto.cite_key.starts_with("zhong2024"), "{}", dto.cite_key);
        // Settled: a second pass changes nothing.
        assert!(!store.apply_citation_facts(&entries, &refs, 2026));
    }

    #[test]
    fn a_user_edit_is_never_overwritten() {
        let (mut store, entries) = library();
        let id = id_of(&store, "/l/target.pdf");
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(Some(2025)),
                    authors: Some(Some(vec![Author {
                        family: "Mine".into(),
                        given: String::new(),
                    }])),
                    ..Default::default()
                },
            )
            .expect("patch");
        store.apply_citation_facts(&entries, &witnesses(true, true, true), 2026);
        let paper = target(&store);
        assert_eq!(
            paper.meta.year.as_ref().map(|f| (f.value, f.source)),
            Some((2025, MetaSource::User))
        );
        assert_eq!(paper.authors()[0].family, "Mine");
        let dto = store.paper_dto(&id).expect("dto");
        assert!(!dto.vouched.contains_key("authors"));
        assert!(!dto.vouched.contains_key("year"));
    }

    #[test]
    fn citations_rank_below_the_identifier_and_above_the_first_page() {
        let mut store = store();
        let mut target = entry("/l/target.pdf", TARGET, &[("", "Zhong")]);
        // The arXiv id says 2024; the citing papers will say 2023.
        target.arxiv = Some("2404.01286".into());
        let c1 = entry("/l/c1.pdf", "A first citing paper about other things", &[]);
        let c2 = entry("/l/c2.pdf", "A second citing paper about more things", &[]);
        let entries = vec![target, c1, c2];
        for e in &entries {
            store.reconcile_file(e);
        }
        let refs = refs(&[
            ("/l/c1.pdf", vec![(WRONG.to_owned(), Some(2023))]),
            (
                "/l/c2.pdf",
                vec![(WRONG.replace("A. Zhang", "A. Zhang, B. Two"), Some(2023))],
            ),
        ]);
        store.apply_citation_facts(&entries, &refs, 2026);
        let paper = store
            .papers
            .iter()
            .find(|p| p.ids.arxiv.as_deref() == Some("2404.01286"))
            .expect("target");
        assert_eq!(
            paper.meta.year.as_ref().map(|f| (f.value, f.source)),
            Some((2024, MetaSource::Identifier))
        );
        // The two lists of authors differ: no consensus, the page's stay.
        assert_eq!(
            paper.meta.authors.as_ref().map(|f| f.source),
            Some(MetaSource::FirstPage)
        );
        // The venue they agree on fills the gap.
        assert_eq!(
            paper.meta.venue.as_ref().map(|f| f.source),
            Some(MetaSource::Citations)
        );
        assert!(MetaSource::Citations.rank() > MetaSource::PdfInfo.rank());
        assert!(MetaSource::Citations.rank() > MetaSource::FirstPage.rank());
        assert!(MetaSource::Citations.rank() < MetaSource::Identifier.rank());
        assert!(MetaSource::Citations.rank() < MetaSource::Xmp.rank());
        assert!(!MetaSource::Citations.is_lookup());
    }

    #[test]
    fn versions_of_one_citing_paper_count_once_and_never_vouch_for_themselves() {
        let mut store = store();
        let same = "Higher-order response theory in optimal stochastic thermodynamics";
        let entries = vec![
            entry("/l/target.pdf", TARGET, &[("", "Zhong")]),
            entry("/l/preprint.pdf", same, &[]),
            entry("/l/journal.pdf", same, &[]),
        ];
        for e in &entries {
            store.reconcile_file(e);
        }
        // Two records under one title (not merged): one voice.
        let lists = refs(&[
            ("/l/preprint.pdf", vec![(APS.to_owned(), Some(2024))]),
            ("/l/journal.pdf", vec![(SPRINGER.to_owned(), Some(2024))]),
        ]);
        store.apply_citation_facts(&entries, &lists, 2026);
        assert_eq!(
            target(&store).meta.authors.as_ref().map(|f| f.source),
            Some(MetaSource::FirstPage)
        );

        // A version of the target citing it does not vouch either.
        let mut store = super::fixtures::store();
        let entries = vec![
            entry("/l/target.pdf", TARGET, &[("", "Zhong")]),
            entry("/l/target-v2.pdf", TARGET, &[]),
            entry("/l/c1.pdf", "A first citing paper about other things", &[]),
        ];
        for e in &entries {
            store.reconcile_file(e);
        }
        let lists = refs(&[
            ("/l/target-v2.pdf", vec![(APS.to_owned(), Some(2024))]),
            ("/l/c1.pdf", vec![(SPRINGER.to_owned(), Some(2024))]),
        ]);
        store.apply_citation_facts(&entries, &lists, 2026);
        assert_eq!(
            target(&store).meta.authors.as_ref().map(|f| f.source),
            Some(MetaSource::FirstPage)
        );
    }

    #[test]
    fn when_the_consensus_goes_the_files_own_value_comes_back() {
        let (mut store, entries) = library();
        store.apply_citation_facts(&entries, &witnesses(true, true, true), 2026);
        assert_eq!(
            target(&store).meta.authors.as_ref().map(|f| f.source),
            Some(MetaSource::Citations)
        );
        // The second witness's list changed: one against one.
        assert!(store.apply_citation_facts(&entries, &witnesses(true, false, true), 2026));
        let paper = target(&store);
        assert_eq!(
            paper
                .meta
                .authors
                .as_ref()
                .map(|f| (f.value[0].family.as_str(), f.source)),
            Some(("Zhong", MetaSource::FirstPage))
        );
        assert_eq!(paper.meta.year, None);
        assert!(!paper.citation_facts.vouched.contains_key("authors"));
        assert!(!paper.citation_facts.vouched.contains_key("year"));
        // Where the two still agree (the journal reference), they stand.
        assert!(paper.citation_facts.vouched.contains_key("volume"));
    }

    #[test]
    fn a_citing_papers_own_list_dates_it() {
        let (mut store, entries) = library();
        let lists = refs(&[(
            "/l/c1.pdf",
            vec![
                ("A. B, X, Phys. Rev. E 1, 1 (2019).".to_owned(), Some(2019)),
                ("C. D, Y, doi:10.1000/y (2021).".to_owned(), Some(2021)),
                ("E. F, Z (2016).".to_owned(), Some(2016)),
            ],
        )]);
        store.apply_citation_facts(&entries, &lists, 2026);
        let id = id_of(&store, "/l/c1.pdf");
        let dto = store.paper_dto(&id).expect("dto");
        assert_eq!(dto.year_floor, Some(2021));
        assert_eq!(dto.year_hint, None);
        let row = store
            .paper_rows(&Default::default())
            .rows
            .into_iter()
            .find(|r| r.id == id)
            .expect("row");
        assert_eq!((row.year, row.year_floor), (None, Some(2021)));
        // A recorded year far below the floor is questioned, not changed.
        store
            .apply_patch(
                &id,
                MetaPatchDto {
                    year: Some(Some(2012)),
                    ..Default::default()
                },
            )
            .expect("patch");
        let dto = store.paper_dto(&id).expect("dto");
        assert_eq!(dto.year_floor, None, "a year is shown, not the floor");
        assert_eq!(
            dto.year_hint.as_deref(),
            Some("cites work from 2021; year may be wrong")
        );
        assert_eq!(dto.meta.year.map(|f| f.value), Some(2012));
        // The floor never reaches BibTeX.
        let (bib, _) = store.bibtex_for(&[id]).expect("bibtex");
        assert!(bib.contains("2012") && !bib.contains("2021"), "{bib}");
    }

    #[test]
    fn an_undated_item_sorts_by_its_floor() {
        let (mut store, entries) = library();
        let lists = refs(&[(
            "/l/c1.pdf",
            vec![("C. D, Y, doi:10.1000/y (2001).".to_owned(), Some(2001))],
        )]);
        store.apply_citation_facts(&entries, &lists, 2026);
        let c1 = store
            .papers
            .iter()
            .find(|p| p.files[0].path == "/l/c1.pdf")
            .expect("c1");
        assert_eq!(c1.sort_year(), Some(2001));
        let id = c1.id.clone();
        let (bib, _) = store.bibtex_for(std::slice::from_ref(&id)).expect("bibtex");
        assert!(!bib.contains("2001"), "{bib}");
    }
}

/// Identifiers from citing papers, and the topic links author identity
/// reads.
#[cfg(test)]
mod id_tests {
    use super::fixtures::{entry, id_of, refs, store};
    use super::*;
    use std::collections::HashMap;

    const TARGET: &str =
        "Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport";
    const DOI: &str = "10.1103/physrevlett.133.057102";
    const CITING: [&str; 3] = ["/l/c1.pdf", "/l/c2.pdf", "/l/c3.pdf"];

    /// The entry as one citing paper prints it, with `doi` when given.
    fn cite(first: &str, year: i32, doi: Option<&str>) -> (String, Option<i32>) {
        let doi = doi.map(|d| format!(" doi:{d}")).unwrap_or_default();
        (
            format!(
                "{first} and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Optics Letters ({year}).{doi}"
            ),
            Some(year),
        )
    }

    fn library(extra: Vec<LibraryEntry>) -> (PaperStore, Vec<LibraryEntry>) {
        let mut store = store();
        let mut entries = vec![
            entry("/l/target.pdf", TARGET, &[("", "Zhong")]),
            entry(
                CITING[0],
                "Higher-order response theory in optimal stochastic thermodynamics",
                &[("Jordan", "Sawchuk")],
            ),
            entry(
                CITING[1],
                "Optimal control of overdamped systems at finite time",
                &[("Some", "One")],
            ),
            entry(
                CITING[2],
                "Thermodynamic geometry of minimum dissipation driven barrier crossing",
                &[("Other", "Person")],
            ),
        ];
        entries.extend(extra);
        for e in &entries {
            store.reconcile_file(e);
        }
        (store, entries)
    }

    /// Each citing paper's one entry, by position in `CITING`.
    fn lists(cited: &[(String, Option<i32>)]) -> DocRefs {
        let lists: Vec<(&str, super::fixtures::Listed)> = cited
            .iter()
            .zip(CITING)
            .map(|(entry, path)| (path, vec![entry.clone()]))
            .collect();
        refs(&lists)
    }

    fn target(store: &PaperStore) -> &PaperRecord {
        let id = id_of(store, "/l/target.pdf");
        store.papers.iter().find(|p| p.id == id).expect("target")
    }

    #[test]
    fn two_citing_papers_printing_one_doi_give_it_to_the_item() {
        let (mut store, entries) = library(Vec::new());
        // One is not enough.
        store.apply_citation_facts(&entries, &lists(&[cite("A. Zhong", 2024, Some(DOI))]), 2026);
        assert_eq!(target(&store).ids.doi, None);

        let two = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, None),
        ]);
        assert!(store.apply_citation_facts(&entries, &two, 2026));
        let paper = target(&store);
        assert_eq!(paper.ids.doi.as_deref(), Some(DOI));
        assert_eq!(paper.ids.doi_source, Some(MetaSource::Citations));
        let dto = store.paper_dto(&paper.id).expect("dto");
        assert_eq!(dto.doi_source, Some(MetaSource::Citations));
        assert_eq!(dto.vouched.get("doi"), Some(&2));
        // The identity layer reads it like any other: a citation naming
        // the DOI resolves to the item.
        let tables = store.citation_tables();
        let index = store.index_of(&paper.id).expect("index");
        assert_eq!(
            tables.by_id.get(&CitationIdDto::Doi(DOI.into())),
            Some(&index)
        );
        // Settled.
        assert!(!store.apply_citation_facts(&entries, &two, 2026));
    }

    #[test]
    fn a_conflicting_id_or_a_mismatched_entry_blocks_it() {
        let (mut store, entries) = library(Vec::new());
        let conflict = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some("10.1103/physrevlett.133.999999")),
        ]);
        store.apply_citation_facts(&entries, &conflict, 2026);
        assert_eq!(target(&store).ids.doi, None, "one citing paper disagrees");

        // Another first author: the entry is about another work, whatever
        // its title says, and its DOI is no witness.
        let (mut store, entries) = library(Vec::new());
        let mismatch = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("B. Brown", 2024, Some(DOI)),
        ]);
        store.apply_citation_facts(&entries, &mismatch, 2026);
        assert_eq!(target(&store).ids.doi, None, "first author differs");

        // A year more than one off, against a year the item has.
        let (mut store, entries) = library(Vec::new());
        let id = id_of(&store, "/l/target.pdf");
        store
            .apply_patch(
                &id,
                crate::library::types::MetaPatchDto {
                    year: Some(Some(2024)),
                    ..Default::default()
                },
            )
            .expect("patch");
        let late = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2019, Some(DOI)),
        ]);
        store.apply_citation_facts(&entries, &late, 2026);
        assert_eq!(target(&store).ids.doi, None, "years five apart");
        let near = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2023, Some(DOI)),
        ]);
        store.apply_citation_facts(&entries, &near, 2026);
        assert_eq!(target(&store).ids.doi.as_deref(), Some(DOI), "a year apart");
    }

    #[test]
    fn an_id_another_item_holds_becomes_a_merge_suggestion() {
        let mut holder = entry(
            "/l/holder.pdf",
            "A record that already holds the identifier somehow",
            &[("Q.", "Holder")],
        );
        holder.doi = Some(DOI.to_owned());
        let (mut store, entries) = library(vec![holder]);
        let three = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some(DOI)),
        ]);
        store.apply_citation_facts(&entries, &three, 2026);
        let paper = target(&store);
        assert_eq!(paper.ids.doi, None, "never two items with one DOI");
        let holder_id = id_of(&store, "/l/holder.pdf");
        let cited = paper.citation_facts.cited_as.clone().expect("cited as");
        assert_eq!(cited.paper_id, holder_id);
        let groups = store.duplicates(&HashMap::new());
        let group = groups
            .iter()
            .find(|g| g.paper_ids.contains(&holder_id))
            .expect("suggested");
        assert!(group.paper_ids.contains(&paper.id));
        assert_eq!(group.reason, "cited as the same DOI by 3 papers");
        // Only suggested: both records stay.
        assert_ne!(
            id_of(&store, "/l/holder.pdf"),
            id_of(&store, "/l/target.pdf")
        );
    }

    #[test]
    fn the_files_own_id_wins_and_the_adopted_one_goes_with_its_consensus() {
        let (mut store, mut entries) = library(Vec::new());
        let two = lists(&[
            cite("A. Zhong", 2024, Some(DOI)),
            cite("A. Zhong", 2024, Some(DOI)),
        ]);
        store.apply_citation_facts(&entries, &two, 2026);
        assert_eq!(target(&store).ids.doi.as_deref(), Some(DOI));
        // A rescan that finds no DOI in the file keeps it: it was never
        // the file's to retract.
        store.reconcile_file(&entries[0]);
        assert_eq!(target(&store).ids.doi.as_deref(), Some(DOI));
        // A lost witness drops it.
        store.apply_citation_facts(&entries, &lists(&[cite("A. Zhong", 2024, Some(DOI))]), 2026);
        let paper = target(&store);
        assert_eq!(
            (paper.ids.doi.as_deref(), paper.ids.doi_source),
            (None, None)
        );
        assert!(!paper.citation_facts.vouched.contains_key("doi"));

        // Back, and then the file is found to print another DOI.
        store.apply_citation_facts(&entries, &two, 2026);
        assert_eq!(target(&store).ids.doi.as_deref(), Some(DOI));
        entries[0].doi = Some("10.1000/own".to_owned());
        store.reconcile_file(&entries[0]);
        store.apply_citation_facts(&entries, &two, 2026);
        let paper = target(&store);
        assert_eq!(paper.ids.doi.as_deref(), Some("10.1000/own"));
        assert_eq!(paper.ids.doi_source, Some(MetaSource::FirstPage));
        assert!(!paper.citation_facts.vouched.contains_key("doi"));

        // The user's beats it too, and a frozen key stays frozen.
        let (mut store, entries) = library(Vec::new());
        let id = id_of(&store, "/l/target.pdf");
        store.freeze_keys(std::slice::from_ref(&id));
        let key = target(&store).cite_key.clone();
        store.apply_citation_facts(&entries, &two, 2026);
        assert_eq!(target(&store).cite_key, key);
        store
            .apply_patch(
                &id,
                crate::library::types::MetaPatchDto {
                    doi: Some(Some("10.1000/mine".into())),
                    ..Default::default()
                },
            )
            .expect("patch");
        store.apply_citation_facts(&entries, &two, 2026);
        assert_eq!(target(&store).ids.doi.as_deref(), Some("10.1000/mine"));
        assert_eq!(target(&store).ids.doi_source, Some(MetaSource::User));
    }

    #[test]
    fn an_arxiv_id_is_taken_the_same_way() {
        let (mut store, entries) = library(Vec::new());
        let two = lists(&[
            cite("A. Zhong", 2024, Some("10.48550/arXiv.2404.01286")),
            (
                cite("A. Zhong", 2024, None)
                    .0
                    .replace("(2024).", "arXiv:2404.01286v2 (2024)."),
                Some(2024),
            ),
        ]);
        store.apply_citation_facts(&entries, &two, 2026);
        let paper = target(&store);
        assert_eq!(paper.ids.arxiv.as_deref(), Some("2404.01286"));
        assert_eq!(paper.ids.arxiv_source, Some(MetaSource::Citations));
        assert_eq!(paper.ids.doi, None);
    }

    #[test]
    fn a_citation_on_one_topic_makes_two_initials_one_person() {
        let mut store = store();
        let shared = |n: usize| {
            (
                format!(
                    "K. Other, A rather specific study of entropy production number {n}, Phys. Rev. E {n}, {n}0 (2019)."
                ),
                Some(2019),
            )
        };
        let entries = vec![
            entry(
                "/l/early.pdf",
                "Stochastic thermodynamics of tiny heat engines",
                &[("J.", "Smith")],
            ),
            entry(
                "/l/late.pdf",
                "Optimal driving protocols for tiny heat engines revisited",
                &[("J.", "Smith")],
            ),
        ];
        for e in &entries {
            store.reconcile_file(e);
        }
        let mut late: Vec<(String, Option<i32>)> = (1..=3).map(shared).collect();
        late.push((
            "J. Smith, Stochastic thermodynamics of tiny heat engines, Phys. Rev. E 9, 9 (2018)."
                .to_owned(),
            Some(2018),
        ));
        let lists = refs(&[
            ("/l/early.pdf", (1..=3).map(shared).collect()),
            ("/l/late.pdf", late),
        ]);
        store.refresh_persons();
        let early = id_of(&store, "/l/early.pdf");
        assert!(
            store.persons().person_at(&early, 0).is_none(),
            "no evidence yet"
        );
        assert!(store.apply_citation_facts(&entries, &lists, 2026));
        store.refresh_persons();
        let person = store.persons().person_at(&early, 0).expect("one person");
        assert_eq!(person.papers.len(), 2);
        let items = store.person_items(&person.id.clone()).expect("items");
        assert_eq!(items.name, "J. Smith");
        assert_eq!(items.rows.len(), 2);
        let dto = store.paper_dto(&early).expect("dto");
        assert_eq!(dto.author_persons, [Some(person.id.clone())]);
    }
}
