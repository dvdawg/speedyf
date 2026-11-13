//! Author identity (projects.md sec. 2, "Authors as people"): the name
//! variants the library prints for one person ("J. Smith", "John Smith",
//! "Smith, J. A.") clustered into persons. Offline and derived: everything
//! here is recomputed from the records' author lists and the citation
//! links the citation pass leaves behind (`PersonsSection::topic_links`).
//!
//! **Candidates.** Two printed names are candidates when their family names
//! are the same once case, diacritics, letters other than A-Z and leading
//! particles are set aside ("van der Waals" is "Waals"; "Garcia-Lopez" is
//! "Garcia Lopez") and their given names are compatible position by
//! position: an initial matches any name it begins, two spelled-out names
//! must be equal ("J. A." fits "John Andrew" and "John", not "J. B." or
//! "James").
//!
//! **Evidence.** A candidate pair on two different items is merged only
//! with one of, strongest first: the same spelled-out first given name;
//! a shared coauthor (compatible names, given names on both); or a
//! citation link between the two items with at least [`MIN_SHARED_REFS`]
//! references in common (the same topic). Clusters grow complete-linkage:
//! two clusters join only when every pair of names across them is
//! compatible and no item would hold the person twice, so "J. Smith" never
//! bridges "John Smith" and "James Smith". Pairs are taken in a fixed order
//! (evidence, then occurrence order by item id), so the clusters are the
//! same on every run and every machine.
//!
//! **Ids.** A person id lives in `library.json`'s `persons` section (its
//! own version, [`PERSONS_VERSION`]; another version is rebuilt). A
//! recomputed cluster takes the id most of its members had, so ids survive
//! edits and new items; a new one is named after its first member.

use super::papers::{fold, PaperRecord};
use super::types::{Author, PersonItemsDto};
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The `persons` section's format. Another version is discarded and
/// rebuilt: nothing in it is the user's.
pub const PERSONS_VERSION: u32 = 1;
/// References two linked items must share to count as one topic.
pub const MIN_SHARED_REFS: usize = 3;

const PARTICLES: &[&str] = &[
    "van", "von", "der", "den", "de", "del", "della", "di", "da", "du", "dos", "des", "la", "le",
    "ten", "ter", "zu", "bin", "al", "el", "y",
];

/// A printed name as compared.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NameKey {
    /// Folded letters of the family name, particles dropped.
    pub family: String,
    /// Folded given names, one per word; an initial is one letter.
    pub given: Vec<String>,
}

impl NameKey {
    pub fn of(author: &Author) -> Option<Self> {
        let words: Vec<String> = author
            .family
            .split(|c: char| c.is_whitespace() || c == '-')
            .map(letters)
            .filter(|w| !w.is_empty())
            .collect();
        let start = words
            .iter()
            .position(|w| !PARTICLES.contains(&w.as_str()))
            .unwrap_or(words.len().saturating_sub(1));
        let family: String = words[start.min(words.len())..].concat();
        if family.is_empty() {
            return None;
        }
        let mut given: Vec<String> = Vec::new();
        for word in author
            .given
            .split(|c: char| c.is_whitespace() || c == '.' || c == '-')
            .filter(|w| !w.is_empty())
        {
            let upper = word.chars().all(|c| c.is_uppercase());
            let folded = letters(word);
            if folded.is_empty() {
                continue;
            }
            // "JA", Vancouver's run of initials.
            if upper && (2..=3).contains(&folded.chars().count()) {
                given.extend(folded.chars().map(String::from));
            } else {
                given.push(folded);
            }
        }
        // Particles the parser left on the given side ("Johannes van der").
        while given
            .last()
            .is_some_and(|w| w.len() > 1 && PARTICLES.contains(&w.as_str()))
        {
            given.pop();
        }
        Some(Self { family, given })
    }

    /// The first given name spelled out, if it is.
    fn first_full(&self) -> Option<&str> {
        self.given
            .first()
            .filter(|g| g.chars().count() > 1)
            .map(String::as_str)
    }

    /// "smith:j.a", for keeping ids across recomputations.
    fn text(&self) -> String {
        format!("{}:{}", self.family, self.given.join("."))
    }
}

fn letters(text: &str) -> String {
    fold(text).chars().filter(|c| c.is_alphabetic()).collect()
}

/// The same family and given names that fit position by position (see the
/// module doc). An empty given name fits any.
pub fn compatible(a: &NameKey, b: &NameKey) -> bool {
    a.family == b.family
        && a.given.iter().zip(&b.given).all(|(x, y)| {
            if x.chars().count() == 1 || y.chars().count() == 1 {
                x.chars().next() == y.chars().next()
            } else {
                x == y
            }
        })
}

/// Whether two printed names can be one person (`compatible`).
pub fn same_name(a: &Author, b: &Author) -> bool {
    match (NameKey::of(a), NameKey::of(b)) {
        (Some(a), Some(b)) => compatible(&a, &b),
        _ => false,
    }
}

/// One person as stored: an id and its members, each `"{item id}#{name}"`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersonRecord {
    pub id: String,
    pub members: Vec<String>,
}

/// `library.json`'s `persons` section.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonsSection {
    #[serde(default)]
    pub version: u32,
    /// Item id pairs (smaller first) linked by a citation and sharing at
    /// least [`MIN_SHARED_REFS`] references, whose author lists hold a
    /// candidate pair. Written by the citation pass, which has the lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_links: Vec<(String, String)>,
    /// Persons with more than one item, in a fixed order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<PersonRecord>,
    /// What the clusters were computed from, to skip unchanged input.
    #[serde(skip)]
    input: Option<u64>,
    #[serde(skip)]
    index: Box<PersonIndex>,
}

impl PersonsSection {
    /// As loaded: another version is rebuilt from nothing.
    pub fn loaded(self) -> Self {
        if self.version == PERSONS_VERSION {
            self
        } else {
            Self {
                version: PERSONS_VERSION,
                ..Self::default()
            }
        }
    }

    pub fn index(&self) -> &PersonIndex {
        &self.index
    }

    /// Recompute the clusters when the author lists or the links changed.
    /// Returns whether the stored persons changed.
    pub fn refresh(&mut self, papers: &[PaperRecord]) -> bool {
        self.version = PERSONS_VERSION;
        let input = input_hash(papers, &self.topic_links);
        if self.input == Some(input) {
            return false;
        }
        self.input = Some(input);
        let links: HashSet<(&str, &str)> = self
            .topic_links
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let clusters = cluster(papers, &links);
        let (people, index) = name_persons(papers, &clusters, &self.people);
        *self.index = index;
        if people == self.people {
            return false;
        }
        self.people = people;
        true
    }

    /// Replace the topic links. Returns whether they changed.
    pub fn set_topic_links(&mut self, mut links: Vec<(String, String)>) -> bool {
        links.sort();
        links.dedup();
        if links == self.topic_links {
            return false;
        }
        self.topic_links = links;
        true
    }
}

/// One occurrence of a name: an author slot of an item.
#[derive(Clone, Debug)]
struct Occurrence {
    paper: usize,
    position: usize,
    key: NameKey,
}

fn input_hash(papers: &[PaperRecord], links: &[(String, String)]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |text: &str| {
        for byte in text.bytes().chain([0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for paper in papers {
        eat(&paper.id);
        for author in paper.authors() {
            eat(&author.family);
            eat(&author.given);
        }
    }
    for (a, b) in links {
        eat(a);
        eat(b);
    }
    hash
}

/// The occurrences, in a fixed order (item id, then position).
fn occurrences(papers: &[PaperRecord]) -> Vec<Occurrence> {
    let mut order: Vec<usize> = (0..papers.len()).collect();
    order.sort_by(|&a, &b| papers[a].id.cmp(&papers[b].id));
    let mut out = Vec::new();
    for paper in order {
        for (position, author) in papers[paper].authors().iter().enumerate() {
            if let Some(key) = NameKey::of(author) {
                out.push(Occurrence {
                    paper,
                    position,
                    key,
                });
            }
        }
    }
    out
}

/// Whether the two items share a coauthor besides the two occurrences.
fn shared_coauthor(papers: &[PaperRecord], a: &Occurrence, b: &Occurrence) -> bool {
    let others = |occurrence: &Occurrence| -> Vec<NameKey> {
        papers[occurrence.paper]
            .authors()
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != occurrence.position)
            .filter_map(|(_, author)| NameKey::of(author))
            .filter(|key| !key.given.is_empty())
            .collect()
    };
    let theirs = others(b);
    others(a)
        .iter()
        .any(|x| theirs.iter().any(|y| compatible(x, y)))
}

/// Clusters of occurrence indices (into `occurrences(papers)`), each
/// sorted, in order of their first member.
fn cluster(papers: &[PaperRecord], links: &HashSet<(&str, &str)>) -> Vec<Vec<Occurrence>> {
    let occurrences = occurrences(papers);
    let mut buckets: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (at, occurrence) in occurrences.iter().enumerate() {
        buckets
            .entry(occurrence.key.family.as_str())
            .or_default()
            .push(at);
    }
    // (strength, a, b): lower strength is stronger evidence.
    let mut edges: Vec<(u8, usize, usize)> = Vec::new();
    for members in buckets.values() {
        for (i, &a) in members.iter().enumerate() {
            for &b in &members[i + 1..] {
                let (x, y) = (&occurrences[a], &occurrences[b]);
                if x.paper == y.paper || !compatible(&x.key, &y.key) {
                    continue;
                }
                let strength =
                    if x.key.first_full().is_some() && x.key.first_full() == y.key.first_full() {
                        0
                    } else if shared_coauthor(papers, x, y) {
                        1
                    } else {
                        let (p, q) = (papers[x.paper].id.as_str(), papers[y.paper].id.as_str());
                        let pair = if p < q { (p, q) } else { (q, p) };
                        if links.contains(&pair) {
                            2
                        } else {
                            continue;
                        }
                    };
                edges.push((strength, a, b));
            }
        }
    }
    edges.sort_unstable();

    // Complete-linkage union: each cluster's members, by root.
    let mut root: Vec<usize> = (0..occurrences.len()).collect();
    let mut members: Vec<Vec<usize>> = (0..occurrences.len()).map(|at| vec![at]).collect();
    for (_, a, b) in edges {
        let (ra, rb) = (root[a], root[b]);
        if ra == rb {
            continue;
        }
        let fits = members[ra].iter().all(|&x| {
            members[rb].iter().all(|&y| {
                occurrences[x].paper != occurrences[y].paper
                    && compatible(&occurrences[x].key, &occurrences[y].key)
            })
        });
        if !fits {
            continue;
        }
        let (keep, gone) = (ra.min(rb), ra.max(rb));
        let moved = std::mem::take(&mut members[gone]);
        for &m in &moved {
            root[m] = keep;
        }
        members[keep].extend(moved);
        members[keep].sort_unstable();
    }
    members
        .into_iter()
        .filter(|m| !m.is_empty())
        .map(|m| m.into_iter().map(|at| occurrences[at].clone()).collect())
        .collect()
}

/// A person as the library shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    /// The fullest given name seen, with the most support among those.
    pub name: String,
    /// The other forms printed, most frequent first.
    pub variants: Vec<String>,
    /// Item ids, each once, in occurrence order.
    pub papers: Vec<String>,
    /// The family name as most items print it.
    pub family: String,
}

/// Persons by id, and the person of each author slot.
#[derive(Clone, Debug, Default)]
pub struct PersonIndex {
    persons: Vec<Person>,
    by_id: HashMap<String, usize>,
    /// (item id, author position) -> person.
    by_slot: HashMap<(String, usize), usize>,
    /// A name as compared -> the persons printed under it.
    by_name: HashMap<NameKey, Vec<usize>>,
}

impl PersonIndex {
    pub fn person(&self, id: &str) -> Option<&Person> {
        self.by_id.get(id).map(|&at| &self.persons[at])
    }

    /// The person of an item's `position`th author, when that person has
    /// more than one item.
    pub fn person_at(&self, paper_id: &str, position: usize) -> Option<&Person> {
        self.by_slot
            .get(&(paper_id.to_owned(), position))
            .map(|&at| &self.persons[at])
            .filter(|person| person.papers.len() > 1)
    }

    /// The one person printed under this name, if exactly one is.
    pub fn person_named(&self, author: &Author) -> Option<&Person> {
        match self.by_name.get(&NameKey::of(author)?)?.as_slice() {
            [only] => Some(&self.persons[*only]),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn persons(&self) -> &[Person] {
        &self.persons
    }
}

fn display(author: &Author) -> String {
    [author.given.trim(), author.family.trim()]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ids for the clusters (keeping `before`'s where members carried over),
/// what is stored, and the index.
fn name_persons(
    papers: &[PaperRecord],
    clusters: &[Vec<Occurrence>],
    before: &[PersonRecord],
) -> (Vec<PersonRecord>, PersonIndex) {
    let member_key = |o: &Occurrence| format!("{}#{}", papers[o.paper].id, o.key.text());
    let previous: HashMap<&str, &str> = before
        .iter()
        .flat_map(|p| p.members.iter().map(move |m| (m.as_str(), p.id.as_str())))
        .collect();
    // Bigger clusters choose first, then in occurrence order.
    let mut order: Vec<usize> = (0..clusters.len()).collect();
    order.sort_by_key(|&c| std::cmp::Reverse(clusters[c].len()));
    let mut ids: Vec<String> = vec![String::new(); clusters.len()];
    let mut taken: HashSet<String> = HashSet::new();
    for &c in &order {
        let mut votes: BTreeMap<&str, usize> = BTreeMap::new();
        for o in &clusters[c] {
            if let Some(id) = previous.get(member_key(o).as_str()) {
                *votes.entry(id).or_default() += 1;
            }
        }
        let kept = votes
            .iter()
            .filter(|(id, _)| !taken.contains(**id))
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(id, _)| (*id).to_owned());
        let id = kept.unwrap_or_else(|| {
            let seed = member_key(&clusters[c][0]);
            let mut n = 0u32;
            loop {
                let candidate = format!("p{:016x}", fnv(&format!("{seed}/{n}")));
                if !taken.contains(&candidate) {
                    break candidate;
                }
                n += 1;
            }
        });
        taken.insert(id.clone());
        ids[c] = id;
    }

    let mut index = PersonIndex::default();
    let mut stored = Vec::new();
    for (c, cluster) in clusters.iter().enumerate() {
        let printed: Vec<&Author> = cluster
            .iter()
            .map(|o| &papers[o.paper].authors()[o.position])
            .collect();
        let mut forms: Vec<(String, usize, &Author)> = Vec::new();
        for author in &printed {
            let form = display(author);
            match forms.iter_mut().find(|(f, _, _)| *f == form) {
                Some(entry) => entry.1 += 1,
                None => forms.push((form, 1, author)),
            }
        }
        let fullness = |a: &Author| {
            let key = NameKey::of(a);
            let given = key.map(|k| k.given).unwrap_or_default();
            (
                given.iter().filter(|g| g.chars().count() > 1).count(),
                given.len(),
            )
        };
        let best = forms
            .iter()
            .max_by(|a, b| {
                fullness(a.2)
                    .cmp(&fullness(b.2))
                    .then(a.1.cmp(&b.1))
                    .then(b.0.cmp(&a.0))
            })
            .map(|f| f.0.clone())
            .unwrap_or_default();
        let mut variants: Vec<(String, usize)> = forms
            .iter()
            .filter(|f| f.0 != best)
            .map(|f| (f.0.clone(), f.1))
            .collect();
        variants.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut families: Vec<(&str, usize)> = Vec::new();
        for author in &printed {
            let family = author.family.trim();
            match families.iter_mut().find(|(f, _)| *f == family) {
                Some(entry) => entry.1 += 1,
                None => families.push((family, 1)),
            }
        }
        let family = families
            .iter()
            .max_by(|a, b| {
                let cased = |f: &str| f.chars().any(char::is_lowercase);
                a.1.cmp(&b.1)
                    .then(cased(a.0).cmp(&cased(b.0)))
                    .then(b.0.cmp(a.0))
            })
            .map(|f| f.0.to_owned())
            .unwrap_or_default();
        let mut paper_ids: Vec<String> = Vec::new();
        for o in cluster {
            let id = &papers[o.paper].id;
            if !paper_ids.contains(id) {
                paper_ids.push(id.clone());
            }
        }
        let at = index.persons.len();
        index.by_id.insert(ids[c].clone(), at);
        for o in cluster {
            index
                .by_slot
                .insert((papers[o.paper].id.clone(), o.position), at);
            let list = index.by_name.entry(o.key.clone()).or_default();
            if !list.contains(&at) {
                list.push(at);
            }
        }
        if paper_ids.len() > 1 {
            let mut members: Vec<String> = cluster.iter().map(member_key).collect();
            members.sort();
            stored.push(PersonRecord {
                id: ids[c].clone(),
                members,
            });
        }
        index.persons.push(Person {
            id: ids[c].clone(),
            name: best,
            variants: variants.into_iter().map(|(v, _)| v).collect(),
            papers: paper_ids,
            family,
        });
    }
    stored.sort_by(|a, b| a.id.cmp(&b.id));
    (stored, index)
}

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl super::store::PaperStore {
    /// Bring the persons up to date with the author lists. Every save
    /// calls it. Returns whether the stored persons changed.
    pub fn refresh_persons(&mut self) -> bool {
        self.persons.refresh(&self.papers)
    }

    pub fn persons(&self) -> &PersonIndex {
        self.persons.index()
    }

    /// A person and their items in list order.
    pub fn person_items(&self, id: &str) -> AppResult<PersonItemsDto> {
        let person = self
            .persons()
            .person(id)
            .ok_or_else(|| AppError::Internal(format!("no person {id} in the library")))?;
        let papers: Vec<&PaperRecord> = self
            .papers
            .iter()
            .filter(|paper| person.papers.contains(&paper.id))
            .collect();
        Ok(PersonItemsDto {
            id: person.id.clone(),
            name: person.name.clone(),
            variants: person.variants.clone(),
            rows: self.rows_in_list_order(papers),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{MetaField, MetaSource};

    fn author(given: &str, family: &str) -> Author {
        Author {
            family: family.to_owned(),
            given: given.to_owned(),
        }
    }

    fn paper(id: &str, authors: &[(&str, &str)]) -> PaperRecord {
        let mut paper = PaperRecord::new();
        paper.id = id.to_owned();
        paper.meta.authors = Some(MetaField {
            value: authors.iter().map(|(g, f)| author(g, f)).collect(),
            source: MetaSource::FirstPage,
        });
        paper
    }

    fn key(given: &str, family: &str) -> NameKey {
        NameKey::of(&author(given, family)).expect("name")
    }

    /// Each person with more than one item as its sorted item ids.
    fn people(papers: &[PaperRecord], links: &[(&str, &str)]) -> Vec<Vec<String>> {
        let mut section = PersonsSection::default();
        section.set_topic_links(
            links
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
        );
        section.refresh(papers);
        let mut out: Vec<Vec<String>> = section
            .index()
            .persons()
            .iter()
            .filter(|p| p.papers.len() > 1)
            .map(|p| {
                let mut ids = p.papers.clone();
                ids.sort();
                ids
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn names_compare_by_family_and_compatible_initials() {
        assert!(compatible(&key("J.", "Smith"), &key("John", "Smith")));
        assert!(compatible(&key("J. A.", "Smith"), &key("John", "Smith")));
        assert!(compatible(
            &key("J.A.", "Smith"),
            &key("John Andrew", "Smith")
        ));
        assert!(compatible(&key("JA", "Smith"), &key("J. A.", "Smith")));
        assert!(!compatible(&key("J.", "Smith"), &key("K.", "Smith")));
        assert!(!compatible(&key("J. A.", "Smith"), &key("J. B.", "Smith")));
        assert!(!compatible(&key("John", "Smith"), &key("James", "Smith")));
        // Particles, case and diacritics.
        assert!(compatible(
            &key("J. D.", "van der Waals"),
            &key("Johannes Diderik", "Waals")
        ));
        assert!(compatible(
            &key("Johannes van der", "Waals"),
            &key("J.", "Van der Waals")
        ));
        assert!(compatible(&key("Kurt", "Gödel"), &key("K.", "GODEL")));
        assert!(compatible(
            &key("M.", "Garcia-Lopez"),
            &key("Maria", "García López")
        ));
        assert!(!compatible(&key("J.", "Smith"), &key("J.", "Smyth")));
        assert!(same_name(
            &author("A.", "Zhong"),
            &author("Adrianne", "Zhong")
        ));
    }

    #[test]
    fn a_shared_coauthor_merges_variants_of_one_person() {
        let papers = [
            paper("a", &[("J.", "Smith"), ("Ada", "Lovelace")]),
            paper("b", &[("John", "Smith"), ("A.", "Lovelace")]),
            paper("c", &[("J.", "Smith"), ("Kurt", "Other")]),
        ];
        let found = people(&papers, &[]);
        assert!(
            found.contains(&vec!["a".to_owned(), "b".to_owned()]),
            "{found:?}"
        );
        // "J. Smith" on c has no evidence: not merged.
        assert!(
            !found.iter().any(|p| p.contains(&"c".to_owned())),
            "{found:?}"
        );
    }

    #[test]
    fn identical_full_given_names_merge_and_different_ones_never_do() {
        let papers = [
            paper("a", &[("John", "Smith")]),
            paper("b", &[("John A.", "Smith")]),
            paper("c", &[("James", "Smith"), ("Ada", "Lovelace")]),
            paper("d", &[("J.", "Smith"), ("A.", "Lovelace")]),
        ];
        let found = people(&papers, &[]);
        assert!(
            found.contains(&vec!["a".to_owned(), "b".to_owned()]),
            "{found:?}"
        );
        // d's "J. Smith" shares a coauthor with c's James, so it joins him;
        // James and John stay two people.
        assert!(
            found.contains(&vec!["c".to_owned(), "d".to_owned()]),
            "{found:?}"
        );
        assert!(
            !found
                .iter()
                .any(|p| p.contains(&"a".to_owned()) && p.len() > 2),
            "{found:?}"
        );
    }

    #[test]
    fn an_initial_never_bridges_two_spelled_out_names() {
        // "J. Smith" has evidence with both John (coauthor Lovelace) and
        // James (coauthor Goedel), but John and James are two people.
        let papers = [
            paper("a", &[("John", "Smith"), ("Ada", "Lovelace")]),
            paper("b", &[("J.", "Smith"), ("A.", "Lovelace"), ("K.", "Gödel")]),
            paper("c", &[("James", "Smith"), ("Kurt", "Godel")]),
        ];
        let mut section = PersonsSection::default();
        section.refresh(&papers);
        let index = section.index();
        let john = index.person_at("a", 0).expect("John Smith");
        assert_eq!(john.papers, ["a", "b"]);
        assert!(index.person_at("c", 0).is_none(), "James stays apart");
    }

    #[test]
    fn incompatible_initials_and_a_common_surname_stay_apart() {
        let papers = [
            paper("a", &[("J.", "Wang"), ("Li", "Chen")]),
            paper("b", &[("K.", "Wang"), ("Li", "Chen")]),
            paper("c", &[("Y.", "Wang"), ("Li", "Chen")]),
            paper("d", &[("J. A.", "Smith"), ("X.", "Coauthor")]),
            paper("e", &[("J. B.", "Smith"), ("X.", "Coauthor")]),
            paper("f", &[("J.", "Wang"), ("Li", "Chen")]),
        ];
        let mut section = PersonsSection::default();
        section.refresh(&papers);
        let index = section.index();
        assert_eq!(
            index.person_at("a", 1).expect("Li Chen").papers,
            ["a", "b", "c", "f"]
        );
        // The J. Wangs share a coauthor and are one person; K. and Y. are
        // not J. however often they write with Li Chen.
        assert_eq!(index.person_at("a", 0).expect("J. Wang").papers, ["a", "f"]);
        assert!(index.person_at("b", 0).is_none());
        assert!(index.person_at("c", 0).is_none());
        assert!(index.person_at("d", 0).is_none());
        assert!(index.person_at("e", 0).is_none());
    }
    #[test]
    fn a_citation_link_on_one_topic_merges() {
        let papers = [
            paper("a", &[("J.", "Smith")]),
            paper("b", &[("J.", "Smith")]),
        ];
        assert!(people(&papers, &[]).is_empty());
        assert_eq!(
            people(&papers, &[("a", "b")]),
            [vec!["a".to_owned(), "b".to_owned()]]
        );
    }

    #[test]
    fn particles_and_diacritics_cluster() {
        let papers = [
            paper("a", &[("Johannes Diderik", "van der Waals")]),
            paper("b", &[("Johannes", "Waals")]),
            paper("c", &[("Kurt", "Gödel")]),
            paper("d", &[("Kurt", "Godel")]),
        ];
        let found = people(&papers, &[]);
        assert_eq!(
            found,
            [
                vec!["a".to_owned(), "b".to_owned()],
                vec!["c".to_owned(), "d".to_owned()]
            ]
        );
    }

    #[test]
    fn the_best_form_is_the_fullest_and_variants_are_kept() {
        let papers = [
            paper("a", &[("J.", "Smith"), ("Ada", "Lovelace")]),
            paper("b", &[("John", "Smith"), ("A.", "Lovelace")]),
            paper("c", &[("J.", "Smith"), ("Ada", "Lovelace")]),
        ];
        let mut section = PersonsSection::default();
        section.refresh(&papers);
        let smith = section.index().person_at("a", 0).expect("person").clone();
        assert_eq!(smith.name, "John Smith");
        assert_eq!(smith.variants, ["J. Smith"]);
        assert_eq!(smith.papers, ["a", "b", "c"]);
        let ada = section.index().person_at("b", 1).expect("person");
        assert_eq!(ada.name, "Ada Lovelace");
    }

    #[test]
    fn clusters_and_ids_are_deterministic_and_survive_new_items() {
        let papers = vec![
            paper("a", &[("J.", "Smith"), ("Ada", "Lovelace")]),
            paper("b", &[("John", "Smith"), ("A.", "Lovelace")]),
        ];
        let mut one = PersonsSection::default();
        assert!(one.refresh(&papers));
        let mut reversed = papers.clone();
        reversed.reverse();
        let mut two = PersonsSection::default();
        two.refresh(&reversed);
        assert_eq!(one.people, two.people, "order of records does not matter");
        assert!(!one.refresh(&papers), "unchanged input");

        // A new item by the same person keeps the id.
        let id = one.index().person_at("a", 0).expect("person").id.clone();
        let mut more = papers.clone();
        more.push(paper("0", &[("John", "Smith")]));
        assert!(one.refresh(&more));
        let person = one.index().person_at("0", 0).expect("person");
        assert_eq!(person.id, id);
        assert_eq!(person.papers.len(), 3);

        // Round trip through the file keeps ids.
        let json = serde_json::to_string(&one).expect("json");
        let mut loaded: PersonsSection = serde_json::from_str::<PersonsSection>(&json)
            .expect("parse")
            .loaded();
        loaded.refresh(&more);
        assert_eq!(loaded.index().person_at("b", 0).expect("person").id, id);
        // Another version is rebuilt.
        let old: PersonsSection =
            serde_json::from_str(r#"{"version":0,"people":[{"id":"x","members":[]}]}"#)
                .expect("parse");
        assert!(old.loaded().people.is_empty());
    }
}
