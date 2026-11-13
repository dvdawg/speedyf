//! Heading text as the outline shows it: ligatures and footnote markers
//! gone, a letter-spaced or small-caps initial put back on its word, a
//! hyphenated break rejoined, and shouting capitals ("INTRODUCTION")
//! brought down to title case - acronyms ("DFT") and numerals left alone.

use crate::engine::layout_text::is_superscript_char;
use unicode_normalization::UnicodeNormalization;

/// Section names English papers use without a number. Matched against the
/// whole title, letters only, so "4. Conclusions" and "CONCLUSIONS" both
/// count once their numbering is set aside.
const KNOWN_NAMES: &[&str] = &[
    "abstract",
    "introduction",
    "results",
    "discussion",
    "resultsanddiscussion",
    "methods",
    "materialsandmethods",
    "methodsandmaterials",
    "methodology",
    "conclusion",
    "conclusions",
    "concludingremarks",
    "conclusionsandoutlook",
    "summaryandoutlook",
    "acknowledgments",
    "acknowledgements",
    "acknowledgment",
    "acknowledgement",
    "references",
    "bibliography",
    "appendix",
    "appendices",
    "supplementaryinformation",
    "supplementarymaterial",
    "supplementarymaterials",
    "dataavailability",
    "dataavailabilitystatement",
    "codeavailability",
    "authorcontributions",
    "competinginterests",
    "relatedwork",
    "background",
    "preliminaries",
    "experiments",
    "limitations",
    "futurework",
    "preface",
    "foreword",
];

/// Known names that are a document's top level wherever they are set:
/// the matter around the sections rather than a section of the argument.
const TOP_LEVEL_NAMES: &[&str] = &[
    "abstract",
    "acknowledgments",
    "acknowledgements",
    "acknowledgment",
    "acknowledgement",
    "references",
    "bibliography",
    "appendix",
    "appendices",
    "supplementaryinformation",
    "supplementarymaterial",
    "supplementarymaterials",
    "dataavailability",
    "dataavailabilitystatement",
    "codeavailability",
    "authorcontributions",
    "competinginterests",
    "preface",
    "foreword",
];

/// Whether a known name belongs at the top level regardless of its style.
pub fn is_top_level_name(title: &str) -> bool {
    TOP_LEVEL_NAMES.contains(&letters(title).as_str())
}

/// Letters of `text`, lower-cased, nothing else.
pub fn letters(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `title` (numbering already removed) is one of the section names
/// papers use without numbering.
pub fn is_known_name(title: &str) -> bool {
    let key = letters(title);
    KNOWN_NAMES.contains(&key.as_str())
}

/// Footnote markers and ligatures out, spaces collapsed, and an initial
/// letter-spaced off its word ("I NTRODUCTION") put back.
pub fn clean(text: &str) -> String {
    let stripped: String = text.chars().filter(|c| !is_superscript_char(*c)).collect();
    let folded: String = stripped
        .nfkc()
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' => '-',
            '\u{00a0}' => ' ',
            c => c,
        })
        .collect();
    // "7 .1.1" -> "7.1.1": a space extraction put inside a number.
    let mut compact = String::with_capacity(folded.len());
    let chars: Vec<char> = folded.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let spaced_dot = *c == ' '
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1) == Some(&'.')
            && chars.get(i + 2).is_some_and(char::is_ascii_digit);
        if !spaced_dot {
            compact.push(*c);
        }
    }
    let words: Vec<&str> = compact.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        if let Some(next) = words.get(i + 1) {
            if rejoin_initial(word, next) {
                out.push(format!("{word}{next}"));
                i += 2;
                continue;
            }
        }
        out.push(word.to_owned());
        i += 1;
    }
    out.join(" ")
        .trim_end_matches(['*', '\u{2020}', '\u{2021}'])
        .trim()
        .to_owned()
}

/// A single capital split off the word it begins - small caps set with a
/// larger initial often come out of extraction that way. English has three
/// one-letter words, so those rejoin only into a known section name.
fn rejoin_initial(word: &str, next: &str) -> bool {
    let mut chars = word.chars();
    let (Some(initial), None) = (chars.next(), chars.next()) else {
        return false;
    };
    if !initial.is_uppercase() || next.chars().count() < 2 {
        return false;
    }
    if !next.chars().all(char::is_alphabetic) {
        return false;
    }
    let all_caps = next.chars().all(char::is_uppercase);
    let all_lower = next.chars().all(char::is_lowercase);
    if !(all_caps || all_lower) {
        return false;
    }
    if matches!(initial, 'A' | 'I' | 'O') {
        return is_known_name(&format!("{word}{next}"));
    }
    true
}

/// Join a heading's wrapped lines, undoing a hyphenated break.
pub fn join_lines(first: &str, second: &str) -> String {
    let first = first.trim_end();
    if let Some(stem) = first.strip_suffix('-') {
        if second.starts_with(|c: char| c.is_lowercase()) {
            return format!("{stem}{second}");
        }
    }
    format!("{first} {second}")
}

/// Whether a title is set in capitals: three or more letters, none lower
/// case.
pub fn is_all_caps(text: &str) -> bool {
    let mut upper = 0;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        if c.is_lowercase() {
            return false;
        }
        if c.is_uppercase() {
            upper += 1;
        }
    }
    upper >= 3
}

const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "by", "for", "from", "in", "into", "of", "on", "or", "the", "to",
    "via", "vs", "with", "versus", "per",
];

/// Common acronyms that contain a vowel, which the no-vowel rule below
/// would otherwise title-case ("Dna").
const ACRONYMS: &[&str] = &[
    "AI", "API", "DNA", "RNA", "QED", "CPU", "GPU", "TPU", "UAV", "EU", "US", "UK", "USA", "UN",
    "ODE", "PDE", "SDE", "IO", "IOT", "UI", "OOD", "EEG", "ECG", "MEG", "PET", "AGN", "ISM", "IGM",
    "UV", "IR", "EOS", "NASA", "ESA", "LIGO", "MRI", "FMRI", "PCA", "ICA", "RL", "LLM", "LLMS",
    "NLP", "CNN", "RNN", "GAN", "VAE", "SVM", "MCMC", "SGD", "ADMM", "BEC", "QCD", "LHC", "ATLAS",
    "CMS", "ALICE", "LHCB", "HEP", "IOU", "API", "SQL", "HIV", "AIDS", "COVID", "SARS", "PDF",
    "XML", "HTML", "OCR", "IEEE", "ACM", "AMS", "APS", "NSF", "NIH", "ERC", "DOE",
];

/// Whether an all-caps word stays in capitals: an acronym, a numeral, a
/// single letter (an appendix's "A"), anything with a digit.
fn keeps_capitals(word: &str) -> bool {
    let core: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
    if core.is_empty() {
        return true;
    }
    if core.chars().count() == 1 || core.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if crate::engine::outline_gen::numbering::roman_value(&core).is_some() {
        return true;
    }
    if word.starts_with('(') && word.trim_end_matches([',', '.', ':']).ends_with(')') {
        return true;
    }
    if ACRONYMS.contains(&core.as_str()) {
        return true;
    }
    // A word with no vowel is not an English word: "DFT", "NMR", "CMB".
    !core.chars().any(|c| "AEIOUY".contains(c))
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// Title case for a heading set in capitals; anything else is returned as
/// it was printed.
pub fn display_case(text: &str) -> String {
    if !is_all_caps(text) {
        return text.to_owned();
    }
    let mut out = Vec::new();
    let mut first_word = true;
    for word in text.split_whitespace() {
        if keeps_capitals(word) {
            out.push(word.to_owned());
            if word.chars().any(char::is_alphabetic) {
                first_word = word.ends_with(':');
            }
            continue;
        }
        let parts: Vec<String> = word
            .split('-')
            .enumerate()
            .map(|(i, part)| {
                let lower = part.to_lowercase();
                let bare: String = lower.chars().filter(|c| c.is_alphabetic()).collect();
                if !(first_word && i == 0) && SMALL_WORDS.contains(&bare.as_str()) {
                    lower
                } else if keeps_capitals(part) && part.chars().count() > 1 {
                    part.to_owned()
                } else {
                    capitalize(part)
                }
            })
            .collect();
        out.push(parts.join("-"));
        // After a colon a new title starts: "Appendix A: The Model".
        first_word = word.ends_with(':');
    }
    out.join(" ")
}

/// The finished title: cleaned, trailing punctuation off, case fixed.
pub fn display(text: &str) -> String {
    let cleaned = clean(text);
    let trimmed = cleaned
        .trim_end_matches(['.', ':', ';', ',', '\u{2014}', '\u{2013}', '-', ' '])
        .trim();
    display_case(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_caps_headings_come_down_to_title_case() {
        assert_eq!(display("INTRODUCTION"), "Introduction");
        assert_eq!(display("I. INTRODUCTION"), "I. Introduction");
        assert_eq!(
            display("II. DENSITY FUNCTIONAL THEORY (DFT) CALCULATIONS"),
            "II. Density Functional Theory (DFT) Calculations"
        );
        assert_eq!(display("RESULTS AND DISCUSSION"), "Results and Discussion");
        assert_eq!(
            display("APPENDIX A: THE DNA MODEL"),
            "Appendix A: The DNA Model"
        );
        assert_eq!(display("THE NMR SIGNAL"), "The NMR Signal");
        assert_eq!(
            display("STATE-OF-THE-ART METHODS"),
            "State-of-the-Art Methods"
        );
    }

    #[test]
    fn mixed_case_is_kept_as_printed() {
        assert_eq!(
            display("2.1 Results for DFT and the GW approximation"),
            "2.1 Results for DFT and the GW approximation"
        );
        assert_eq!(display("Model."), "Model");
    }

    #[test]
    fn ligatures_markers_and_split_initials_are_repaired() {
        assert_eq!(clean("Signi\u{fb01}cance tests"), "Significance tests");
        assert_eq!(clean("Introduction\u{b9}"), "Introduction");
        assert_eq!(clean("Methods*"), "Methods");
        assert_eq!(clean("I NTRODUCTION"), "INTRODUCTION");
        assert_eq!(clean("R ESULTS"), "RESULTS");
        assert_eq!(clean("A MODEL"), "A MODEL");
        assert_eq!(clean("A new model"), "A new model");
    }

    #[test]
    fn hyphenated_wraps_rejoin() {
        assert_eq!(
            join_lines("Non-equilibrium dyna-", "mics of spins"),
            "Non-equilibrium dynamics of spins"
        );
        assert_eq!(
            join_lines("Results for", "the model"),
            "Results for the model"
        );
        assert_eq!(
            join_lines("Self-", "Consistent Fields"),
            "Self- Consistent Fields"
        );
    }

    #[test]
    fn known_names_ignore_case_and_spacing() {
        assert!(is_known_name("ACKNOWLEDGMENTS"));
        assert!(is_known_name("Materials and Methods"));
        assert!(is_known_name("Data Availability"));
        assert!(!is_known_name("Introduction to groups"));
    }
}
