//! A printed table of contents, read and mapped onto the pages it names.
//!
//! The page is found by its title ("Contents", "Table of Contents") among
//! the first pages, with the pages after it that carry on in the same
//! shape. Each row is an entry: its text, the page number at its end
//! (dot leaders dropped), and its indent, which gives its level.
//!
//! Printed page numbers are not page indices - "1" is often the fifteenth
//! page of the file. The document's own page labels map them when it has
//! them; otherwise the offset is found by looking for each entry's text on
//! the pages, and taken only when most entries agree on it. Every entry is
//! then looked for on its page, which is also what puts the jump at the
//! heading rather than the page top. Too few found, and the contents page
//! is not used at all.

use super::detect::{anchor_y, Doc, Heading, Line, Page, Stats, MAX_LEVELS};
use super::normalize;
use super::numbering;
use std::collections::{HashMap, HashSet};

/// The contents is looked for in the first pages only.
const MAX_TOC_START: usize = 40;
/// A contents page lists at least this many entries with page numbers.
const MIN_ROWS: usize = 4;
/// Share of entries that must be found on the page their number names.
const MIN_VERIFIED: f32 = 0.6;

pub(crate) fn is_contents_title(text: &str) -> bool {
    matches!(
        normalize::letters(text).as_str(),
        "contents"
            | "tableofcontents"
            | "briefcontents"
            | "detailedcontents"
            | "inhalt"
            | "inhaltsverzeichnis"
            | "sommaire"
            | "indice"
            | "contenido"
            | "contenidos"
            | "sumario"
    )
}

pub(crate) fn is_list_title(text: &str) -> bool {
    let key = normalize::letters(text);
    key.starts_with("listof") && key.len() <= 24
}

/// One row of a contents page: its segments joined left to right.
#[derive(Clone, Debug)]
struct Row {
    text: String,
    left: f32,
    top: f32,
    size: f32,
}

fn rows(page: &Page) -> Vec<Row> {
    let mut lines: Vec<&Line> = page.lines.iter().collect();
    lines.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left)));
    let mut out: Vec<(Vec<&Line>, f32, f32)> = Vec::new(); // members, center, size
    for line in lines {
        let center = (line.top + line.bottom) / 2.0;
        if let Some(row) = out.last_mut() {
            if (center - row.1).abs() < 0.4 * row.2.max(line.size) {
                row.0.push(line);
                continue;
            }
        }
        out.push((vec![line], center, line.size));
    }
    out.into_iter()
        .map(|(mut members, _, _)| {
            members.sort_by(|a, b| a.left.total_cmp(&b.left));
            Row {
                text: members
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                left: members[0].left,
                top: members.iter().map(|l| l.top).fold(f32::INFINITY, f32::min),
                size: members[0].size,
            }
        })
        .collect()
}

/// A row's title and the page number printed at its end.
fn split_entry(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    // The trailing token: after the last space or leader dot.
    let start = trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_ascii_alphanumeric())
        .map(|(i, c)| i + c.len_utf8())?;
    let (head, last) = trimmed.split_at(start);
    let arabic = !last.is_empty() && last.len() <= 4 && last.chars().all(|c| c.is_ascii_digit());
    let roman = !arabic
        && last.chars().all(|c| c.is_ascii_lowercase())
        && numbering::roman_value(&last.to_ascii_uppercase()).is_some();
    if !(arabic || roman) {
        return None;
    }
    let before = head.chars().last()?;
    if !matches!(before, ' ' | '.' | '\u{b7}' | '\u{2026}') {
        return None;
    }
    let title = head
        .trim_end_matches([' ', '.', '\u{b7}', '\u{2026}'])
        .trim();
    // leaders written with spaces: ". . . . ."
    let title = strip_leaders(title);
    if title.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return None;
    }
    Some((title, last.to_owned()))
}

fn strip_leaders(title: &str) -> String {
    let mut words: Vec<&str> = title.split_whitespace().collect();
    while words
        .last()
        .is_some_and(|w| w.chars().all(|c| matches!(c, '.' | '\u{b7}' | '\u{2026}')))
    {
        words.pop();
    }
    let joined = words.join(" ");
    // "Introduction........" glued to the title
    joined
        .trim_end_matches(['.', '\u{b7}', '\u{2026}', ' '])
        .to_owned()
}

/// Pages that are a table of contents or a list of figures or tables, by
/// position in `doc.pages`.
pub fn toc_pages(doc: &Doc) -> HashSet<usize> {
    let mut out = HashSet::new();
    let limit = doc.pages.len().min(MAX_TOC_START);
    let mut p = 0;
    while p < limit {
        let page = &doc.pages[p];
        let titled = page
            .lines
            .iter()
            .take(8)
            .any(|l| is_contents_title(&l.text) || is_list_title(&l.text));
        if titled && numbered_rows(page) >= MIN_ROWS {
            out.insert(p);
            // and the pages that carry on in the same shape
            let mut q = p + 1;
            while q < doc.pages.len() && q < p + 30 && is_continuation(&doc.pages[q]) {
                out.insert(q);
                q += 1;
            }
            p = q;
            continue;
        }
        p += 1;
    }
    out
}

fn numbered_rows(page: &Page) -> usize {
    rows(page)
        .iter()
        .filter(|r| split_entry(&r.text).is_some())
        .count()
}

fn is_continuation(page: &Page) -> bool {
    let all = rows(page);
    let with_letters = all
        .iter()
        .filter(|r| r.text.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .count();
    let numbered = numbered_rows(page);
    // An edited volume gives each entry a wrapped title line and an author
    // line besides its numbered one.
    numbered >= MIN_ROWS && numbered * 3 >= with_letters
}

#[derive(Clone, Debug)]
struct Entry {
    title: String,
    printed: Option<String>,
    left: f32,
}

/// The entries of the contents pages, in order. Only the first run of
/// contents pages counts, and not a list of figures.
fn entries(doc: &Doc, pages: &[usize]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut pending: Option<Row> = None;
    // A row with no page number stands as an entry of its own (a part
    // heading) only when it reads like one.
    let push_pageless = |out: &mut Vec<Entry>, row: Row| {
        let text = normalize::clean(&row.text);
        let words = text.split_whitespace().count();
        let reads_as_heading = words <= 10
            && !text.ends_with([',', '.', ';', ':'])
            && !text.starts_with(|c: char| c.is_lowercase());
        if reads_as_heading {
            out.push(Entry {
                title: text,
                printed: None,
                left: row.left,
            });
        }
    };
    for (k, &p) in pages.iter().enumerate() {
        let page = &doc.pages[p];
        let all = rows(page);
        // On the first page, only what follows the "Contents" title: an
        // abstract above it is not part of the list.
        let start = if k == 0 {
            all.iter()
                .position(|r| is_contents_title(&r.text))
                .map_or(0, |i| i + 1)
        } else {
            0
        };
        let h = page.height.max(1.0);
        for row in all.into_iter().skip(start) {
            let bare_number = {
                let t = row.text.trim();
                t.chars().all(|c| c.is_ascii_digit())
                    || numbering::roman_value(&t.to_ascii_uppercase()).is_some()
            };
            if row.top < 0.055 * h || row.top > 0.945 * h || bare_number {
                continue; // running head or page number
            }
            let Some((title, printed)) = split_entry(&row.text) else {
                if row.text.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
                    if let Some(prev) = pending.take() {
                        if prev.left <= row.left + 2.0 {
                            push_pageless(&mut out, prev);
                        }
                    }
                    pending = Some(row);
                }
                continue;
            };
            let mut title = normalize::clean(&title);
            let mut left = row.left;
            if let Some(prev) = pending.take() {
                let wrapped = row.left >= prev.left - 2.0
                    && row.left <= prev.left + 30.0
                    && (row.size - prev.size).abs() < 0.6
                    && numbering::parse(&row.text).is_none();
                if wrapped {
                    title = normalize::join_lines(&normalize::clean(&prev.text), &title);
                    left = prev.left;
                } else if prev.left <= row.left + 2.0 {
                    // a part heading over the entries that follow it; one
                    // indented past them is an annotation (an edited
                    // volume's chapter authors), not an entry
                    push_pageless(&mut out, prev);
                }
            }
            out.push(Entry {
                title,
                printed: Some(printed),
                left,
            });
        }
    }
    // A row left over at the end has nothing under it.
    drop(pending);
    out
}

/// A title's letters with its numbering set aside, for matching an entry
/// against the heading on its page.
fn match_key(text: &str) -> String {
    let title = numbering::parse(text).map_or_else(|| text.to_owned(), |n| n.title);
    normalize::letters(&normalize::clean(&title))
}

fn keys_match(entry: &str, line: &str) -> bool {
    if entry.len() < 4 || line.len() < 4 {
        return false;
    }
    if entry == line {
        return true;
    }
    let (short, long) = if entry.len() < line.len() {
        (entry, line)
    } else {
        (line, entry)
    };
    short.len() >= 10 && long.starts_with(short)
}

/// Whether `headings` reproduce `toc`: nearly every contents entry has a
/// heading of the same title on the same page.
pub fn covers(headings: &[Heading], toc: &[Heading]) -> bool {
    if headings.is_empty() || toc.is_empty() {
        return false;
    }
    let found = toc
        .iter()
        .filter(|entry| {
            let key = match_key(&entry.title);
            headings
                .iter()
                .any(|h| h.page == entry.page && keys_match(&key, &match_key(&h.title)))
        })
        .count();
    found * 10 >= toc.len() * 9
}

/// The outline a printed contents gives, if one is found and maps cleanly.
pub fn outline(doc: &Doc, stats: &Stats) -> Option<Vec<Heading>> {
    // The first run of contents pages, without lists of figures.
    let mut toc: Vec<usize> = stats.toc_pages.iter().copied().collect();
    toc.sort_unstable();
    let first = *toc.first()?;
    let titled = doc.pages[first]
        .lines
        .iter()
        .take(8)
        .any(|l| is_contents_title(&l.text));
    if !titled {
        return None;
    }
    let mut run = vec![first];
    while let Some(next) = toc.iter().find(|&&p| p == run.last().unwrap() + 1) {
        if doc.pages[*next]
            .lines
            .iter()
            .take(8)
            .any(|l| is_list_title(&l.text))
        {
            break;
        }
        run.push(*next);
    }
    let entries = entries(doc, &run);
    if entries.iter().filter(|e| e.printed.is_some()).count() < MIN_ROWS {
        return None;
    }
    let after = run.last().unwrap() + 1;

    // Every candidate line's match key, per page after the contents.
    let page_keys: Vec<Vec<(usize, String)>> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(p, page)| {
            if p < after || stats.toc_pages.contains(&p) {
                return Vec::new();
            }
            page.lines
                .iter()
                .enumerate()
                .filter(|(i, _)| !stats.furniture.contains(&(p, *i)))
                .map(|(i, l)| (i, match_key(&l.text)))
                .collect()
        })
        .collect();
    let found_on = |key: &str, p: usize| -> Option<usize> {
        let keys = page_keys.get(p)?;
        keys.iter()
            .filter(|(_, k)| keys_match(key, k))
            .map(|(i, _)| *i)
            .max_by(|a, b| {
                doc.pages[p].lines[*a]
                    .size
                    .total_cmp(&doc.pages[p].lines[*b].size)
            })
    };

    let entry_keys: Vec<String> = entries.iter().map(|e| match_key(&e.title)).collect();

    // Printed page -> position in doc.pages: labels, else a voted offset.
    let mut by_label: HashMap<&str, usize> = HashMap::new();
    let mut labels_differ = false;
    for (p, page) in doc.pages.iter().enumerate().skip(after) {
        if let Some(label) = &page.label {
            by_label.entry(label.as_str()).or_insert(p);
            if *label != (page.index + 1).to_string() {
                labels_differ = true;
            }
        }
    }
    let position_of_index: HashMap<u32, usize> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(p, page)| (page.index, p))
        .collect();
    let mut votes: HashMap<i64, usize> = HashMap::new();
    let mut tested = 0usize;
    for (entry, key) in entries.iter().zip(&entry_keys).take(60) {
        let Some(printed) = entry.printed.as_deref().and_then(|s| s.parse::<i64>().ok()) else {
            continue;
        };
        tested += 1;
        for (p, page) in doc.pages.iter().enumerate().skip(after) {
            if found_on(key, p).is_some() {
                *votes.entry(page.index as i64 - (printed - 1)).or_default() += 1;
            }
        }
    }
    let offset = votes
        .iter()
        .max_by_key(|(off, n)| (**n, -(off.abs())))
        .filter(|(_, n)| **n >= 3 && (**n as f32) >= 0.4 * tested as f32)
        .map(|(off, _)| *off);
    let use_labels = labels_differ && !by_label.is_empty();
    let target = |printed: &str| -> Option<usize> {
        if use_labels {
            if let Some(p) = by_label.get(printed) {
                return Some(*p);
            }
        }
        let n: i64 = printed.parse().ok()?;
        let index = n - 1 + offset?;
        position_of_index.get(&u32::try_from(index).ok()?).copied()
    };

    // Levels from indentation.
    let mut lefts: Vec<f32> = entries.iter().map(|e| e.left).collect();
    lefts.sort_by(f32::total_cmp);
    let mut clusters: Vec<f32> = Vec::new();
    for left in lefts {
        if clusters.last().is_none_or(|c| left - c > 6.0) {
            clusters.push(left);
        }
    }
    let level_of =
        |left: f32| -> usize { clusters.iter().rposition(|c| left >= c - 6.0).unwrap_or(0) };

    let mut headings: Vec<Heading> = Vec::new();
    let mut with_pages = 0usize;
    let mut verified = 0usize;
    let mut last_p = after;
    let mut waiting: Vec<(usize, &Entry, &String)> = Vec::new(); // entries with no page yet
    let mut headings_last_page = after;
    for (entry, key) in entries.iter().zip(&entry_keys) {
        let level = level_of(entry.left);
        let Some(printed) = &entry.printed else {
            waiting.push((level, entry, key));
            continue;
        };
        with_pages += 1;
        let Some(p) = target(printed) else { continue };
        // Look for the heading on its page, or one either side.
        let mut hit = None;
        for candidate in [p, p + 1, p.wrapping_sub(1)] {
            if let Some(i) = found_on(key, candidate) {
                hit = Some((candidate, i));
                break;
            }
        }
        let mut title = normalize::display(&entry.title);
        let (p, y) = match hit {
            Some((q, i)) => {
                verified += 1;
                let page = &doc.pages[q];
                // The heading as set on its page usually reads better than
                // its contents line (larger type, cleaner word spacing);
                // used when it is the same title, not just its start.
                // (A contents row's dot leaders throw off the word-gap
                // estimate; "Problemsetup" there is "Problem setup" here.)
                let printed = &page.lines[i].text;
                let prefix = |t: &str| numbering::parse(t).map(|n| n.prefix);
                if match_key(printed) == *key && prefix(printed) == prefix(&entry.title) {
                    title = normalize::display(printed);
                }
                (q, Some(anchor_y(page.height, page.lines[i].top)))
            }
            None => (p, None),
        };
        if p < last_p {
            // out of order: a misread number, not a section
            continue;
        }
        last_p = p;
        for (level, waiting_entry, waiting_key) in waiting.drain(..) {
            // A part heading with no page number of its own: its title
            // page is somewhere up to the entry that follows it.
            let start = headings_last_page.max(after);
            let found = (start..=p).find_map(|q| found_on(waiting_key, q).map(|i| (q, i)));
            let (q, y) = match found {
                Some((q, i)) => (
                    q,
                    Some(anchor_y(doc.pages[q].height, doc.pages[q].lines[i].top)),
                ),
                None => (p, None),
            };
            headings.push(Heading {
                level: level as u8,
                title: normalize::display(&waiting_entry.title),
                page: doc.pages[q].index,
                y,
            });
        }
        headings_last_page = p;
        headings.push(Heading {
            level: level as u8,
            title,
            page: doc.pages[p].index,
            y,
        });
    }
    if with_pages == 0 || (verified as f32) < MIN_VERIFIED * with_pages as f32 {
        return None;
    }
    headings.retain(|h| h.level < MAX_LEVELS);
    // Levels as used, from the shallowest.
    let mut used: Vec<u8> = headings.iter().map(|h| h.level).collect();
    used.sort_unstable();
    used.dedup();
    for h in &mut headings {
        h.level = used.iter().position(|l| *l == h.level).unwrap_or(0) as u8;
    }
    let top = headings.iter().filter(|h| h.level == 0).count();
    (top >= 2 && headings.len() >= 3).then_some(headings)
}

#[cfg(test)]
mod tests {
    use super::super::detect::tests::{doc, PageBuilder};
    use super::super::detect::{generate, Line};
    use super::*;

    fn row(text: &str, left: f32, top: f32) -> Line {
        Line {
            text: text.to_owned(),
            size: 10.0,
            top,
            bottom: top + 7.0,
            left,
            right: left + 300.0,
            ..Line::default()
        }
    }

    #[test]
    fn splits_titles_from_page_numbers_and_leaders() {
        assert_eq!(
            split_entry("1.2 Background . . . . . . . 17"),
            Some(("1.2 Background".into(), "17".into()))
        );
        assert_eq!(
            split_entry("Introduction..........3"),
            Some(("Introduction".into(), "3".into()))
        );
        assert_eq!(split_entry("Civil engineering civil"), None);
        assert_eq!(
            split_entry("Section 12"),
            Some(("Section".into(), "12".into()))
        );
        assert_eq!(
            split_entry("Introduction .......... 3"),
            Some(("Introduction".into(), "3".into()))
        );
        assert_eq!(
            split_entry("Preface xi"),
            Some(("Preface".into(), "xi".into()))
        );
        assert_eq!(split_entry("Results 2019 and beyond"), None);
    }

    /// A book: cover, contents (printed numbers), then chapters starting
    /// at printed page 1 = file page 4.
    fn book() -> Vec<Page> {
        let mut pages = vec![PageBuilder::new(0).heading("The Book", 24.0, true).build()];
        let mut toc = PageBuilder::new(1).heading("Contents", 18.0, true).build();
        let mut y = 150.0;
        for (text, left) in [
            ("1 Groups 1", 72.0),
            ("1.1 Definitions 1", 90.0),
            ("1.2 Examples 2", 90.0),
            ("2 Rings 3", 72.0),
            ("2.1 Ideals 3", 90.0),
            ("3 Fields 4", 72.0),
        ] {
            toc.lines.push(row(text, left, y));
            y += 14.0;
        }
        pages.push(toc);
        pages.push(PageBuilder::new(2).body(30).build());
        pages.push(
            PageBuilder::new(3)
                .heading("1 Groups", 20.0, true)
                .body(5)
                .heading("1.1 Definitions", 12.0, true)
                .body(20)
                .build(),
        );
        pages.push(
            PageBuilder::new(4)
                .heading("1.2 Examples", 12.0, true)
                .body(30)
                .build(),
        );
        pages.push(
            PageBuilder::new(5)
                .heading("2 Rings", 20.0, true)
                .body(5)
                .heading("2.1 Ideals", 12.0, true)
                .body(20)
                .build(),
        );
        pages.push(
            PageBuilder::new(6)
                .heading("3 Fields", 20.0, true)
                .body(30)
                .build(),
        );
        pages
    }

    #[test]
    fn printed_contents_maps_through_the_page_offset() {
        let out = generate(&doc(book()));
        let got: Vec<(u8, &str, u32)> = out
            .iter()
            .map(|h| (h.level, h.title.as_str(), h.page))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "1 Groups", 3),
                (1, "1.1 Definitions", 3),
                (1, "1.2 Examples", 4),
                (0, "2 Rings", 5),
                (1, "2.1 Ideals", 5),
                (0, "3 Fields", 6),
            ]
        );
        assert!(out.iter().all(|h| h.y.is_some()));
    }

    #[test]
    fn page_labels_map_printed_numbers_directly() {
        let mut pages = book();
        for (i, page) in pages.iter_mut().enumerate() {
            page.label = Some(if i < 3 {
                ["i", "ii", "iii"][i].to_owned()
            } else {
                (i - 2).to_string()
            });
        }
        let out = generate(&doc(pages));
        assert_eq!(out[0].page, 3);
        assert_eq!(out.last().unwrap().page, 6);
    }

    #[test]
    fn contents_lines_are_never_headings_themselves() {
        let out = generate(&doc(book()));
        // every entry points past the contents page
        assert!(out.iter().all(|h| h.page >= 3));
    }

    #[test]
    fn a_contents_that_does_not_map_is_not_used() {
        let mut pages = book();
        // Headings renamed: nothing in the contents can be found.
        for page in pages.iter_mut().skip(3) {
            for line in page.lines.iter_mut() {
                if line.bold {
                    line.text = format!("{} zz", line.text.split(' ').next().unwrap());
                }
            }
        }
        let stats = Stats::compute(&doc(pages.clone()));
        assert!(outline(&doc(pages), &stats).is_none());
    }
}
