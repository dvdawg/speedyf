//! Title normalization and matching for title searches (projects.md sec. 4).
//!
//! A title search - on request, for a paper that has no identifier - comes
//! back with up to five candidates from each source asked. Whether one is
//! applied without asking is decided here, by the rule the user chose:
//! **exact, or a near match with the same year**. Exact is equal normalized
//! titles; near is a similarity of [`NEAR_THRESHOLD`] or better with both
//! years known and equal; anything less is parked on the paper
//! (`PaperDto::candidates`) for the user to pick from.
//! [`merge_candidates`] folds the sources' lists into one, so the same work
//! found by three services is one row with the best record's fields.
//!
//! Nothing here touches the network.

use crate::library::types::LookupCandidateDto;
use std::cmp::{Ordering, Reverse};
use std::collections::BTreeSet;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Similarity at or above which two titles are a near match.
pub const NEAR_THRESHOLD: f32 = 0.85;
/// A title with fewer words than this is compared by character bigrams
/// rather than by words (see [`similarity`]).
const SHORT_TITLE_WORDS: usize = 4;
/// The prefix of the DOIs DataCite registers for arXiv; such a DOI names
/// the preprint, so it is compared as an arXiv id, not as a DOI.
const ARXIV_DOI_PREFIX: &str = "10.48550/arxiv.";

/// The form titles are compared in: lower-case; NFKD with combining marks
/// stripped, so `Schrödinger` and `Schrodinger` agree and `ﬁ` is `fi`;
/// everything but letters, digits and spaces dropped, so punctuation,
/// hyphens and a trailing period do not count; spaces collapsed.
///
/// Dropping a hyphen rather than spacing it makes `non-equilibrium` match
/// `nonequilibrium`; `free-energy` against `free energy` is then a two-word
/// difference, which is why the exact test and [`similarity`] also compare
/// the titles with their spaces removed ([`squeeze`]): spacing inside
/// words never counts.
pub fn normalize(title: &str) -> String {
    let folded: String = title.nfkd().filter(|c| !is_combining_mark(*c)).collect();
    let mut out = String::with_capacity(folded.len());
    let mut space_pending = false;
    for c in folded.chars() {
        if c.is_alphanumeric() {
            if space_pending && !out.is_empty() {
                out.push(' ');
            }
            space_pending = false;
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() {
            space_pending = true;
        }
    }
    out
}

/// A normalized title without its spaces, for the comparison that ignores
/// where words begin and end.
fn squeeze(normalized: &str) -> String {
    normalized.chars().filter(|c| *c != ' ').collect()
}

/// How alike two titles are, 0 to 1. Both are normalized first; titles
/// that differ only in spacing or hyphenation score 1.
///
/// For titles of four or more words it is the Jaccard index of their word
/// sets: shared words over all words, so word order does not matter and one
/// word added or changed in six costs about 0.15. Short titles have too few
/// words for that to grade anything (two words, one changed, is 0.33
/// whether the change is `Computing` -> `Computation` or -> `Chemistry`), so
/// for them it is the Dice coefficient over the character bigrams of the
/// space-free text, which credits a shared stem and charges a different
/// word in proportion to its length.
pub fn similarity(a: &str, b: &str) -> f32 {
    similarity_normalized(&normalize(a), &normalize(b))
}

fn similarity_normalized(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b || squeeze(a) == squeeze(b) {
        return 1.0;
    }
    let words_a: BTreeSet<&str> = a.split(' ').collect();
    let words_b: BTreeSet<&str> = b.split(' ').collect();
    if words_a.len() < SHORT_TITLE_WORDS || words_b.len() < SHORT_TITLE_WORDS {
        return bigram_dice(a, b);
    }
    let shared = words_a.intersection(&words_b).count();
    let all = words_a.union(&words_b).count();
    shared as f32 / all as f32
}

/// Dice over character bigrams, as multisets, of the text without spaces.
fn bigram_dice(a: &str, b: &str) -> f32 {
    let bigrams = |text: &str| -> Vec<(char, char)> {
        let chars: Vec<char> = squeeze(text).chars().collect();
        let mut pairs: Vec<(char, char)> = chars.windows(2).map(|w| (w[0], w[1])).collect();
        pairs.sort_unstable();
        pairs
    };
    let (a, b) = (bigrams(a), bigrams(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (2 * shared) as f32 / (a.len() + b.len()) as f32
}

/// How well a candidate matches the paper it was found for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchQuality {
    /// The normalized titles are equal. Applied without asking.
    Exact,
    /// Similarity >= [`NEAR_THRESHOLD`] and the same year on both sides.
    /// Applied without asking - the user's rule.
    Near,
    /// Anything less, including a near title with a year missing or
    /// different. Parked for the user.
    Weak,
}

impl MatchQuality {
    /// Whether the lookup chain applies a candidate of this quality on its
    /// own: exact, or near with the same year.
    pub fn applies(self) -> bool {
        !matches!(self, MatchQuality::Weak)
    }
}

/// Grades a candidate against the paper. `Exact` when the normalized titles
/// are equal, spacing aside (the year is not consulted: a title that
/// matches to the letter is the work, and the paper often has no year to
/// compare); `Near` when
/// the similarity is at least [`NEAR_THRESHOLD`] and both years are known
/// and equal; else `Weak`.
pub fn judge(
    paper_title: &str,
    paper_year: Option<i32>,
    candidate_title: &str,
    candidate_year: Option<i32>,
) -> MatchQuality {
    let (paper, candidate) = (normalize(paper_title), normalize(candidate_title));
    if !paper.is_empty() && (paper == candidate || squeeze(&paper) == squeeze(&candidate)) {
        return MatchQuality::Exact;
    }
    let same_year = matches!((paper_year, candidate_year), (Some(a), Some(b)) if a == b);
    if same_year && similarity_normalized(&paper, &candidate) >= NEAR_THRESHOLD {
        MatchQuality::Near
    } else {
        MatchQuality::Weak
    }
}

/// Folds several sources' candidate lists into one. Two candidates are the
/// same work when their DOIs agree, failing that (either lacks one) when
/// their arXiv ids agree, failing that when their normalized titles are
/// equal; two that both carry DOIs - or both arXiv ids - that differ are
/// different works whatever their titles say. An arXiv DOI
/// (`10.48550/arxiv....`) counts as the arXiv id it stands for, not as a DOI.
/// The higher-ranked source's record is kept and its empty fields filled
/// from the others; the result is in source rank order, highest first,
/// each source's own order within.
pub fn merge_candidates(lists: Vec<Vec<LookupCandidateDto>>) -> Vec<LookupCandidateDto> {
    let mut all: Vec<LookupCandidateDto> = lists.into_iter().flatten().collect();
    all.sort_by_key(|candidate| Reverse(candidate.source.rank()));
    let mut merged: Vec<(String, LookupCandidateDto)> = Vec::new();
    for candidate in all {
        let title = normalize(&candidate.title);
        let same = merged
            .iter_mut()
            .find(|(kept_title, kept)| same_work(kept, kept_title, &candidate, &title));
        match same {
            Some((_, kept)) => fill_from(kept, candidate),
            None => merged.push((title, candidate)),
        }
    }
    merged.into_iter().map(|(_, candidate)| candidate).collect()
}

fn same_work(
    kept: &LookupCandidateDto,
    kept_title: &str,
    other: &LookupCandidateDto,
    other_title: &str,
) -> bool {
    if let (Some(a), Some(b)) = (proper_doi(kept), proper_doi(other)) {
        return a.eq_ignore_ascii_case(b);
    }
    if let (Some(a), Some(b)) = (arxiv_id(kept), arxiv_id(other)) {
        return a.eq_ignore_ascii_case(&b);
    }
    !kept_title.is_empty() && kept_title == other_title
}

/// The candidate's DOI unless it is an arXiv DOI.
fn proper_doi(candidate: &LookupCandidateDto) -> Option<&str> {
    let doi = candidate.doi.as_deref()?.trim();
    let is_arxiv = doi
        .get(..ARXIV_DOI_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(ARXIV_DOI_PREFIX));
    (!doi.is_empty() && !is_arxiv).then_some(doi)
}

/// The candidate's arXiv id, or the one its arXiv DOI stands for.
fn arxiv_id(candidate: &LookupCandidateDto) -> Option<String> {
    if let Some(id) = candidate.arxiv.as_deref().map(str::trim) {
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    let doi = candidate.doi.as_deref()?.trim();
    let head = doi.get(..ARXIV_DOI_PREFIX.len())?;
    if !head.eq_ignore_ascii_case(ARXIV_DOI_PREFIX) {
        return None;
    }
    let id = doi[ARXIV_DOI_PREFIX.len()..].trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// What tells a candidate apart once the user has declined it, so a later
/// search does not park the same work again: its DOI, else its arXiv id (an
/// arXiv DOI counts as one), else its normalized title.
pub fn identity(candidate: &LookupCandidateDto) -> String {
    if let Some(doi) = proper_doi(candidate) {
        return format!("doi:{}", doi.to_ascii_lowercase());
    }
    if let Some(id) = arxiv_id(candidate) {
        return format!("arxiv:{}", id.to_ascii_lowercase());
    }
    format!("title:{}", normalize(&candidate.title))
}

/// Fills what `kept` lacks from `other`. The title and source stay.
fn fill_from(kept: &mut LookupCandidateDto, other: LookupCandidateDto) {
    if kept.authors.is_empty() {
        kept.authors = other.authors;
    }
    fill(&mut kept.year, other.year);
    fill(&mut kept.venue, other.venue);
    fill(&mut kept.volume, other.volume);
    fill(&mut kept.issue, other.issue);
    fill(&mut kept.pages, other.pages);
    fill(&mut kept.publisher, other.publisher);
    fill(&mut kept.abstract_, other.abstract_);
    fill(&mut kept.kind, other.kind);
    fill(&mut kept.doi, other.doi);
    fill(&mut kept.arxiv, other.arxiv);
    fill(&mut kept.isbn, other.isbn);
}

fn fill<T>(slot: &mut Option<T>, value: Option<T>) {
    if slot.is_none() {
        *slot = value;
    }
}

// ---------------------------------------------------------------------------
// Tests. The auto-apply rule under test is the user's: exact, or a near
// match with the same year.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{Author, MetaSource, PaperKind};

    const JARZYNSKI: &str = "Nonequilibrium Equality for Free Energy Differences";

    // -- normalize --------------------------------------------------------

    #[test]
    fn normalization_folds_case_marks_punctuation_and_space() {
        assert_eq!(
            normalize(JARZYNSKI),
            "nonequilibrium equality for free energy differences"
        );
        assert_eq!(
            normalize("  Schrödinger's   Cat:  A Review. "),
            "schrodingers cat a review"
        );
        assert_eq!(normalize("Non-equilibrium"), "nonequilibrium");
        assert_eq!(
            normalize("Self-Organized Criticality"),
            "selforganized criticality"
        );
        assert_eq!(
            normalize("Eﬃcient ﬁltering"),
            "efficient filtering",
            "ligatures"
        );
        assert_eq!(
            normalize("Ｆｕｌｌｗｉｄｔｈ 2024"),
            "fullwidth 2024",
            "compatibility forms"
        );
        assert_eq!(
            normalize("Title with a trailing period."),
            "title with a trailing period"
        );
        assert_eq!(normalize("«Quoted» — and dashed"), "quoted and dashed");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize(" ... "), "");
        assert_eq!(
            normalize("Über Σ-Modelle"),
            "uber σmodelle",
            "non-latin letters stay"
        );
    }

    // -- similarity -------------------------------------------------------

    #[test]
    fn identical_and_reordered_titles_score_one() {
        assert_eq!(similarity(JARZYNSKI, JARZYNSKI), 1.0);
        assert_eq!(
            similarity(
                JARZYNSKI,
                "nonequilibrium equality for free-energy differences"
            ),
            1.0
        );
        assert_eq!(
            similarity(
                "Free Energy Differences: Nonequilibrium Equality For",
                JARZYNSKI
            ),
            1.0,
            "word order does not count"
        );
        assert_eq!(similarity("Deep Learning", "Deep learning."), 1.0);
        assert_eq!(
            similarity(
                "Free-Energy Calculations of Proteins",
                "Free energy calculations of proteins"
            ),
            1.0,
            "spacing inside words never counts"
        );
    }

    #[test]
    fn one_word_in_six_costs_about_a_seventh() {
        let revisited = "Nonequilibrium Equality for Free Energy Differences Revisited";
        let score = similarity(JARZYNSKI, revisited);
        assert!((score - 6.0 / 7.0).abs() < 1e-6, "{score}");
        assert!(score >= NEAR_THRESHOLD);

        let changed = "Nonequilibrium Inequality for Free Energy Differences";
        let score = similarity(JARZYNSKI, changed);
        assert!((score - 5.0 / 7.0).abs() < 1e-6, "{score}");
        assert!(score < NEAR_THRESHOLD);
    }

    #[test]
    fn short_titles_are_graded_by_bigrams() {
        // Two words, one changed: Jaccard would say 0.33 for both of these;
        // bigrams tell a stem change from a different word.
        let stem = similarity("Quantum Computing", "Quantum Computation");
        assert!((0.75..NEAR_THRESHOLD).contains(&stem), "{stem}");
        let different = similarity("Quantum Computing", "Quantum Chemistry");
        assert!(different < 0.6, "{different}");
        assert!(stem > different);
        assert_eq!(similarity("A", "B"), 0.0);
        assert_eq!(similarity("A", "A"), 1.0);
    }

    #[test]
    fn nothing_is_like_an_empty_title() {
        assert_eq!(similarity("", JARZYNSKI), 0.0);
        assert_eq!(similarity(JARZYNSKI, " . "), 0.0);
        assert_eq!(similarity("", ""), 0.0);
    }

    // -- judge: the user's rule -------------------------------------------

    #[test]
    fn an_exact_title_applies_with_or_without_years() {
        assert_eq!(
            judge(JARZYNSKI, Some(1997), JARZYNSKI, Some(1997)),
            MatchQuality::Exact
        );
        assert_eq!(
            judge(JARZYNSKI, None, JARZYNSKI, Some(1997)),
            MatchQuality::Exact
        );
        assert_eq!(
            judge(JARZYNSKI, Some(1997), JARZYNSKI, None),
            MatchQuality::Exact
        );
        assert_eq!(
            judge(JARZYNSKI, Some(1997), JARZYNSKI, Some(2013)),
            MatchQuality::Exact,
            "an exact title is the work; a reprint's year does not unmatch it"
        );
        assert_eq!(
            judge(
                "non-equilibrium equality for free energy differences.",
                None,
                JARZYNSKI,
                None
            ),
            MatchQuality::Exact
        );
        assert_eq!(
            judge(
                "Free-Energy Calculations of Proteins",
                None,
                "Free energy calculations of proteins",
                None
            ),
            MatchQuality::Exact,
            "hyphenated against open: the same words"
        );
        assert!(MatchQuality::Exact.applies());
    }

    #[test]
    fn a_near_title_applies_only_with_the_same_year_on_both_sides() {
        let near = "Nonequilibrium Equality for Free Energy Differences Revisited";
        assert_eq!(
            judge(JARZYNSKI, Some(1997), near, Some(1997)),
            MatchQuality::Near
        );
        assert!(MatchQuality::Near.applies());
        assert_eq!(
            judge(JARZYNSKI, Some(1997), near, Some(1998)),
            MatchQuality::Weak
        );
        assert_eq!(judge(JARZYNSKI, None, near, Some(1997)), MatchQuality::Weak);
        assert_eq!(judge(JARZYNSKI, Some(1997), near, None), MatchQuality::Weak);
        assert_eq!(judge(JARZYNSKI, None, near, None), MatchQuality::Weak);
        assert!(!MatchQuality::Weak.applies());
    }

    #[test]
    fn a_title_below_the_threshold_is_weak_even_with_the_year() {
        let far = "Nonequilibrium Inequality for Free Energy Differences";
        assert_eq!(
            judge(JARZYNSKI, Some(1997), far, Some(1997)),
            MatchQuality::Weak
        );
        assert_eq!(
            judge(
                "Quantum Computing",
                Some(2000),
                "Quantum Computation",
                Some(2000)
            ),
            MatchQuality::Weak
        );
        assert_eq!(judge("", Some(2000), "", Some(2000)), MatchQuality::Weak);
    }

    // -- merge ------------------------------------------------------------

    fn candidate(source: MetaSource, title: &str) -> LookupCandidateDto {
        LookupCandidateDto {
            source,
            title: title.to_string(),
            authors: Vec::new(),
            year: None,
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

    fn author(family: &str) -> Author {
        Author {
            family: family.to_string(),
            given: String::new(),
        }
    }

    #[test]
    fn merge_dedupes_by_doi_keeps_the_best_source_and_fills_its_gaps() {
        let mut crossref = candidate(MetaSource::Crossref, JARZYNSKI);
        crossref.doi = Some("10.1103/physrevlett.78.2690".into());
        crossref.year = Some(1997);
        crossref.venue = Some("Physical Review Letters".into());

        let mut openalex = candidate(
            MetaSource::Openalex,
            "Nonequilibrium equality for free energy differences",
        );
        openalex.doi = Some("10.1103/PhysRevLett.78.2690".into());
        openalex.year = Some(1996);
        openalex.authors = vec![author("Jarzynski")];
        openalex.arxiv = Some("cond-mat/9610209".into());
        openalex.abstract_ = Some("An equality…".into());

        let mut s2 = candidate(MetaSource::SemanticScholar, JARZYNSKI);
        s2.doi = Some("10.1103/physrevlett.78.2690".into());
        s2.kind = Some(PaperKind::Article);
        s2.venue = Some("Phys. Rev. Lett.".into());

        let merged = merge_candidates(vec![vec![openalex], vec![crossref], vec![s2]]);
        assert_eq!(merged.len(), 1);
        let one = &merged[0];
        assert_eq!(one.source, MetaSource::Crossref, "the highest rank is kept");
        assert_eq!(one.title, JARZYNSKI);
        assert_eq!(one.year, Some(1997), "a filled field is not overwritten");
        assert_eq!(one.venue.as_deref(), Some("Physical Review Letters"));
        assert_eq!(
            one.authors,
            vec![author("Jarzynski")],
            "empty authors are filled"
        );
        assert_eq!(one.arxiv.as_deref(), Some("cond-mat/9610209"));
        assert_eq!(one.abstract_.as_deref(), Some("An equality…"));
        assert_eq!(one.kind, Some(PaperKind::Article), "from the third source");
    }

    #[test]
    fn merge_dedupes_by_arxiv_id_then_by_title() {
        let mut arxiv = candidate(MetaSource::Arxiv, "Active Matter");
        arxiv.arxiv = Some("2401.12345".into());
        let mut inspire = candidate(MetaSource::Inspire, "Active matter.");
        inspire.arxiv = Some("2401.12345".into());
        inspire.doi = Some("10.1103/revmodphys.85.1143".into());
        let dblp = candidate(MetaSource::Dblp, "Active Matter");
        let mut other = candidate(MetaSource::Dblp, "Passive Matter");
        other.year = Some(2020);

        let merged = merge_candidates(vec![vec![arxiv], vec![dblp, other], vec![inspire]]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].source, MetaSource::Inspire);
        assert_eq!(merged[0].doi.as_deref(), Some("10.1103/revmodphys.85.1143"));
        assert_eq!(merged[0].arxiv.as_deref(), Some("2401.12345"));
        assert_eq!(merged[1].title, "Passive Matter");
    }

    #[test]
    fn merge_keeps_works_apart_when_their_identifiers_disagree() {
        let mut journal = candidate(MetaSource::Crossref, "Same Title");
        journal.doi = Some("10.1000/journal".into());
        let mut conference = candidate(MetaSource::Crossref, "Same Title");
        conference.doi = Some("10.1000/conference".into());
        let mut preprint_a = candidate(MetaSource::Arxiv, "Same Title");
        preprint_a.arxiv = Some("2401.00001".into());
        let mut preprint_b = candidate(MetaSource::Arxiv, "Same Title");
        preprint_b.arxiv = Some("2401.00002".into());

        let merged = merge_candidates(vec![
            vec![journal, conference],
            vec![preprint_a, preprint_b],
        ]);
        // The two DOIs stay apart; each preprint has no DOI, so it joins the
        // first title match without an arXiv id of its own - the journal
        // record - and the second preprint then differs from it by arXiv id
        // and joins the conference record.
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].doi.as_deref(), Some("10.1000/journal"));
        assert_eq!(merged[0].arxiv.as_deref(), Some("2401.00001"));
        assert_eq!(merged[1].doi.as_deref(), Some("10.1000/conference"));
        assert_eq!(merged[1].arxiv.as_deref(), Some("2401.00002"));
    }

    #[test]
    fn merge_reads_an_arxiv_doi_as_the_arxiv_id() {
        let mut s2 = candidate(MetaSource::SemanticScholar, "A Preprint");
        s2.doi = Some("10.48550/arxiv.2401.12345".into());
        let mut arxiv = candidate(MetaSource::Arxiv, "A preprint");
        arxiv.arxiv = Some("2401.12345".into());
        let mut crossref = candidate(MetaSource::Crossref, "A Preprint");
        crossref.doi = Some("10.1000/published".into());

        let merged = merge_candidates(vec![vec![s2], vec![arxiv], vec![crossref]]);
        assert_eq!(merged.len(), 1, "the arXiv DOI is not a conflicting DOI");
        assert_eq!(merged[0].source, MetaSource::Crossref);
        assert_eq!(merged[0].doi.as_deref(), Some("10.1000/published"));
        assert_eq!(merged[0].arxiv.as_deref(), Some("2401.12345"));
    }

    #[test]
    fn merge_orders_by_source_rank_and_keeps_each_sources_order_within() {
        let lists = vec![
            vec![
                candidate(MetaSource::Dblp, "D one"),
                candidate(MetaSource::Dblp, "D two"),
            ],
            vec![
                candidate(MetaSource::Crossref, "C one"),
                candidate(MetaSource::Crossref, "C two"),
            ],
            vec![candidate(MetaSource::Openalex, "O one")],
        ];
        let titles: Vec<String> = merge_candidates(lists)
            .into_iter()
            .map(|c| c.title)
            .collect();
        assert_eq!(titles, vec!["C one", "C two", "D one", "D two", "O one"]);
        assert!(merge_candidates(Vec::new()).is_empty());
        assert!(merge_candidates(vec![Vec::new(), Vec::new()]).is_empty());
    }

    #[test]
    fn merge_never_joins_on_an_empty_title() {
        let a = candidate(MetaSource::Dblp, "...");
        let b = candidate(MetaSource::Dblp, "???");
        assert_eq!(merge_candidates(vec![vec![a, b]]).len(), 2);
    }

    #[test]
    fn identity_is_the_doi_then_the_arxiv_id_then_the_title() {
        let mut c = candidate(MetaSource::Crossref, "A Title: here");
        assert_eq!(identity(&c), "title:a title here");
        c.arxiv = Some("2401.12345".into());
        assert_eq!(identity(&c), "arxiv:2401.12345");
        c.doi = Some("10.48550/arXiv.2401.12345".into());
        assert_eq!(
            identity(&c),
            "arxiv:2401.12345",
            "an arXiv DOI is not a DOI"
        );
        c.doi = Some("10.1103/PhysRevLett.78.2690".into());
        assert_eq!(identity(&c), "doi:10.1103/physrevlett.78.2690");
    }
}
