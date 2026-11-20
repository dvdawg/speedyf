//! The year a paper's first page prints for itself.
//!
//! A journal's masthead and an APS received line state the year of record
//! outright: "PHYSICAL REVIEW C 80, 024601 (2009)", "(Received 8 June 2009;
//! published 5 August 2009)". A copyright line, a conference's name or a
//! lone date under a title usually agree with it, and are read when the
//! page has nothing better. "Published online" is left alone: on a back
//! issue it is the day the archive was digitized, decades after the paper.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// A year off the page, and whether the page stated it as the year of
/// record (`strong`) or only implied it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintedYear {
    pub year: i32,
    pub strong: bool,
}

/// How many lines down a masthead may sit: a cover banner or an access
/// notice can come before it.
const MASTHEAD_LINES: usize = 6;

fn masthead() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A venue's name, a volume (or the "?" of a preprint's template), an
    // article number or nothing, then the year in parentheses.
    RE.get_or_init(|| {
        Regex::new(
            r"^[A-Za-z][A-Za-z .,&:'-]{2,}\s(?:\d+|\?)[ ,]*(?:\d+\s*)?\(((?:1[89]|20)\d\d)\)",
        )
        .expect("valid regex")
    })
}

fn whole_masthead() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A masthead the text layer put below the abstract: the whole line and
    // nothing else, page range included, so a reference in a footnote
    // ("Phys. Rev. C 80, 024601 (2009), and refs. therein") is not taken.
    RE.get_or_init(|| {
        Regex::new(
            r"^[A-Za-z][A-Za-z .,&:'-]{2,}\s\d+[ ,]*(?:\d+\s*)?\(((?:1[89]|20)\d\d)\)\s*(?:\d+\s*[\x{2013}-]\s*\d+)?$",
        )
        .expect("valid regex")
    })
}

fn volume_banner() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "PHYSICAL REVIEW C VOLUME 41, NUMBER 5 MAY 1990".
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bvolume\s+\d+\b.*\b((?:1[89]|20)\d\d)$").expect("valid regex")
    })
}

fn published() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[;(]\s*published\s+\d{1,2}\s+[a-z]+\.?\s+((?:1[89]|20)\d\d)")
            .expect("valid regex")
    })
}

fn cite_this() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Taylor & Francis: "To cite this article: A. Author (1995) Title...".
    RE.get_or_init(|| {
        Regex::new(r"(?is)to cite this article:.{0,200}?\(((?:1[89]|20)\d\d)\)")
            .expect("valid regex")
    })
}

fn copyright() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "(c)" and a lone "c" at the start of a line are a © the text layer
    // lost; the holder may come before the year ("c MIT 2011").
    RE.get_or_init(|| {
        Regex::new(r"(?im)(?:\x{A9}|\(c\)|\bcopyright\b|^c\s)[^\n]{0,60}?\b((?:1[89]|20)\d\d)\b")
            .expect("valid regex")
    })
}

fn conference() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "(NeurIPS 2026)", "(ICML 2024)".
    RE.get_or_init(|| {
        Regex::new(r"\([A-Z][A-Za-z]*[A-Z][A-Za-z]*\s((?:19|20)\d\d)\)").expect("valid regex")
    })
}

fn date_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // A line that is a date and nothing else, or "(Dated: June 15, 2022)".
    RE.get_or_init(|| {
        Regex::new(
            r"(?im)^\(?(?:dated:\s*)?(?:january|february|march|april|may|june|july|august|september|october|november|december)\s+(?:\d{1,2},?\s+)?((?:19|20)\d\d)\)?$",
        )
        .expect("valid regex")
    })
}

fn received() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\breceived\b[^\n;]{0,40}?\b((?:19|20)\d\d)\b").expect("valid regex")
    })
}

fn year_of(re: &Regex, text: &str, max_year: i32) -> Option<i32> {
    re.captures_iter(text)
        .filter_map(|caps| caps.get(1)?.as_str().parse::<i32>().ok())
        .find(|year| (1800..=max_year).contains(year))
}

/// The year `text` (a first page, top first) prints for itself, if any.
/// `max_year` bounds it: a year after next is a typo or a page number.
pub fn printed_year(text: &str, max_year: i32) -> Option<PrintedYear> {
    let head: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MASTHEAD_LINES)
        .collect();
    let strong = |year| PrintedYear { year, strong: true };
    let weak = |year| PrintedYear {
        year,
        strong: false,
    };
    let in_head = |re: &Regex| head.iter().find_map(|line| year_of(re, line, max_year));
    let any_line = |re: &Regex| {
        text.lines()
            .find_map(|line| year_of(re, line.trim(), max_year))
    };
    if let Some(year) = in_head(masthead())
        .or_else(|| in_head(volume_banner()))
        .or_else(|| any_line(whole_masthead()))
    {
        return Some(strong(year));
    }
    if let Some(year) =
        year_of(published(), text, max_year).or_else(|| year_of(cite_this(), text, max_year))
    {
        return Some(strong(year));
    }
    [copyright(), conference(), date_line(), received()]
        .into_iter()
        .find_map(|re| year_of(re, text, max_year))
        .map(weak)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i32 = 2027;

    fn year(text: &str) -> Option<(i32, bool)> {
        printed_year(text, NOW).map(|p| (p.year, p.strong))
    }

    #[test]
    fn a_masthead_states_the_year_of_record() {
        assert_eq!(
            year("PHYSICAL REVIEW C 80, 024601 (2009)\nTitle"),
            Some((2009, true))
        );
        assert_eq!(
            year("Computer Physics Communications 222 (2018) 263–266\nContents lists"),
            Some((2018, true))
        );
        assert_eq!(
            year("Journal of Machine Learning Research ? (2018) ?-?"),
            Some((2018, true))
        );
        // Over the copyright line further down, which is often the year the
        // article went online.
        assert_eq!(
            year("Nuclear Data Sheets 131 (2016) 107–221\nTitle\n0090-3752/© 2015 Elsevier Inc."),
            Some((2016, true))
        );
    }

    #[test]
    fn a_masthead_below_the_abstract_counts_when_it_is_the_whole_line() {
        let text = "Title\nAuthors\n(Received 11 July 2015; accepted 13 October 2015)\nAbstract text.\nNuclear Data Sheets 131 (2016) 107–221";
        assert_eq!(year(text), Some((2016, true)));
        let footnote =
            "Title\nAuthors\nAffiliation\nAbstract.\nMore abstract.\nIntroduction.\nsee Phys. Rev. C 80, 024601 (2009), and refs. therein";
        assert_eq!(year(footnote), None);
    }

    #[test]
    fn an_old_volume_banner_is_a_masthead() {
        let text =
            "PHYSICAL REVIEW C VOLUME 41, NUMBER 5 MAY 1990\nTitle\n(Received 25 October 1989)";
        assert_eq!(year(text), Some((1990, true)));
    }

    #[test]
    fn the_aps_received_line_says_when_it_was_published() {
        let text = "Title\nAuthors\n(Received 23 May 2011; revised manuscript received 11 January 2012; published 13 February 2012)";
        assert_eq!(year(text), Some((2012, true)));
    }

    #[test]
    fn published_online_is_not_the_year_of_a_back_issue() {
        let text = "Journal of Nuclear Science and Technology\nISSN: 0022-3131\nTo cite this article: K. Nishio\n(1995) Measurement of Fragment Mass\nPublished online: 15 Mar 2012.";
        assert_eq!(year(text), Some((1995, true)));
        assert_eq!(year("Title\nPublished online: 15 Mar 2012."), None);
    }

    #[test]
    fn copyright_conference_and_date_lines_are_weaker_evidence() {
        assert_eq!(
            year("Title\n©2009 The American Physical Society"),
            Some((2009, false))
        );
        assert_eq!(
            year("Title\nc 2018 Raef Bassily, Shay Moran."),
            Some((2018, false))
        );
        assert_eq!(
            year("Title\nc Massachusetts Institute of Technology 2011. All rights reserved."),
            Some((2011, false))
        );
        assert_eq!(
            year("PMLR: Volume 300. Copyright 2026 by the author(s)."),
            Some((2026, false))
        );
        assert_eq!(
            year("Submitted to 40th Conference on Neural Information Processing Systems (NeurIPS 2026)."),
            Some((2026, false))
        );
        assert_eq!(
            year("A Title\nAn Author\nSeptember 23, 2026\nAbstract"),
            Some((2026, false))
        );
        assert_eq!(year("(Dated: June 15, 2022)"), Some((2022, false)));
        assert_eq!(year("Title\nReceived March 31, 1965"), Some((1965, false)));
    }

    #[test]
    fn citations_in_the_text_are_not_the_papers_year() {
        assert_eq!(
            year("as shown by Littlestone and Warmuth (1986); Floyd and Warmuth (1995)"),
            None
        );
        assert_eq!(year("the 1939 seminal work of Bohr and Wheeler [1]"), None);
        assert_eq!(year("Title\n© 2091 Typo Press"), None);
    }
}
