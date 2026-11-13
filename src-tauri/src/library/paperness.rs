//! Is this PDF a library item?
//!
//! SpeedyF is a PDF viewer first; a manual, a form, a scanned receipt and a
//! lecture handout all open in it. The reader must not treat each of them as
//! a paper waiting to be filed. So before it offers to add an open document
//! to the library, it asks what the first pages say about themselves: an
//! arXiv stamp, a DOI, a publisher's name in the header or footer, the
//! "Received ... Accepted" line of a journal, an abstract with references -
//! and, since a thesis and a textbook are library items as much as a paper
//! is, a dissertation's title page or a book's copyright page with its ISBN.
//!
//! The answer is a verdict with the reason behind it, so the panel can say
//! *why* it thinks so rather than presenting a guess as a fact. The panel
//! prefixes the reason with the kind ("Looks like a thesis: ..."), so a reason
//! is a short noun phrase. "Unknown" is the honest default and costs
//! nothing: the user can still add the file.

use super::identifiers::{arxiv_stamp, find_pii, pii_has_valid_issn};
use super::types::{PaperKind, PapernessDto, PapernessVerdict};
use super::{find_isbn, page_head};
use crate::engine::citation::find_explicit_citations;
use crate::engine::types::CitationIdDto;

/// The Info dictionary fields that help. Any may be absent.
#[derive(Default, Clone, Copy, Debug)]
pub struct InfoFields<'a> {
    pub subject: Option<&'a str>,
    pub producer: Option<&'a str>,
    pub creator: Option<&'a str>,
}

/// Fewer non-blank characters than this and a page says too little to
/// judge: a cover with a title and a name, a half-title, a blank verso.
pub const SPARSE_PAGE_CHARS: usize = 400;

/// What a publisher's name on the page says about the kind of item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkKind {
    /// A journal, or a publisher that stamps articles.
    Article,
    /// A conference or its proceedings.
    Proceedings,
    /// A preprint server: a paper, of no particular kind.
    Preprint,
    /// Journals and proceedings both; the page's own words decide.
    Either,
}

impl MarkKind {
    fn kind(self, folded: &str) -> Option<PaperKind> {
        match self {
            MarkKind::Article => Some(PaperKind::Article),
            MarkKind::Proceedings => Some(PaperKind::Inproceedings),
            MarkKind::Preprint => None,
            MarkKind::Either => venue_kind(folded),
        }
    }
}

/// Publishers and venues whose name on a first page settles the question.
/// Lower-case; matched as substrings of the folded page text, first match
/// wins. Short names carry a surrounding space or punctuation so "acm" does
/// not fire on "macmillan".
const PUBLISHER_MARKS: &[(&str, &str, MarkKind)] = &[
    ("sciencedirect", "ScienceDirect", MarkKind::Article),
    ("elsevier", "Elsevier", MarkKind::Article),
    (
        "journal of machine learning research",
        "JMLR",
        MarkKind::Article,
    ),
    ("jmlr.org", "JMLR", MarkKind::Article),
    (
        "proceedings of machine learning research",
        "PMLR",
        MarkKind::Proceedings,
    ),
    ("physical review", "an APS journal", MarkKind::Article),
    (
        "american physical society",
        "the American Physical Society",
        MarkKind::Article,
    ),
    ("phys. rev.", "an APS journal", MarkKind::Article),
    (
        "rev. mod. phys",
        "Reviews of Modern Physics",
        MarkKind::Article,
    ),
    ("taylor & francis", "Taylor & Francis", MarkKind::Article),
    ("tandfonline", "Taylor & Francis", MarkKind::Article),
    ("springer", "Springer", MarkKind::Either),
    ("nature portfolio", "Nature", MarkKind::Article),
    ("nature publishing", "Nature", MarkKind::Article),
    ("macmillan publishers", "Nature", MarkKind::Article),
    ("john wiley", "Wiley", MarkKind::Article),
    ("wiley-vch", "Wiley", MarkKind::Article),
    ("wiley online library", "Wiley", MarkKind::Article),
    (
        "proceedings of the ieee",
        "Proceedings of the IEEE",
        MarkKind::Article,
    ),
    ("ieee ", "IEEE", MarkKind::Either),
    ("ieee.", "IEEE", MarkKind::Either),
    (
        "association for computing machinery",
        "ACM",
        MarkKind::Either,
    ),
    ("acm.org", "ACM", MarkKind::Either),
    ("pnas.org", "PNAS", MarkKind::Article),
    (
        "proceedings of the national academy of sciences",
        "PNAS",
        MarkKind::Article,
    ),
    ("annual review of", "Annual Reviews", MarkKind::Article),
    ("annualreviews.org", "Annual Reviews", MarkKind::Article),
    ("iop publishing", "IOP Publishing", MarkKind::Article),
    ("institute of physics", "IOP Publishing", MarkKind::Article),
    ("iopscience", "IOP Publishing", MarkKind::Article),
    ("royal society", "the Royal Society", MarkKind::Article),
    (
        "cambridge university press",
        "Cambridge University Press",
        MarkKind::Either,
    ),
    (
        "oxford university press",
        "Oxford University Press",
        MarkKind::Either,
    ),
    (
        "society for industrial and applied mathematics",
        "SIAM",
        MarkKind::Article,
    ),
    ("siam j.", "SIAM", MarkKind::Article),
    ("american chemical society", "ACS", MarkKind::Article),
    ("pubs.acs.org", "ACS", MarkKind::Article),
    ("aip publishing", "AIP Publishing", MarkKind::Article),
    ("j. chem. phys", "an AIP journal", MarkKind::Article),
    (
        "advances in neural information processing systems",
        "NeurIPS",
        MarkKind::Proceedings,
    ),
    ("neurips", "NeurIPS", MarkKind::Proceedings),
    (
        "international conference on machine learning",
        "ICML",
        MarkKind::Proceedings,
    ),
    (
        "international conference on learning representations",
        "ICLR",
        MarkKind::Proceedings,
    ),
    ("openreview.net", "OpenReview", MarkKind::Either),
    ("arxiv.org", "arXiv", MarkKind::Preprint),
    ("biorxiv", "bioRxiv", MarkKind::Preprint),
    ("medrxiv", "medRxiv", MarkKind::Preprint),
    ("ssrn", "SSRN", MarkKind::Preprint),
    ("mdpi", "MDPI", MarkKind::Article),
    ("frontiers in ", "Frontiers", MarkKind::Article),
    ("plos ", "PLOS", MarkKind::Article),
    ("journals.plos.org", "PLOS", MarkKind::Article),
    ("sage publications", "SAGE", MarkKind::Article),
    ("de gruyter", "De Gruyter", MarkKind::Either),
    ("world scientific", "World Scientific", MarkKind::Either),
    // Last, so a journal that calls itself "Proceedings of ..." (PNAS, the
    // Royal Society, the IEEE) is taken for what it is above.
    (
        "proceedings of",
        "a conference's proceedings",
        MarkKind::Proceedings,
    ),
];

/// Tools that only ever produce papers, as named in the Info dictionary.
const PRODUCER_MARKS: &[(&str, &str, MarkKind)] = &[
    ("elsevier", "Elsevier", MarkKind::Article),
    ("springer", "Springer", MarkKind::Either),
    ("arxiv", "arXiv", MarkKind::Preprint),
    ("ieee", "IEEE", MarkKind::Either),
    ("acm ", "ACM", MarkKind::Either),
    ("aps ", "APS", MarkKind::Article),
    ("wiley", "Wiley", MarkKind::Article),
    ("iop ", "IOP Publishing", MarkKind::Article),
    ("river valley", "a journal typesetter", MarkKind::Article),
    ("cenveo", "a journal typesetter", MarkKind::Article),
    ("spi global", "a journal typesetter", MarkKind::Article),
    ("integra", "a journal typesetter", MarkKind::Article),
];

// ascii-guard: data begin - words matched in page text
/// The submission formula of a thesis title page. Nothing else says it,
/// so any of these settles the question on its own.
const THESIS_SUBMISSION_MARKS: &[&str] = &[
    "a dissertation submitted",
    "a thesis submitted",
    "dissertation submitted",
    "thesis submitted",
    "dissertation presented",
    "thesis presented",
    "in partial fulfillment",
    "in partial fulfilment",
    "in fulfillment of the requirements",
    "in fulfilment of the requirements",
    "for the degree of",
    "submitted to the faculty",
    "submitted to the graduate",
    "submitted to the department",
    "submitted in accordance with the requirements",
    "zur erlangung des",
    "thèse de doctorat",
    "thèse présentée",
    "tesis doctoral",
    "tesi di dottorato",
    "proefschrift",
];

/// A degree with the word in it: "PhD thesis", "doctoral dissertation".
const THESIS_DEGREE_MARKS: &[&str] = &[
    "ph.d. thesis",
    "ph.d thesis",
    "ph. d. thesis",
    "phd thesis",
    "ph.d. dissertation",
    "ph.d dissertation",
    "phd dissertation",
    "doctoral thesis",
    "doctoral dissertation",
    "master's thesis",
    "master’s thesis",
    "masters thesis",
    "master thesis",
    "master's dissertation",
    "master’s dissertation",
    "bachelor's thesis",
    "bachelor’s thesis",
    "bachelor thesis",
    "diploma thesis",
    "licentiate thesis",
    "habilitation thesis",
    "habilitationsschrift",
    "mphil thesis",
    "m.sc. thesis",
    "msc thesis",
];

/// Degrees that count only beside the word "thesis" or "dissertation": a
/// CV lists "Master of Science" too.
const DEGREE_NAMES: &[&str] = &[
    "doctor of philosophy",
    "doctor of science",
    "doctor of engineering",
    "master of science",
    "master of arts",
    "master of engineering",
    "master of philosophy",
    "bachelor of science",
    "bachelor of arts",
];

/// The people of a thesis, counted beside the word.
const THESIS_PEOPLE_MARKS: &[&str] = &[
    "thesis advisor",
    "thesis adviser",
    "thesis supervisor",
    "thesis committee",
    "dissertation advisor",
    "dissertation adviser",
    "dissertation supervisor",
    "dissertation committee",
    "doctoral committee",
    "examining committee",
    "committee members",
    "supervisor:",
    "supervisors:",
    "advisor:",
    "advisors:",
    "adviser:",
    "supervised by",
    "directeur de thèse",
    "doktorvater",
    "betreuer:",
    "gutachter:",
];

const UNIVERSITY_MARKS: &[&str] = &[
    "university",
    "universität",
    "universitat",
    "université",
    "universidad",
    "universidade",
    "università",
    "universiteit",
    "universitet",
    "institute of technology",
    "polytechnic",
    "politecnico",
    "école polytechnique",
    "college",
];
// ascii-guard: data end

/// Documents that talk about theses without being one.
const THESIS_TALK_MARKS: &[&str] = &[
    "syllabus",
    "template",
    "guidelines",
    "handbook",
    "course",
    "deadline",
    "grading",
    "lecture",
    "instructions for",
    "how to write",
    "regulations",
];

/// The cataloguing lines of a book's copyright page.
const CATALOGUE_MARKS: &[&str] = &[
    "library of congress cataloging",
    "library of congress cataloguing",
    "library of congress catalog card",
    "library of congress control number",
    "cataloging-in-publication",
    "cataloguing-in-publication",
    "cataloging in publication",
    "cataloguing in publication",
    "catalogue record for this book",
    "catalog record for this book",
    "british library cataloguing",
    "deutsche nationalbibliothek",
];

// ascii-guard: data begin - words matched in page text
/// Houses that publish books and (as far as a first page goes) little
/// else. Journal-heavy publishers are left out on purpose: an old
/// journal page says "Printed in Great Britain ... All rights reserved"
/// beside Pergamon or North-Holland too.
const BOOK_PRESS_MARKS: &[(&str, &str)] = &[
    ("university press", "a university press"),
    ("mit press", "MIT Press"),
    ("imperial college press", "Imperial College Press"),
    ("clarendon press", "Clarendon Press"),
    ("addison-wesley", "Addison-Wesley"),
    ("addison wesley", "Addison-Wesley"),
    ("mcgraw-hill", "McGraw-Hill"),
    ("mcgraw hill", "McGraw-Hill"),
    ("prentice hall", "Prentice Hall"),
    ("prentice-hall", "Prentice Hall"),
    ("pearson", "Pearson"),
    ("o'reilly", "O'Reilly"),
    ("o’reilly", "O'Reilly"),
    ("dover publications", "Dover"),
    ("w. h. freeman", "W. H. Freeman"),
    ("w.h. freeman", "W. H. Freeman"),
    ("freeman and company", "W. H. Freeman"),
    ("birkhäuser", "Birkhauser"),
    ("birkhauser", "Birkhauser"),
    ("butterworth-heinemann", "Butterworth-Heinemann"),
    ("chapman & hall", "Chapman & Hall"),
    ("chapman and hall", "Chapman & Hall"),
    ("crc press", "CRC Press"),
    ("morgan kaufmann", "Morgan Kaufmann"),
    ("morgan & claypool", "Morgan & Claypool"),
    ("manning publications", "Manning"),
    ("no starch press", "No Starch Press"),
    ("pragmatic bookshelf", "The Pragmatic Bookshelf"),
    ("apress", "Apress"),
    ("packt", "Packt"),
    ("cengage", "Cengage"),
    ("brooks/cole", "Brooks/Cole"),
    ("benjamin/cummings", "Benjamin/Cummings"),
    ("benjamin cummings", "Benjamin/Cummings"),
    ("jones & bartlett", "Jones & Bartlett"),
    ("jones and bartlett", "Jones & Bartlett"),
    ("basic books", "Basic Books"),
    ("w. w. norton", "W. W. Norton"),
    ("houghton mifflin", "Houghton Mifflin"),
    ("penguin", "Penguin"),
    ("random house", "Random House"),
    ("harpercollins", "HarperCollins"),
    ("simon & schuster", "Simon & Schuster"),
    ("routledge", "Routledge"),
    ("blackwell publishing", "Blackwell"),
];
// ascii-guard: data end

// ascii-guard: data begin - words matched in page text
/// What a manual says and a book does not. A manual has chapters and a
/// copyright line too; these keep it out.
const MANUAL_MARKS: &[&str] = &[
    "user manual",
    "owner's manual",
    "owner’s manual",
    "instruction manual",
    "service manual",
    "user guide",
    "user's guide",
    "user’s guide",
    "operating instructions",
    "warranty",
    "troubleshooting",
    "safety information",
    "safety instructions",
    "safety precautions",
    "firmware",
    "quick start",
    "installation guide",
    "technical support",
    "customer support",
    "customer service",
    "model no",
    "serial number",
    "part number",
    "datasheet",
    "data sheet",
];
// ascii-guard: data end

fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_space = false;
        }
    }
    out
}

/// The document's own DOI and arXiv id, as the scan reads them
/// (`entry_from_first_page`): the first DOI in the head of the page,
/// above any reference list, and the arXiv id off the stamp alone. A bare
/// `arXiv:ID` anywhere is a citation and says nothing about the document
/// but that it cites things.
fn own_identifiers(head: &str, text: &str) -> (Option<String>, Option<String>) {
    let doi = find_explicit_citations(head)
        .into_iter()
        .find_map(|occurrence| match occurrence.id {
            CitationIdDto::Doi(value) => Some(value),
            CitationIdDto::ArXiv(_) => None,
        });
    (doi, arxiv_stamp(text).map(|stamp| stamp.id))
}

fn has_word(folded: &str, word: &str) -> bool {
    folded.match_indices(word).any(|(at, _)| {
        let before = folded[..at].chars().next_back();
        let after = folded[at + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn has_any(folded: &str, marks: &[&str]) -> bool {
    marks.iter().any(|mark| folded.contains(mark))
}

/// The words of a conference paper's venue line.
fn is_proceedings_like(folded: &str) -> bool {
    has_any(
        folded,
        &[
            "proceedings of",
            "in proceedings",
            "conference proceedings",
            "conference on",
            "workshop on",
            "symposium on",
        ],
    )
}

/// The kind a venue's own words give away, for a publisher that prints
/// both journals and proceedings, or a DOI with no name beside it.
fn venue_kind(folded: &str) -> Option<PaperKind> {
    if is_proceedings_like(folded) {
        Some(PaperKind::Inproceedings)
    } else if folded.contains("transactions on")
        || has_word(folded, "journal")
        || has_word(folded, "letters")
    {
        Some(PaperKind::Article)
    } else {
        None
    }
}

/// The page-level facts the thesis and book rules share.
struct Signals<'a> {
    folded: &'a str,
    arxiv: bool,
    doi: bool,
    publisher: Option<&'static (&'static str, &'static str, MarkKind)>,
    received: bool,
    proceedings: bool,
}

impl Signals<'_> {
    /// Something a paper carries and a thesis or a book, on its own
    /// pages, does not.
    fn paper_marks(&self) -> bool {
        self.arxiv || self.doi || self.publisher.is_some()
    }

    /// The furniture of a journal page: dates, an ISSN, a volume, a venue.
    fn journal_like(&self) -> bool {
        self.received
            || self.proceedings
            || has_word(self.folded, "issn")
            || has_word(self.folded, "vol")
    }
}

/// Whether "thesis" or "dissertation" stands within a hundred characters of
/// a university's name - the layout of a title page, not of a paper whose
/// affiliations happen to share a page with the word.
fn university_near_word(folded: &str) -> bool {
    let words: Vec<usize> = ["thesis", "dissertation"]
        .iter()
        .flat_map(|word| folded.match_indices(word).map(|(at, _)| at))
        .collect();
    if words.is_empty() {
        return false;
    }
    UNIVERSITY_MARKS
        .iter()
        .flat_map(|mark| folded.match_indices(mark).map(|(at, _)| at))
        .any(|at| words.iter().any(|word| word.abs_diff(at) <= 100))
}

/// A line of the page that is a university's name and nothing else, as a
/// title page prints it. Cheap, and only decoration for the reason.
fn university_line(text: &str) -> Option<String> {
    const NOT_A_NAME: &[&str] = &[
        "department",
        "faculty",
        "school",
        "submitted",
        "committee",
        "program",
        "graduate",
        "thesis",
        "dissertation",
    ];
    text.lines()
        .map(|line| strip_leading_article(line.trim()).trim_end_matches([',', '.', ';']))
        .find(|line| {
            let count = line.chars().count();
            if !(3..=60).contains(&count)
                || line.chars().any(|c| c.is_ascii_digit())
                || !line.chars().next().is_some_and(char::is_uppercase)
            {
                return false;
            }
            let lower = line.to_lowercase();
            has_any(&lower, UNIVERSITY_MARKS) && !has_any(&lower, NOT_A_NAME)
        })
        .map(title_case_if_shouting)
}

/// "OF STANFORD UNIVERSITY", as a title page continues its sentence across
/// lines, less the "of"; "the University of Chicago" less the "the".
fn strip_leading_article(line: &str) -> &str {
    let lower = line.to_lowercase();
    for prefix in ["of the ", "of ", "the "] {
        if lower.starts_with(prefix) {
            // ASCII prefix, matched case-insensitively: the byte count is
            // the same in the original.
            return line[prefix.len()..].trim_start();
        }
    }
    line
}

/// "UNIVERSITY OF CALIFORNIA, BERKELEY" -> "University of California,
/// Berkeley". A line with any lower-case letter is left as written.
fn title_case_if_shouting(line: &str) -> String {
    if line.chars().any(char::is_lowercase) {
        return line.to_owned();
    }
    // ascii-guard: data begin - words matched in page text
    const SMALL: &[&str] = &[
        "of", "at", "the", "and", "for", "in", "de", "di", "du", "des", "la", "le", "für", "und",
        "van", "der", "del", "degli",
    ];
    // ascii-guard: data end
    line.split(' ')
        .enumerate()
        .map(|(index, word)| {
            let lower = word.to_lowercase();
            if index > 0 && SMALL.contains(&lower.as_str()) {
                lower
            } else if word.chars().count() <= 3 {
                // An acronym: MIT, UC, ETH.
                word.to_owned()
            } else {
                let mut chars = lower.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The reason to call the pages a thesis, or none.
fn thesis_reason(text: &str, s: &Signals) -> Option<String> {
    let folded = s.folded;
    let word = has_word(folded, "thesis")
        || has_word(folded, "theses")
        || has_word(folded, "dissertation");
    let submission = has_any(folded, THESIS_SUBMISSION_MARKS);
    let degree = word && has_any(folded, DEGREE_NAMES);
    let named = has_any(folded, THESIS_DEGREE_MARKS);
    let people = word && has_any(folded, THESIS_PEOPLE_MARKS);
    let university = word && university_near_word(folded);

    // The submission formula, or a degree named beside the word, is the
    // title page's own voice - unless a journal's page says it in an
    // author's note ("based on a dissertation submitted to...").
    let strong = (submission || degree) && !(s.publisher.is_some() || (s.doi && s.journal_like()));
    // "PhD thesis" and a supervisor's name are a thesis's words too, but a
    // paper cites theses and thanks supervisors; a paper's marks win.
    let medium = (named || people) && !s.paper_marks();
    // The university beside the word, when nothing says this is a document
    // about theses (a syllabus, a template) rather than one.
    let weak = university && !s.paper_marks() && !has_any(folded, THESIS_TALK_MARKS);
    if !(strong || medium || weak) {
        return None;
    }
    let noun = match (folded.find("dissertation"), folded.find("thesis")) {
        (Some(d), Some(t)) if d < t => "dissertation",
        (Some(_), None) => "dissertation",
        _ => "thesis",
    };
    if submission {
        if let Some(university) = university_line(text) {
            return Some(format!("a {noun} submitted to {university}"));
        }
    }
    Some(format!("a {noun} title page"))
}

/// The reason to call the pages a book, or none.
fn book_reason(s: &Signals, isbn: Option<&str>) -> Option<String> {
    let folded = s.folded;
    let issn = has_word(folded, "issn");
    // The ISBN is the book's own number. A conference paper prints its
    // proceedings' ISBN in the footer, beside the venue, so the venue's
    // words say otherwise.
    if let Some(isbn) = isbn {
        if !(s.arxiv || s.received || s.proceedings || issn) {
            let place = if has_any(folded, &["copyright", "\u{a9}", "all rights reserved"]) {
                "the copyright page"
            } else {
                "the first page"
            };
            return Some(format!("ISBN {isbn} on {place}"));
        }
    }
    if has_any(folded, CATALOGUE_MARKS) && !s.arxiv {
        return Some("a book's copyright page".to_owned());
    }
    // The rest is weaker, and says nothing against a paper's stamps or a
    // manual's words.
    if s.arxiv || s.doi || s.journal_like() || has_any(folded, MANUAL_MARKS) {
        return None;
    }
    let press = BOOK_PRESS_MARKS
        .iter()
        .find(|(mark, _)| folded.contains(mark))
        .map(|(_, name)| *name);
    if has_word(folded, "edition") {
        if let Some(name) = s.publisher.map(|(_, name, _)| *name).or(press) {
            return Some(format!("an edition from {name}"));
        }
    }
    if folded.contains("all rights reserved") && folded.contains("printed in") {
        if let Some(name) = press {
            return Some(format!("a copyright page from {name}"));
        }
    }
    if has_word(folded, "contents") && s.publisher.is_none() {
        if folded.contains("chapter 1") || folded.contains("chapter one") {
            return Some("a table of contents with chapters".to_owned());
        }
        if has_word(folded, "preface") {
            return Some("a table of contents and a preface".to_owned());
        }
    }
    None
}

/// Whether a page says too little on its own: fewer than
/// `SPARSE_PAGE_CHARS` characters that are not blank.
pub fn is_sparse(page: &str) -> bool {
    page.chars().filter(|c| !c.is_whitespace()).count() < SPARSE_PAGE_CHARS
}

/// Whether the first page alone settles nothing, so the pages after it are
/// worth reading: a thesis puts its submission formula on the title page
/// behind the cover, a book its ISBN on the copyright page behind the
/// half-title. A page that says little, or says nothing the classifier
/// recognizes, asks for more.
pub fn needs_more_pages(first_page: &str, info: &InfoFields) -> bool {
    is_sparse(first_page) || classify(first_page, info).verdict == PapernessVerdict::Unknown
}

/// What the text - the first page, or the first few pages joined - and the
/// Info dictionary say.
pub fn classify(text: &str, info: &InfoFields) -> PapernessDto {
    let folded = fold(text);
    // The identifiers the pages carry as their own are read above the
    // reference list, like the scan reads them; the words of the whole
    // text still count for everything else.
    let head = page_head(text);
    let (doi, arxiv) = own_identifiers(head, text);
    let isbn = find_isbn(head);

    let verdict =
        |verdict: PapernessVerdict, reason: String, kind: Option<PaperKind>| PapernessDto {
            verdict,
            reason,
            kind,
            doi: doi.clone(),
            arxiv: arxiv.clone(),
            isbn: isbn.clone(),
        };

    let signals = Signals {
        folded: &folded,
        arxiv: arxiv.is_some(),
        doi: doi.is_some(),
        publisher: PUBLISHER_MARKS
            .iter()
            .find(|(mark, _, _)| folded.contains(mark)),
        received: has_word(&folded, "received") && has_word(&folded, "accepted"),
        proceedings: is_proceedings_like(&folded),
    };

    // A thesis or a book names itself on its own title or copyright page;
    // what follows are the signals of a paper, which a thesis may carry too
    // (a DOI from the repository, an arXiv stamp) without being one.
    if let Some(reason) = thesis_reason(text, &signals) {
        return verdict(PapernessVerdict::Paper, reason, Some(PaperKind::Thesis));
    }
    if let Some(reason) = book_reason(&signals, isbn.as_deref()) {
        return verdict(PapernessVerdict::Paper, reason, Some(PaperKind::Book));
    }

    // An arXiv stamp is the clearest signal there is: it is printed on the
    // first page of every arXiv paper and on nothing else. Only the stamp:
    // a bare arXiv id is a citation, and a page that cites preprints is
    // judged on its other signals.
    if let Some(id) = &arxiv {
        return verdict(
            PapernessVerdict::Paper,
            format!("arXiv:{id} on the first page"),
            None,
        );
    }
    if let Some((_, name, kind)) = signals.publisher {
        return verdict(
            PapernessVerdict::Paper,
            format!("{name} on the first page"),
            kind.kind(&folded),
        );
    }
    if let Some(id) = &doi {
        return verdict(
            PapernessVerdict::Paper,
            format!("DOI {id} on the first page"),
            venue_kind(&folded).or(Some(PaperKind::Article)),
        );
    }
    // An Elsevier PII is what their scans of older articles print instead of
    // a DOI; the scan turns it into one. Checked by its ISSN digit so a run
    // of numbers from a form does not pass as a paper.
    if let Some(pii) = find_pii(head).filter(|pii| pii_has_valid_issn(pii)) {
        return verdict(
            PapernessVerdict::Paper,
            format!("Elsevier PII {pii} on the first page"),
            Some(PaperKind::Article),
        );
    }
    let info_text = fold(
        &[info.producer, info.creator, info.subject]
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
    );
    for (mark, name, kind) in PRODUCER_MARKS {
        if info_text.contains(mark) {
            return verdict(
                PapernessVerdict::Paper,
                format!("made by {name}"),
                kind.kind(&folded),
            );
        }
    }

    // Structure: the shape of a paper without anyone's name on it.
    let abstract_ = has_word(&folded, "abstract");
    let references = has_word(&folded, "references") || has_word(&folded, "bibliography");
    let introduction = has_word(&folded, "introduction");
    let keywords = has_word(&folded, "keywords") || has_word(&folded, "key words");
    let et_al = folded.contains("et al");
    if signals.received {
        return verdict(
            PapernessVerdict::Likely,
            "has a journal's received and accepted dates".to_owned(),
            None,
        );
    }
    if abstract_ && (introduction || keywords || references) {
        return verdict(
            PapernessVerdict::Likely,
            "has an abstract and the sections of a paper".to_owned(),
            None,
        );
    }
    if abstract_ && et_al {
        return verdict(
            PapernessVerdict::Likely,
            "has an abstract and cites other work".to_owned(),
            None,
        );
    }
    let latex = info_text.contains("latex") || info_text.contains("pdftex");
    if latex && (introduction || references) {
        return verdict(
            PapernessVerdict::Likely,
            "typeset with LaTeX and sectioned like a paper".to_owned(),
            None,
        );
    }
    verdict(PapernessVerdict::Unknown, String::new(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_text(text: &str) -> PapernessDto {
        classify(text, &InfoFields::default())
    }

    #[test]
    fn an_arxiv_stamp_settles_it() {
        let page = "Attention Is All You Need\nAshish Vaswani\narXiv:1706.03762v5 [cs.CL] 6 Dec 2017\nAbstract\nThe dominant sequence transduction models";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.arxiv.as_deref(), Some("1706.03762"));
        assert!(got.reason.contains("arXiv:1706.03762"), "{}", got.reason);
        assert_eq!(got.kind, None, "an arXiv stamp says paper, not which kind");
        assert_eq!(got.isbn, None);
    }

    /// A bare arXiv id is a citation, wherever it stands, and so is a DOI
    /// below the reference-list heading: neither is the document's own,
    /// and neither makes it a paper. The page is judged on what is left.
    #[test]
    fn a_cited_identifier_is_not_the_documents_own() {
        let references = "A Short Note On Things\nJ. Doe\nAbstract\nWe build on prior work.\n1 Introduction\nReferences\n[1] A. Vaswani et al. arXiv:1706.03762, 2017.\n[2] C. Jarzynski. doi:10.1103/PhysRevLett.78.2690";
        let got = classify_text(references);
        assert_eq!(got.verdict, PapernessVerdict::Likely, "{}", got.reason);
        assert_eq!(got.reason, "has an abstract and the sections of a paper");
        assert_eq!(got.arxiv, None);
        assert_eq!(got.doi, None);

        // A bare id in the head is a citation too; a lone one leaves the
        // page unknown.
        let got = classify_text("A Short Note On Things\nWe extend arXiv:1706.03762 to graphs.");
        assert_eq!(got.verdict, PapernessVerdict::Unknown);
        assert_eq!(got.arxiv, None);

        // The stamp settles it even when PDFium reads it out last, below
        // the references; a DOI in the head is the document's.
        let stamped = format!("{references}\narXiv:2401.00001v1 [cs.LG] 2 Jan 2024");
        let got = classify_text(&stamped);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.arxiv.as_deref(), Some("2401.00001"));
        assert_eq!(got.reason, "arXiv:2401.00001 on the first page");
        let got = classify_text(
            "A Short Note On Things\ndoi:10.5555/speedyf.001\nReferences\n[1] doi:10.1000/cited",
        );
        assert_eq!(got.doi.as_deref(), Some("10.5555/speedyf.001"));
        assert!(
            got.reason.starts_with("DOI 10.5555/speedyf.001"),
            "{}",
            got.reason
        );
        // An ISBN or a PII below the heading is somebody else's as well.
        let got = classify_text("A Short Note On Things\nReferences\n[1] A book. ISBN 0-306-40615-2.\n[2] PII: S0370-2693(97)00412-1");
        assert_eq!(got.verdict, PapernessVerdict::Unknown, "{}", got.reason);
        assert_eq!(got.isbn, None);
    }

    #[test]
    fn publishers_are_recognized_by_name() {
        for (page, name, kind) in [
            (
                "Contents lists available at ScienceDirect\nPhysica A\njournal homepage",
                "ScienceDirect",
                Some(PaperKind::Article),
            ),
            (
                "Journal of Machine Learning Research 12 (2011) 2825-2830\nSubmitted 3/11; Published 10/11",
                "JMLR",
                Some(PaperKind::Article),
            ),
            (
                "PHYSICAL REVIEW LETTERS 78, 2690 (1997)\nNonequilibrium Equality",
                "an APS journal",
                Some(PaperKind::Article),
            ),
            (
                "Full terms and conditions of use: https://www.tandfonline.com/action/journalInformation",
                "Taylor & Francis",
                Some(PaperKind::Article),
            ),
            (
                "Advances in Neural Information Processing Systems 30 (NIPS 2017)",
                "NeurIPS",
                Some(PaperKind::Inproceedings),
            ),
            (
                "Proceedings of the 36th International Conference on Machine Learning, Long Beach",
                "ICML",
                Some(PaperKind::Inproceedings),
            ),
            (
                "Published as a conference paper at ICLR 2021\nInternational Conference on Learning Representations",
                "ICLR",
                Some(PaperKind::Inproceedings),
            ),
            // A publisher of both: the venue line decides.
            (
                "IEEE TRANSACTIONS ON INFORMATION THEORY, VOL. 52, NO. 4, APRIL 2006",
                "IEEE",
                Some(PaperKind::Article),
            ),
            (
                "In Proceedings of the 2019 CHI Conference on Human Factors\nAssociation for Computing Machinery",
                "ACM",
                Some(PaperKind::Inproceedings),
            ),
            (
                "© 2010 IEEE. Personal use of this material is permitted.",
                "IEEE",
                None,
            ),
            // A preprint server is a paper of no particular kind.
            ("Preprint posted on bioRxiv", "bioRxiv", None),
            (
                "Proceedings of the National Academy of Sciences\nvol. 117 no. 48",
                "PNAS",
                Some(PaperKind::Article),
            ),
            (
                "Proceedings of the 14th Symposium on Discrete Algorithms (SODA)",
                "a conference's proceedings",
                Some(PaperKind::Inproceedings),
            ),
        ] {
            let got = classify_text(page);
            assert_eq!(got.verdict, PapernessVerdict::Paper, "{page}");
            assert!(got.reason.starts_with(name), "{} for {page}", got.reason);
            assert_eq!(got.kind, kind, "{page}");
        }
    }

    #[test]
    fn an_elsevier_pii_is_a_paper() {
        let got =
            classify_text("PII: 0022-0000(88)90028-1\nJournal of Computer and System Sciences");
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Article));
        assert!(
            got.reason.contains("0022-0000(88)90028-1"),
            "{}",
            got.reason
        );
        // A PII whose ISSN fails its check digit is just digits.
        let got = classify_text("PII: 0022-0001(88)90028-1\nSome scanned form");
        assert_eq!(got.verdict, PapernessVerdict::Unknown);
    }

    #[test]
    fn a_doi_alone_is_enough() {
        let got =
            classify_text("Some title\nhttps://doi.org/10.1103/PhysRevLett.78.2690\nAbstract");
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(got.kind, Some(PaperKind::Article));
        let got = classify_text(
            "Some title\nhttps://doi.org/10.1145/3290605.3300233\nIn Proceedings of CHI 2019",
        );
        assert_eq!(got.kind, Some(PaperKind::Inproceedings));
    }

    #[test]
    fn structure_is_only_likely() {
        let got = classify_text(
            "A Theory of Things\nJ. Doe\nAbstract\nWe show things.\n1 Introduction\nThings are known [1].",
        );
        assert_eq!(got.verdict, PapernessVerdict::Likely);
        assert_eq!(got.kind, None);
        let got = classify_text("Manuscript\nReceived 3 May 2019; Accepted 1 June 2019");
        assert_eq!(got.verdict, PapernessVerdict::Likely);
    }

    #[test]
    fn manuals_receipts_and_notes_stay_unknown() {
        for page in [
            "User Manual\nModel X200\nSafety information\nRead all instructions before use.",
            "RECEIPT\nOrder #4521\nTotal $42.00\nThank you for your purchase",
            "Lecture 7: Eigenvalues\nRecall from last time that a matrix A",
            "INVOICE\nBill to: Someone\nAmount due: 100.00\nDue on receipt",
            // "abstract" alone, as a word in prose, is not a heading.
            "This abstract painting was acquired in 1998.",
            // Chapters and a copyright line, but no ISBN and no press: a
            // manual, not a book.
            "X200 Series\nChapter 1 Getting started\nChapter 2 Charging the battery\n© 2020 Acme Corp. All rights reserved. Printed in China",
            // With a table of contents too, the manual's own words keep it
            // out.
            "X200 User Manual\nContents\nChapter 1 Getting started\nChapter 2 Safety\nTroubleshooting\n© 2020 Acme Corp. All rights reserved. Printed in China",
            // Slides.
            "Deep Learning for Vision\nJ. Doe, Stanford University\nOutline\nMotivation\nMethod\nResults",
        ] {
            let got = classify_text(page);
            assert_eq!(got.verdict, PapernessVerdict::Unknown, "{page}");
            assert!(got.reason.is_empty());
            assert_eq!(got.kind, None);
        }
    }

    #[test]
    fn the_producer_field_counts_when_the_page_says_nothing() {
        let info = InfoFields {
            producer: Some("Elsevier"),
            ..InfoFields::default()
        };
        let got = classify("Some scanned text with no markers", &info);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.reason, "made by Elsevier");
        assert_eq!(got.kind, Some(PaperKind::Article));
    }

    #[test]
    fn short_marks_do_not_fire_inside_other_words() {
        assert_eq!(
            classify_text("Macmillan Dictionary, second edition").verdict,
            PapernessVerdict::Unknown
        );
        assert!(!has_word("the abstractions of", "abstract"));
        assert!(has_word("abstract. we show", "abstract"));
    }

    #[test]
    fn a_dissertation_title_page_is_a_thesis() {
        let page = "LEARNING TO FLY\nA DISSERTATION\nSUBMITTED TO THE DEPARTMENT OF PHYSICS\nAND THE COMMITTEE ON GRADUATE STUDIES\nOF STANFORD UNIVERSITY\nIN PARTIAL FULFILLMENT OF THE REQUIREMENTS\nFOR THE DEGREE OF\nDOCTOR OF PHILOSOPHY\nJane Doe\nJune 2019";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(
            got.reason,
            "a dissertation submitted to Stanford University"
        );

        // The university on a line of its own is named in the reason.
        let page = "Learning to Fly\nby\nJane Doe\nA dissertation submitted in partial satisfaction of the requirements for the degree of Doctor of Philosophy in Physics\nUNIVERSITY OF CALIFORNIA, BERKELEY\nCommittee in charge: Professor A. Smith, Chair";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(
            got.reason,
            "a dissertation submitted to University of California, Berkeley"
        );

        let page = "Learning to Fly\nPh.D. Thesis\nJane Doe\nMassachusetts Institute of Technology\nThesis supervisor: John Roe";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(got.reason, "a thesis title page");

        // German: the word beside the university's name.
        let page = "Dissertation\nzur Erlangung des Doktorgrades\nder Fakultät für Physik\nder Ludwig-Maximilians-Universität München\nvorgelegt von Jane Doe";
        assert_eq!(classify_text(page).kind, Some(PaperKind::Thesis));
    }

    #[test]
    fn a_masters_thesis_is_a_thesis() {
        let page = "Fast Sorting on GPUs\nMaster's Thesis\nJohn Doe\nDepartment of Computer Science\nUniversity of Nowhere\nSupervisor: Prof. A. Smith\n2020";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(got.reason, "a thesis title page");

        let page = "Fast Sorting on GPUs\nA thesis submitted to the University of Nowhere for the degree of Master of Science\nJohn Doe";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(
            got.reason, "a thesis title page",
            "no line is just the name"
        );
    }

    #[test]
    fn mentioning_a_thesis_is_not_being_one() {
        // A syllabus.
        let page = "PHYS 101 Course Syllabus\nStanford University\nFall 2024\nGrading: weekly problem sets and a senior thesis option for honors students.\nDeadline for the thesis proposal: October 1.";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Unknown, "{}", got.reason);
        assert_eq!(got.kind, None);
        // A template.
        let page = "Thesis Template\nUniversity of Nowhere\nUse this template for your thesis. Replace this text with your title.";
        assert_eq!(classify_text(page).verdict, PapernessVerdict::Unknown);
        // A paper whose author's note says where it came from: the
        // journal's page wins.
        let page = "Journal of Personality and Social Psychology 2010, Vol. 98, No. 3, 400-412\nhttps://doi.org/10.1037/a0018000\nThis article is based on a dissertation submitted to the University of Nowhere in partial fulfillment of the requirements for the Ph.D.\nAbstract";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Article));
        // A paper that cites a thesis.
        let page = "PHYSICAL REVIEW E 99, 012345 (2019)\nAs shown in J. Doe's PhD thesis [3], the supervisor: field is irrelevant.";
        assert_eq!(classify_text(page).kind, Some(PaperKind::Article));
    }

    #[test]
    fn a_copyright_page_with_an_isbn_is_a_book() {
        let page = "Cambridge University Press\nThe Edinburgh Building, Cambridge CB2 8RU, UK\n© Cambridge University Press 2004\nFirst published 2004\nPrinted in the United Kingdom\nA catalogue record for this book is available from the British Library\nISBN 978-0-521-64298-9 hardback";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.isbn.as_deref(), Some("9780521642989"));
        assert_eq!(got.reason, "ISBN 9780521642989 on the copyright page");

        // The ISBN is found and reported even behind a Springer DOI.
        let page = "Springer-Verlag Berlin Heidelberg\nISBN 978-3-540-69254-6\nDOI 10.1007/978-3-540-69254-6\nLibrary of Congress Control Number: 2008923456\n© 2008 Springer";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.isbn.as_deref(), Some("9783540692546"));
        assert!(
            got.reason.starts_with("ISBN 9783540692546"),
            "{}",
            got.reason
        );
    }

    #[test]
    fn a_proceedings_isbn_in_a_papers_footer_is_not_a_book() {
        let page = "Permission to make digital or hard copies\nCHI 2019, May 4–9, 2019, Glasgow, Scotland UK\n© 2019 Association for Computing Machinery.\nACM ISBN 978-1-4503-5970-2/19/05\nhttps://doi.org/10.1145/3290605.3300233\nIn Proceedings of the 2019 CHI Conference";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Inproceedings), "{}", got.reason);
        assert_eq!(
            got.isbn.as_deref(),
            Some("9781450359702"),
            "still reported, for the record"
        );
    }

    #[test]
    fn a_copyright_page_without_an_isbn_is_still_a_book() {
        // An older printing: a press, an edition and the cataloguing line.
        let page = "Classical Mechanics\nSecond Edition\nHerbert Goldstein\nAddison-Wesley Publishing Company\nCopyright © 1980 by Addison-Wesley\nAll rights reserved.\nPrinted in the United States of America.\nLibrary of Congress Cataloging in Publication Data";
        let got = classify_text(page);
        assert_eq!(got.verdict, PapernessVerdict::Paper);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.reason, "a book's copyright page");
        assert_eq!(got.isbn, None);

        // Edition and press, no cataloguing line.
        let page = "Numerical Recipes\nThird Edition\nCambridge University Press\nCopyright © 2007";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.reason, "an edition from Cambridge University Press");

        // Rights line, printing line and a press, no edition.
        let page = "Copyright © 1965 by the Massachusetts Institute of Technology\nAll rights reserved. No part of this book may be reproduced\nPrinted in the United States of America\nThe MIT Press, Cambridge, Massachusetts";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.reason, "a copyright page from MIT Press");

        // A table of contents with chapters.
        let page = "Contents\nPreface ix\nChapter 1 Introduction 1\nChapter 2 Linear Maps 25\nChapter 3 Eigenvalues 70";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.reason, "a table of contents with chapters");
    }

    #[test]
    fn a_journal_page_with_a_publishers_edition_or_rights_line_stays_a_paper() {
        // "All rights reserved ... Printed in" on an old journal page, from a
        // journal publisher: not a book.
        let page = "J. Chem. Phys. 81, 511 (1984)\nReceived 12 January 1984; accepted 2 March 1984\n© 1984 American Institute of Physics. All rights reserved. Printed in U.S.A.\nSpecial edition";
        let got = classify_text(page);
        assert_eq!(got.kind, Some(PaperKind::Article), "{}", got.reason);
    }

    #[test]
    fn a_sparse_cover_asks_for_the_next_pages() {
        let info = InfoFields::default();
        let cover = "Learning to Fly\nJane Doe\n2019";
        assert!(is_sparse(cover));
        assert!(needs_more_pages(cover, &info));
        // A full page the classifier cannot place asks too.
        let dense_unknown = "The quick brown fox jumps over the lazy dog. ".repeat(12);
        assert!(!is_sparse(&dense_unknown));
        assert!(needs_more_pages(&dense_unknown, &info));
        // A page that already answers does not.
        let paper = format!("{dense_unknown}\narXiv:1706.03762v5 [cs.CL] 6 Dec 2017");
        assert!(!needs_more_pages(&paper, &info));
        // A sparse page that answers still asks: the rest may say more
        // (the kind, an ISBN), and it costs little.
        assert!(needs_more_pages("A Title\narXiv:1706.03762v5", &info));

        // The decision on the joined pages: the cover says nothing, the
        // title page behind it settles it.
        let title_page = "A dissertation submitted in partial fulfillment of the requirements for the degree of Doctor of Philosophy\nStanford University";
        let joined = format!("{cover}\n{title_page}");
        let got = classify(&joined, &info);
        assert_eq!(got.kind, Some(PaperKind::Thesis));
        assert_eq!(
            got.reason,
            "a dissertation submitted to Stanford University"
        );
        let copyright_page = "© 2004 Cambridge University Press\nISBN 0-521-64298-1";
        let got = classify(&format!("{cover}\n{copyright_page}"), &info);
        assert_eq!(got.kind, Some(PaperKind::Book));
        assert_eq!(got.isbn.as_deref(), Some("0521642981"));
    }

    #[test]
    fn shouting_names_are_calmed() {
        assert_eq!(
            title_case_if_shouting("UNIVERSITY OF CALIFORNIA, BERKELEY"),
            "University of California, Berkeley"
        );
        assert_eq!(
            title_case_if_shouting("Stanford University"),
            "Stanford University"
        );
        assert_eq!(title_case_if_shouting("MIT"), "MIT");
    }
}
