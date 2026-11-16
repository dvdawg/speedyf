//! Every preview kind checked against the page it shows, read-only, over
//! real PDFs. Ignored by default:
//!
//! ```text
//! SPEEDYF_HOVER_EVAL=list.txt SPEEDYF_HOVER_OUT=report.txt \
//!   cargo test --lib hovers::eval_links -- --ignored --nocapture
//! ```
//!
//! Link previews are classified by the words of the link and just before it
//! ("Theorem 2", "Sec. 3", "Eq. (4)", "[12]", "Fig. 5"); text-detected hover
//! spans by their kind. The target is then found on the destination page
//! independently of the preview code - a caption, statement or section
//! head, an equation's number, a bibliography entry's first line - and its
//! extent read off the page's lines. A preview is right when it holds all
//! of the target and none of what surrounds it: the statement's proof, the
//! next reference, the previous section's tail, a line cut in half.

use super::page::{Line, Page};
use super::{extract, IndexBuilder};
use crate::engine::types::{LinkTarget, TextRun};
use crate::engine::{links, pdfium_init, render, text};
use pdfium_render::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const THEOREM_WORDS: &[&str] = &[
    "theorem",
    "lemma",
    "proposition",
    "corollary",
    "definition",
    "remark",
    "example",
    "assumption",
    "claim",
    "conjecture",
    "fact",
    "observation",
    "algorithm",
    "hypothesis",
    "condition",
    "property",
    "problem",
];

/// What a link or span points at.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Citation(Option<String>),
    Theorem(String, String),
    Equation(String),
    Section(String),
    Float(String, String),
    Footnote(String),
}

impl Target {
    fn kind(&self) -> &'static str {
        match self {
            Target::Citation(_) => "citation",
            Target::Theorem(..) => "theorem",
            Target::Equation(_) => "equation",
            Target::Section(_) => "section",
            Target::Float(kind, _) if kind == "table" => "table",
            Target::Float(..) => "figure",
            Target::Footnote(_) => "footnote",
        }
    }
}

fn is_number(token: &str) -> bool {
    let t = token.trim_matches(|c: char| matches!(c, '.' | ':' | ',' | ')' | '('));
    !t.is_empty()
        && t.split('.').all(|p| {
            !p.is_empty()
                && (p.chars().all(|c| c.is_ascii_digit())
                    || (p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase())))
        })
}

fn clean_number(token: &str) -> String {
    token
        .trim_matches(|c: char| matches!(c, '.' | ':' | ',' | ')' | '(' | '[' | ']' | '~'))
        .to_string()
}

/// Read a link: its own text and the words before it.
fn classify_link(before: &str, inside: &str, rect: [f32; 4]) -> Option<Target> {
    let inside = inside.trim();
    let words = format!("{} {}", before.trim_end(), inside);
    let lower = words.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let last_number = inside
        .split(|c: char| c.is_whitespace() || c == '~')
        .rev()
        .find(|t| is_number(t))
        .map(clean_number);
    // "Fig. 3", "Table 2"
    if let Some(number) = last_number.clone() {
        let n = tokens.len();
        let prior = |k: usize| tokens.get(n.wrapping_sub(k)).copied().unwrap_or("");
        for k in 2..=3 {
            let word = prior(k).trim_end_matches(['.', '~']);
            if matches!(word, "fig" | "figure" | "figs" | "figures") {
                return Some(Target::Float("figure".into(), number));
            }
            if matches!(word, "table" | "tables" | "tab") {
                return Some(Target::Float("table".into(), number));
            }
            if matches!(
                word,
                "section" | "sections" | "sec" | "secs" | "appendix" | "chapter"
            ) || word == "\u{a7}"
            {
                return Some(Target::Section(number));
            }
            if let Some(kind) = THEOREM_WORDS
                .iter()
                .find(|w| word == **w || word == format!("{w}s"))
            {
                return Some(Target::Theorem(kind.to_string(), number));
            }
            if matches!(word, "eq" | "eqs" | "equation" | "equations") {
                return Some(Target::Equation(number));
            }
        }
        if let Some(stripped) = inside.strip_prefix('\u{a7}') {
            return Some(Target::Section(clean_number(stripped)));
        }
    }
    // "(3)" alone: an equation; "[12]" or a bare number in brackets: a
    // citation; an author-year link: a citation.
    if inside.starts_with('(') && inside.ends_with(')') && is_number(inside) {
        return Some(Target::Equation(clean_number(inside)));
    }
    let bracketed = before.trim_end().ends_with('[') || inside.starts_with('[');
    if bracketed && inside.chars().any(|c| c.is_ascii_digit()) {
        return Some(Target::Citation(last_number));
    }
    let has_year = inside
        .split(|c: char| !c.is_ascii_digit())
        .any(|t| t.len() == 4 && (t.starts_with("19") || t.starts_with("20")));
    if has_year {
        return Some(Target::Citation(None));
    }
    // An author-year link on the names alone: "(Littlestone and Warmuth".
    let names = inside
        .split_whitespace()
        .all(|w| w.starts_with(char::is_uppercase) || matches!(w, "and" | "et" | "al." | "&"));
    let opens = before.trim_end().ends_with(['(', ';']);
    if names && opens && !inside.is_empty() {
        return Some(Target::Citation(None));
    }
    // A footnote mark: a lone small number.
    if is_number(inside) && inside.len() <= 2 && rect[3] < 7.0 {
        return Some(Target::Footnote(clean_number(inside)));
    }
    None
}

/// A hover span's target, by its kind and label.
fn classify_span(kind: &str, label: &str) -> Option<Target> {
    let number = clean_number(label.rsplit(' ').next()?);
    let word = label.split(' ').next().unwrap_or("").to_lowercase();
    Some(match kind {
        "equation" => Target::Equation(number),
        "section" => Target::Section(number),
        "figure" => Target::Float("figure".into(), number),
        "table" => Target::Float("table".into(), number),
        "footnote" => Target::Footnote(number),
        "theorem" => Target::Theorem(word, number),
        _ => return None,
    })
}

fn text_of(page: &Page, i: usize) -> String {
    page.line_text(i).trim().to_string()
}

/// The gap a page leaves between its lines of prose.
fn leading_gap(page: &Page) -> f32 {
    let mut gaps: Vec<f32> = page
        .lines
        .windows(2)
        .filter(|w| super::page::same_column(&w[0], &w[1]) && w[0].bottom > w[1].top)
        .map(|w| w[0].bottom - w[1].top)
        .filter(|g| *g < 20.0)
        .collect();
    gaps.sort_by(f32::total_cmp);
    gaps.get(gaps.len() / 2).copied().unwrap_or(3.0)
}

fn starts_statement(text: &str) -> bool {
    let lower = text.to_lowercase();
    let mut tokens = lower.split_whitespace();
    let (Some(word), Some(number)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    THEOREM_WORDS.contains(&word.trim_end_matches('.')) && is_number(number)
}

fn starts_section(text: &str) -> bool {
    let mut tokens = text.split_whitespace();
    let (Some(number), Some(word)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    is_number(number)
        && !number.starts_with('(')
        && word.starts_with(|c: char| c.is_uppercase())
        && word.chars().filter(|c| c.is_alphabetic()).count() >= 2
}

/// The line `i` is followed by, in reading order, when it continues the
/// same block: the same column and close, or the top of the next column.
fn next_line(page: &Page, i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < page.lines.len() && page.lines[j].flags.furniture {
        j += 1;
    }
    (j < page.lines.len()).then_some(j)
}

/// Where the target is and how far it runs: its lines, and the lines that
/// must stay out (the proof after it, the next entry, the line before a
/// heading).
struct Truth {
    lines: Vec<usize>,
    outside: Vec<usize>,
}

fn head_near(page: &Page, heads: Vec<usize>, anchor: Option<f32>) -> Option<usize> {
    match anchor {
        Some(y) => heads.into_iter().min_by(|&a, &b| {
            (page.lines[a].top - y)
                .abs()
                .total_cmp(&(page.lines[b].top - y).abs())
        }),
        None => heads.into_iter().next(),
    }
}

fn truth(page: &Page, target: &Target, anchor: Option<(Option<f32>, f32)>) -> Option<Truth> {
    let lines = &page.lines;
    let anchor_y = anchor.map(|a| a.1);
    let gap0 = leading_gap(page);
    let gap = |a: usize, b: usize| lines[a].bottom - lines[b].top;
    match target {
        Target::Theorem(word, number) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let lower = text_of(page, i).to_lowercase();
                    let mut t = lower.split_whitespace();
                    t.next().map(|w| w.trim_end_matches('.')) == Some(word.as_str())
                        && t.next().map(clean_number).as_deref() == Some(number.as_str())
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            let mut out = vec![head];
            let mut outside = Vec::new();
            let italic = next_line(page, head).is_some_and(|j| lines[j].font != page.body_font);
            let mut i = head;
            while out.len() < 40 {
                let Some(j) = next_line(page, i) else { break };
                let t = text_of(page, j);
                let new_column = lines[j].top > lines[i].top + 1.0;
                let display = lines[j].is_display() || lines[i].is_display();
                let ends = t.starts_with("Proof")
                    || starts_statement(&t)
                    || starts_section(&t)
                    || (!new_column && !display && gap(i, j) > gap0 + 3.5)
                    || (italic && !display && lines[j].font == page.body_font && !t.is_empty());
                if ends {
                    outside.push(j);
                    break;
                }
                out.push(j);
                i = j;
            }
            Some(Truth {
                lines: out,
                outside,
            })
        }
        Target::Section(number) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    let mut tokens = t.split_whitespace();
                    let first = tokens.next().map(clean_number);
                    let first = if first.as_deref() == Some("Appendix") {
                        tokens.next().map(clean_number)
                    } else {
                        first
                    };
                    first.as_deref() == Some(number.as_str()) && starts_section(&t)
                        || t.starts_with(&format!("Appendix {number}"))
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            let mut out = vec![head];
            let mut i = head;
            for _ in 0..2 {
                let Some(j) = next_line(page, i) else { break };
                if lines[j].top > lines[i].top + 1.0 {
                    break;
                }
                out.push(j);
                i = j;
            }
            // The line just above the heading, in its column: the previous
            // section's tail.
            let above = (0..lines.len())
                .filter(|&k| {
                    k != head
                        && !lines[k].flags.furniture
                        && lines[k].bottom >= lines[head].top
                        && super::page::same_column(&lines[k], &lines[head])
                })
                .min_by(|&a, &b| lines[a].bottom.total_cmp(&lines[b].bottom));
            Some(Truth {
                lines: out,
                outside: above.into_iter().collect(),
            })
        }
        Target::Equation(number) => {
            let tag = format!("({number})");
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    t.ends_with(&tag) || t.starts_with(&tag)
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            // The display: the math lines set close above and below.
            let mut out = vec![head];
            let (mut x0, mut x1) = (lines[head].x0, lines[head].x1);
            loop {
                let mut grew = false;
                let top = out.iter().map(|&i| lines[i].top).fold(f32::MIN, f32::max);
                let bottom = out
                    .iter()
                    .map(|&i| lines[i].bottom)
                    .fold(f32::MAX, f32::min);
                for (k, l) in lines.iter().enumerate() {
                    if out.contains(&k) || l.flags.furniture {
                        continue;
                    }
                    let overlaps = l.x1.min(x1) - l.x0.max(x0) > 0.0;
                    let near = (l.bottom >= bottom - 0.1 && l.bottom - top <= 0.6 * l.em())
                        || (l.top <= top + 0.1 && bottom - l.top <= 0.6 * l.em())
                        || (l.top <= top + 0.5 && l.bottom >= bottom - 0.5);
                    let tagged_other = {
                        let t = text_of(page, k);
                        t.ends_with(')') && t.contains(" (") && !t.ends_with(&tag)
                    };
                    if overlaps && near && l.is_display() && !tagged_other {
                        out.push(k);
                        x0 = x0.min(l.x0);
                        x1 = x1.max(l.x1);
                        grew = true;
                    }
                }
                if !grew || out.len() > 16 {
                    break;
                }
            }
            Some(Truth {
                lines: out,
                outside: Vec::new(),
            })
        }
        Target::Citation(number) => {
            let head = match number {
                Some(n) => {
                    let label = format!("[{n}]");
                    let heads: Vec<usize> = (0..lines.len())
                        .filter(|&i| text_of(page, i).starts_with(&label))
                        .collect();
                    head_near(page, heads, anchor_y)
                }
                None => None,
            };
            let head = match head {
                Some(h) => h,
                None => {
                    // The entry's first line: the first line at or below the
                    // anchor, by its x when the destination names one.
                    let (x, y) = anchor?;
                    (0..lines.len())
                        .filter(|&i| {
                            let l = &lines[i];
                            !l.flags.furniture
                                && l.top <= y + 3.0
                                && l.top >= y - 30.0
                                && x.is_none_or(|x| l.x0 <= x + 30.0 && l.x1 >= x)
                        })
                        .max_by(|&a, &b| lines[a].top.total_cmp(&lines[b].top))?
                }
            };
            let mut out = vec![head];
            let mut outside = Vec::new();
            // Hanging: lines on the page that start at this entry's margin
            // go on indented.
            let indents = |a: usize, b: usize| {
                (3.0..=30.0).contains(&(lines[b].x0 - lines[a].x0))
                    && gap(a, b) <= gap0 + 4.0
                    && lines[b].top < lines[a].top
            };
            let hanging = (0..lines.len())
                .filter(|&k| {
                    (lines[k].x0 - lines[head].x0).abs() <= 1.5
                        && next_line(page, k).is_some_and(|j| indents(k, j))
                })
                .count()
                >= 2;
            let mut i = head;
            while out.len() < 12 {
                let Some(j) = next_line(page, i) else { break };
                let t = text_of(page, j);
                let new_column = lines[j].top > lines[i].top + 1.0;
                let starts = t.starts_with('[')
                    || (hanging && lines[j].x0 <= lines[head].x0 + 1.5)
                    || (!hanging && !new_column && gap(i, j) > gap0 + 1.5);
                if starts {
                    outside.push(j);
                    break;
                }
                // An entry running on into the next column is judged on
                // its first part; a line far below is the page's foot.
                if new_column || gap(i, j) > gap0 + 6.0 {
                    break;
                }
                out.push(j);
                i = j;
            }
            Some(Truth {
                lines: out,
                outside,
            })
        }
        Target::Footnote(mark) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    t.starts_with(mark.as_str())
                        && !t[mark.len()..].starts_with(|c: char| c.is_ascii_digit())
                        && lines[i].bottom < 0.35 * page.height
                        && (page.body_size <= 0.0 || lines[i].size < page.body_size - 0.3)
                })
                .collect();
            let head = head_near(page, heads, None)?;
            let size = lines[head].size;
            let mut out = vec![head];
            let mut i = head;
            while out.len() < 12 {
                let Some(j) = next_line(page, i) else { break };
                let t = text_of(page, j);
                let next_mark = t
                    .split_whitespace()
                    .next()
                    .is_some_and(|w| w.len() <= 2 && w.chars().all(|c| c.is_ascii_digit()));
                if (lines[j].size - size).abs() > 0.5 || next_mark || gap(i, j) > 4.0 {
                    break;
                }
                out.push(j);
                i = j;
            }
            Some(Truth {
                lines: out,
                outside: Vec::new(),
            })
        }
        Target::Float(..) => None,
    }
}

#[derive(Default)]
struct Tally {
    checked: usize,
    no_truth: usize,
    missing: usize,
    start_cut: usize,
    leaks: usize,
    cut: usize,
}

impl Tally {
    fn add(&mut self, t: &Tally) {
        self.checked += t.checked;
        self.no_truth += t.no_truth;
        self.missing += t.missing;
        self.start_cut += t.start_cut;
        self.leaks += t.leaks;
        self.cut += t.cut;
    }

    fn line(&self, name: &str) -> String {
        format!(
            "{name:<22} checked {:>5}  target-cut {:>4}  head-missing {:>4}  leaks {:>4}  cut-line {:>4}  (no truth {})",
            self.checked, self.missing, self.start_cut, self.leaks, self.cut, self.no_truth
        )
    }
}

/// Judge a crop: all of the target inside, its head above all, nothing
/// that must stay out, no line cut through.
fn judge(page: &Page, crop: [f32; 4], truth: &Truth, tally: &mut Tally) -> Vec<String> {
    tally.checked += 1;
    let mut problems = Vec::new();
    let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
    let inside = |l: &Line| {
        l.x0 >= cx0 - 2.0 && l.x1 <= cx1 + 2.0 && l.bottom >= cy0 - 1.5 && l.top <= cy1 + 1.5
    };
    let share = |l: &Line| {
        if l.x1.min(cx1) - l.x0.max(cx0) <= 3.0 {
            return 0.0;
        }
        ((l.top.min(cy1) - l.bottom.max(cy0)) / l.height()).max(0.0)
    };
    let head = truth.lines[0];
    if !inside(&page.lines[head]) {
        tally.start_cut += 1;
        problems.push(format!("HEAD missing: {:?}", text_of(page, head)));
    }
    if let Some(&i) = truth.lines.iter().find(|&&i| !inside(&page.lines[i])) {
        tally.missing += 1;
        problems.push(format!(
            "TARGET cut: {:?} ({} lines)",
            text_of(page, i),
            truth.lines.len()
        ));
    }
    if let Some(&i) = truth.outside.iter().find(|&&i| share(&page.lines[i]) > 0.5) {
        tally.leaks += 1;
        problems.push(format!("LEAKS: {:?}", text_of(page, i)));
    }
    let cut = (0..page.lines.len()).find(|&i| {
        let l = &page.lines[i];
        let s = share(l);
        !l.flags.furniture && s > 0.15 && s < 0.85 && text_of(page, i).chars().count() >= 3
    });
    if let Some(i) = cut {
        tally.cut += 1;
        problems.push(format!("CUTS: {:?}", text_of(page, i)));
    }
    problems
}

type Link = ([f32; 4], Option<f32>, Option<f32>, u32);

struct Doc {
    name: String,
    pages: Vec<Option<Page>>,
    runs: Vec<Vec<TextRun>>,
    sizes: Vec<[f32; 2]>,
    links: Vec<Vec<Link>>,
    spans: std::collections::HashMap<u32, Vec<super::HoverSpanDto>>,
}

fn load(b: &dyn PdfiumLibraryBindings, path: &str) -> Option<Doc> {
    let raw = render::open_document(b, path, None).ok()?;
    let count = (b.FPDF_GetPageCount(raw).max(0) as u32).min(80);
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut doc = Doc {
        name,
        pages: Vec::new(),
        runs: Vec::new(),
        sizes: Vec::new(),
        links: Vec::new(),
        spans: Default::default(),
    };
    let mut builder = IndexBuilder::new(count);
    while let Some(src) = builder.next_page() {
        let size = render::page_display_size(b, raw, src as i32);
        let handle = b.FPDF_LoadPage(raw, src as i32);
        let model = match (size, handle.is_null()) {
            (Some([w, h]), false) => {
                let input = extract::extract(b, handle, w, h);
                let extracted = text::extract_page(b, handle, w, h);
                let tp = b.FPDFText_LoadPage(handle);
                let found =
                    links::extract_links(b, raw, handle, tp, [w, h], &extracted.boxes, count);
                if !tp.is_null() {
                    b.FPDFText_ClosePage(tp);
                }
                doc.links.push(
                    found
                        .into_iter()
                        .filter_map(|l| match l.target {
                            LinkTarget::Internal { page, x, y } => Some((l.rect, x, y, page)),
                            _ => None,
                        })
                        .collect(),
                );
                doc.runs.push(extracted.runs);
                doc.sizes.push([w, h]);
                Some(Page::new(input))
            }
            _ => {
                doc.links.push(Vec::new());
                doc.runs.push(Vec::new());
                doc.sizes.push([0.0, 0.0]);
                None
            }
        };
        if !handle.is_null() {
            b.FPDF_ClosePage(handle);
        }
        builder.add_page(model.clone());
        doc.pages
            .push(model.map(|p| super::prepare_page(p, Default::default())));
    }
    doc.spans = builder.finish().spans;
    b.FPDF_CloseDocument(raw);
    Some(doc)
}

/// Where a run of the page's characters sits: the characters whose glyphs
/// fall inside `rect`.
fn locate(page: &Page, rect: [f32; 4]) -> Option<(usize, usize)> {
    let inside: Vec<usize> = (0..page.text.len())
        .filter(|&i| {
            page.glyph_at(i).is_some_and(|g| {
                let cx = g.rect[0] + g.rect[2] / 2.0;
                let cy = g.rect[1] + g.rect[3] / 2.0;
                g.has_box()
                    && cx >= rect[0]
                    && cx <= rect[0] + rect[2]
                    && cy >= rect[1]
                    && cy <= rect[1] + rect[3]
            })
        })
        .collect();
    Some((*inside.first()?, *inside.last()? + 1))
}

#[test]
#[ignore]
fn evaluate_all_previews() {
    let Ok(list) = std::env::var("SPEEDYF_HOVER_EVAL") else {
        return;
    };
    let out_path =
        std::env::var("SPEEDYF_HOVER_OUT").unwrap_or_else(|_| "preview-kinds.txt".into());
    let paths: Vec<String> = std::fs::read_to_string(list)
        .expect("list")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let mut totals: BTreeMap<String, Tally> = BTreeMap::new();
    let mut detail = String::new();
    for path in paths {
        let Some(doc) = load(b, &path) else { continue };
        // link previews
        let mut seen = std::collections::HashSet::new();
        for (src, links_here) in doc.links.iter().enumerate() {
            let Some(source) = doc.pages[src].as_ref() else {
                continue;
            };
            for &(rect, x, y, to) in links_here {
                let Some((start, end)) = locate(source, rect) else {
                    continue;
                };
                let before: String = source.text[start.saturating_sub(16)..start]
                    .iter()
                    .collect();
                let inside: String = source.text[start..end].iter().collect();
                let Some(target) = classify_link(&before, &inside, rect) else {
                    continue;
                };
                if matches!(target, Target::Float(..)) {
                    continue;
                }
                let Some(Some(page)) = doc.pages.get(to as usize) else {
                    continue;
                };
                // Destinations can be shared (a page-level one, the top of
                // the reference list): the link's text tells them apart.
                if !seen.insert((
                    to,
                    x.map(f32::to_bits),
                    y.map(f32::to_bits),
                    inside.trim().to_string(),
                )) {
                    continue;
                }
                let key = format!("link {}", target.kind());
                let tally = totals.entry(key.clone()).or_default();
                let anchor = y.map(|y| (x, y));
                let Some(truth) = truth(page, &target, anchor) else {
                    tally.no_truth += 1;
                    let _ = writeln!(
                        detail,
                        "{} NO-TRUTH {key} {target:?} {inside:?} p{}->p{} x={x:?} y={y:?}",
                        doc.name,
                        src + 1,
                        to + 1
                    );
                    continue;
                };
                let [w, h] = doc.sizes[to as usize];
                let crop = if std::env::var("SPEEDYF_OLD_CROP").is_ok() {
                    crate::engine::preview::crop_rect(&crate::engine::preview::CropInput {
                        page_w_pt: w,
                        page_h_pt: h,
                        dest_x: x,
                        dest_y: y,
                        runs: &doc.runs[to as usize],
                    })
                } else {
                    {
                        let words = super::targets::LinkText::from_runs(&doc.runs[src], rect);
                        super::targets::link_crop(
                            Some(page),
                            &doc.runs[to as usize],
                            w,
                            h,
                            x,
                            y,
                            Some(&words),
                        )
                    }
                };
                let problems = judge(page, crop, &truth, tally);
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{} {key} {target:?} p{}->p{} x={x:?} y={y:?} crop [{:.0},{:.0},{:.0},{:.0}]\n    {}",
                        doc.name,
                        src + 1,
                        to + 1,
                        crop[0],
                        crop[1],
                        crop[0] + crop[2],
                        crop[1] + crop[3],
                        problems.join("\n    ")
                    );
                }
            }
        }
        // text-detected hover spans
        let mut seen = std::collections::HashSet::new();
        for spans in doc.spans.values() {
            for span in spans {
                let Some(region) = span.target.region else {
                    continue;
                };
                let Some(target) = classify_span(span.kind, &span.label) else {
                    continue;
                };
                if matches!(target, Target::Float(..)) {
                    continue;
                }
                let to = span.target.page as usize;
                if !seen.insert((to, region.map(f32::to_bits))) {
                    continue;
                }
                let Some(Some(page)) = doc.pages.get(to) else {
                    continue;
                };
                let key = format!("span {}", target.kind());
                let tally = totals.entry(key.clone()).or_default();
                let anchor = Some((span.target.x, span.target.y));
                let Some(truth) = truth(page, &target, anchor) else {
                    tally.no_truth += 1;
                    continue;
                };
                let problems = judge(page, region, &truth, tally);
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{} {key} {target:?} ->p{} region [{:.0},{:.0},{:.0},{:.0}]\n    {}",
                        doc.name,
                        to + 1,
                        region[0],
                        region[1],
                        region[0] + region[2],
                        region[1] + region[3],
                        problems.join("\n    ")
                    );
                }
            }
        }
    }
    let mut report = String::new();
    let mut all = Tally::default();
    for (kind, tally) in &totals {
        let _ = writeln!(report, "{}", tally.line(kind));
        all.add(tally);
    }
    let _ = writeln!(report, "{}", all.line("TOTAL"));
    let _ = writeln!(report, "\nPROBLEMS\n{detail}");
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("wrote {out_path}");
}

/// One destination's lines and target, for debugging:
/// `SPEEDYF_TARGET=path.pdf:page:x:y` (1-based page; x may be `-`).
#[test]
#[ignore]
fn dump_link_target() {
    let Ok(spec) = std::env::var("SPEEDYF_TARGET") else {
        return;
    };
    let parts: Vec<&str> = spec.rsplitn(4, ':').collect();
    let (y, x, page, path) = (parts[0], parts[1], parts[2], parts[3]);
    let y: f32 = y.parse().expect("y");
    let x: Option<f32> = x.parse().ok();
    let src: i32 = page.parse::<i32>().expect("page") - 1;
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let raw = render::open_document(b, path, None).expect("open");
    let [w, h] = render::page_display_size(b, raw, src).expect("size");
    let handle = b.FPDF_LoadPage(raw, src);
    let page = super::prepare_page(
        Page::new(extract::extract(b, handle, w, h)),
        Default::default(),
    );
    for (i, l) in page.lines.iter().enumerate() {
        if (l.top - y).abs() < 60.0 {
            eprintln!(
                "line {i} x {:.1}..{:.1} y {:.1}..{:.1} size {:.1} font {} disp {} {:?}",
                l.x0,
                l.x1,
                l.bottom,
                l.top,
                l.size,
                l.font,
                l.is_display(),
                text_of(&page, i).chars().take(70).collect::<String>()
            );
        }
    }
    eprintln!("target {:?}", super::targets::link_target(&page, x, y));
    b.FPDF_ClosePage(handle);
    b.FPDF_CloseDocument(raw);
}

/// A document's internal links with their text and destination:
/// `SPEEDYF_LINKS=path.pdf`.
#[test]
#[ignore]
fn dump_links() {
    let Ok(path) = std::env::var("SPEEDYF_LINKS") else {
        return;
    };
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let doc = load(b, &path).expect("load");
    for (src, links_here) in doc.links.iter().enumerate() {
        let Some(source) = doc.pages[src].as_ref() else {
            continue;
        };
        for &(rect, x, y, to) in links_here {
            let (before, inside) = locate(source, rect)
                .map(|(s, e)| {
                    (
                        source.text[s.saturating_sub(16)..s]
                            .iter()
                            .collect::<String>(),
                        source.text[s..e].iter().collect::<String>(),
                    )
                })
                .unwrap_or_default();
            eprintln!(
                "p{} {:?} | {:?} -> p{} x={x:?} y={y:?} as {:?}",
                src + 1,
                before,
                inside,
                to + 1,
                classify_link(&before, &inside, rect)
            );
        }
    }
}
