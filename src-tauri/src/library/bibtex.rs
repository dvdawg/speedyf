//! BibTeX serialization for papers (projects.md sec. 7).
//!
//! Rendered in Rust, not in the webview, so the one-off export, the synced
//! `.bib` and the clipboard copy are the same bytes from the same serializer:
//! a key that compiles in one place compiles in all three, and the
//! "incomplete" comment the project header counts is the one the file holds.
//!
//! The input is the wire DTO rather than the on-disk record so the commands
//! can call it with what they already hold. Output is deterministic: no
//! clock, no randomness, field order fixed.

use super::types::{Author, MetaField, PaperDto, PaperKind};
use serde::{Deserialize, Serialize};

/// How entries are written, the same for the clipboard, an export and the
/// synced `.bib` files. Kept in `library.json` (`bibtexOptions`), so a
/// synced file reads the same on every machine. The default is what the
/// serializer wrote before there were options.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct BibtexOptions {
    /// An `abstract` field.
    pub include_abstract: bool,
    /// A `url` field: the doi.org link, else the arXiv abstract page.
    pub include_url: bool,
    /// A `file` field: the path of the item's main file.
    pub include_file: bool,
    /// Double braces around the title, so styles keep its capitals.
    pub protect_title: bool,
    /// `Phys. Rev. Lett.` for `Physical Review Letters`, for the journals
    /// in [`JOURNAL_ABBREVIATIONS`]; any other name is left as it is.
    pub abbreviate_journals: bool,
}

impl Default for BibtexOptions {
    fn default() -> Self {
        Self {
            include_abstract: false,
            include_url: false,
            include_file: false,
            protect_title: true,
            abbreviate_journals: false,
        }
    }
}

/// Full journal names, lower-case and without a leading "the", and their
/// ISO 4 abbreviations: common physics, mathematics and CS venues.
pub const JOURNAL_ABBREVIATIONS: &[(&str, &str)] = &[
    ("physical review letters", "Phys. Rev. Lett."),
    ("physical review e", "Phys. Rev. E"),
    ("reviews of modern physics", "Rev. Mod. Phys."),
    ("journal of statistical physics", "J. Stat. Phys."),
    ("journal of chemical physics", "J. Chem. Phys."),
    ("annals of statistics", "Ann. Statist."),
    (
        "journal of machine learning research",
        "J. Mach. Learn. Res.",
    ),
    (
        "communications in mathematical physics",
        "Commun. Math. Phys.",
    ),
    (
        "ieee transactions on information theory",
        "IEEE Trans. Inf. Theory",
    ),
    (
        "proceedings of the national academy of sciences",
        "Proc. Natl. Acad. Sci. USA",
    ),
    (
        "proceedings of the national academy of sciences of the united states of america",
        "Proc. Natl. Acad. Sci. USA",
    ),
    (
        "proceedings of the national academy of sciences usa",
        "Proc. Natl. Acad. Sci. USA",
    ),
];

/// The abbreviation of a journal name in the table, matched without
/// regard to case, spacing or a leading "The"; `None` for any other.
pub fn abbreviate_journal(name: &str) -> Option<&'static str> {
    let folded = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let folded = folded.strip_prefix("the ").unwrap_or(&folded);
    JOURNAL_ABBREVIATIONS
        .iter()
        .find(|(full, _)| *full == folded)
        .map(|(_, short)| *short)
}

/// First line of a synced file. It tells the user, in the file itself, that
/// hand edits do not survive.
const HEADER: &str = "% Written by SpeedyF. Edits here are overwritten on the next sync.";

/// Every entry for `papers`, in the given order, one blank line between
/// entries, trailing newline. With `header`, the SpeedyF comment line and a
/// blank line come first.
pub fn render(papers: &[PaperDto], header: bool, options: &BibtexOptions) -> String {
    let mut out = String::new();
    if header {
        out.push_str(HEADER);
        out.push('\n');
        if !papers.is_empty() {
            out.push('\n');
        }
    }
    // Each entry ends with exactly one newline, so joining on a newline is
    // exactly one blank line between entries and none after the last.
    let entries: Vec<String> = papers
        .iter()
        .map(|paper| render_one(paper, options))
        .collect();
    out.push_str(&entries.join("\n"));
    out
}

/// One entry: the `% incomplete:` comment line when the paper has gaps, then
/// `@type{key,` the fields and the closing brace. Ends with a single newline
/// (no blank line) so it pastes cleanly and `render` can stack entries.
pub fn render_one(paper: &PaperDto, options: &BibtexOptions) -> String {
    let meta = &paper.meta;
    let kind = entry_type(paper);
    let mut out = String::new();

    if !paper.incomplete.is_empty() {
        let gaps: Vec<String> = paper
            .incomplete
            .iter()
            .map(|f| format!("no {}", field_label(f)))
            .collect();
        out.push_str("% incomplete: ");
        out.push_str(&gaps.join(", "));
        out.push('\n');
    }

    // (name, value already escaped and braced as it should appear inside the
    // outer `{...}`). Order is the spec's; absent fields are simply not pushed.
    let mut fields: Vec<(&str, String)> = Vec::new();

    if let Some(authors) = &meta.authors {
        let joined = format_authors(&authors.value);
        if !joined.is_empty() {
            fields.push(("author", escape(&joined)));
        }
    }
    if let Some(title) = text(&meta.title) {
        // Double braces: BibTeX styles lower-case titles unless protected.
        let title = if options.protect_title {
            format!("{{{}}}", escape(title))
        } else {
            escape(title)
        };
        fields.push(("title", title));
    }
    if let Some(venue) = text(&meta.venue) {
        // `journal` and `booktitle` are what the standard styles print for
        // the two types that have a venue. The others have no venue field of
        // their own; `howpublished` keeps the value in the file rather than
        // dropping it, and bibtex ignores a field a style does not use.
        let name = match kind {
            "article" => "journal",
            "inproceedings" => "booktitle",
            _ => "howpublished",
        };
        let venue = match (name, options.abbreviate_journals) {
            ("journal", true) => abbreviate_journal(venue).unwrap_or(venue),
            _ => venue,
        };
        fields.push((name, escape(venue)));
    }
    if let Some(year) = &meta.year {
        fields.push(("year", year.value.to_string()));
    }
    if let Some(volume) = text(&meta.volume) {
        fields.push(("volume", escape(volume)));
    }
    if let Some(issue) = text(&meta.issue) {
        fields.push(("number", escape(issue)));
    }
    if let Some(pages) = text(&meta.pages) {
        fields.push(("pages", escape(&normalize_pages(pages))));
    }
    if let Some(publisher) = text(&meta.publisher) {
        // A thesis is granted by a school, which is where its "publisher"
        // ends up in the metadata.
        let name = if kind == "phdthesis" {
            "school"
        } else {
            "publisher"
        };
        fields.push((name, escape(publisher)));
    }
    if let Some(doi) = present(&paper.doi) {
        fields.push(("doi", identifier(doi)));
    }
    if arxiv_only(paper) {
        // `arxiv_only` guarantees the id is present.
        let id = present(&paper.arxiv).unwrap_or_default();
        fields.push(("eprint", identifier(id)));
        fields.push(("archivePrefix", "arXiv".to_string()));
    }
    if options.include_url {
        let url = match (present(&paper.doi), present(&paper.arxiv)) {
            (Some(doi), _) => Some(format!("https://doi.org/{doi}")),
            (None, Some(arxiv)) => Some(format!("https://arxiv.org/abs/{arxiv}")),
            (None, None) => None,
        };
        if let Some(url) = url {
            fields.push(("url", identifier(&url)));
        }
    }
    if options.include_abstract {
        if let Some(abstract_) = text(&meta.abstract_) {
            fields.push(("abstract", escape(abstract_)));
        }
    }
    if options.include_file {
        // The main file, as written: a path is read raw like an identifier.
        if let Some(file) = paper.files.iter().find(|file| !file.missing) {
            fields.push(("file", identifier(&file.path)));
        }
    }
    if !paper.incomplete.is_empty() {
        // Name the file so the user can find the paper in the list and look
        // it up; a bare "incomplete" in a 200-entry file helps nobody.
        let note = match paper.files.first() {
            Some(file) => format!("Metadata incomplete; file {}", escape(&file.name)),
            None => "Metadata incomplete".to_string(),
        };
        fields.push(("note", note));
    }

    out.push('@');
    out.push_str(kind);
    out.push('{');
    out.push_str(&paper.cite_key);
    out.push_str(",\n");
    let last = fields.len().saturating_sub(1);
    for (i, (name, value)) in fields.iter().enumerate() {
        out.push_str("  ");
        out.push_str(name);
        out.push_str(" = {");
        out.push_str(value);
        out.push('}');
        if i != last {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// Escapes a BibTeX field value: `& % $ # _ { } ~ ^ \`. Everything else,
/// non-ASCII included, passes through as UTF-8, which every modern TeX reads.
///
/// Braces become `\textbraceleft{}` / `\textbraceright{}` rather than `\{`
/// because bibtex finds the end of a field by counting braces and does not
/// skip an escaped one: a lone `\{` in a title leaves the field open and the
/// whole file unreadable. The text-symbol forms are balanced by construction.
/// `~`, `^` and `\` take their text-symbol forms for the same reason: `\~{}`
/// and `\^{}` are accents waiting for a letter, and a raw `\` would begin a
/// command.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("\\&"),
            '%' => out.push_str("\\%"),
            '$' => out.push_str("\\$"),
            '#' => out.push_str("\\#"),
            '_' => out.push_str("\\_"),
            '{' => out.push_str("\\textbraceleft{}"),
            '}' => out.push_str("\\textbraceright{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '\\' => out.push_str("\\textbackslash{}"),
            _ => out.push(ch),
        }
    }
    out
}

/// `Family, Given and Family, Given`. The comma form is the one BibTeX parses
/// unambiguously for multi-word family names. An author with no given name is
/// just `Family`; one with no family name is just `Given` (BibTeX reads a
/// lone word as a family name, which is the best it can do); one with
/// neither is skipped. An empty list is `""`. Not escaped: `render_one`
/// escapes the joined string, and the separators have nothing to escape.
pub fn format_authors(authors: &[Author]) -> String {
    let names: Vec<String> = authors
        .iter()
        .filter_map(|a| {
            let family = a.family.trim();
            let given = a.given.trim();
            match (family.is_empty(), given.is_empty()) {
                (true, true) => None,
                (false, true) => Some(family.to_string()),
                (true, false) => Some(given.to_string()),
                (false, false) => Some(format!("{family}, {given}")),
            }
        })
        .collect();
    names.join(" and ")
}

/// The entry type, without the `@`. An explicit kind wins. Without one, a
/// venue makes it an article and nothing else makes it `misc` - including
/// the arXiv-only paper, which by definition has no venue.
pub fn entry_type(paper: &PaperDto) -> &'static str {
    match paper.meta.kind.as_ref().map(|k| k.value) {
        Some(PaperKind::Article) => "article",
        Some(PaperKind::Inproceedings) => "inproceedings",
        Some(PaperKind::Book) => "book",
        Some(PaperKind::Thesis) => "phdthesis",
        Some(PaperKind::Misc) => "misc",
        None if text(&paper.meta.venue).is_some() => "article",
        None => "misc",
    }
}

/// An arXiv id, no DOI, no venue: the preprint is the only thing there is to
/// cite, so `eprint` is the locator.
fn arxiv_only(paper: &PaperDto) -> bool {
    present(&paper.arxiv).is_some()
        && present(&paper.doi).is_none()
        && text(&paper.meta.venue).is_none()
}

/// `2690-2693` and `2690-2693` become `2690--2693`, the en dash TeX typesets
/// for a range. Anything that is not exactly two page labels around one dash
/// (`e1234`, `S12`, an already-correct `2690--2693`) is left as given. A label
/// is alphanumeric with at least one digit, so `e1234-e1240` is a range too.
fn normalize_pages(pages: &str) -> String {
    let pages = pages.trim();
    let is_label = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric())
            && s.chars().any(|c| c.is_ascii_digit())
    };
    if let Some(at) = pages.find(['-', '\u{2013}']) {
        // Skip exactly the one separator; a second dash on the right (as in
        // `2690--2693` or `1-2-3`) fails `is_label`, so it is left alone.
        let sep = pages[at..].chars().next().map_or(0, char::len_utf8);
        let left = pages[..at].trim();
        let right = pages[at + sep..].trim();
        if is_label(left) && is_label(right) {
            return format!("{left}--{right}");
        }
    }
    pages.to_string()
}

/// Identifiers (DOI, arXiv id) are written verbatim, as every tool from
/// Crossref to Zotero does: `\doi`, `\href` and biblatex read the field raw
/// and an escaped `_` becomes a literal `\_` in the link. The one exception
/// is a brace, which would unbalance the file; an id holding one is escaped
/// so the file stays readable even if that one link does not.
fn identifier(id: &str) -> String {
    if id.contains(['{', '}']) {
        escape(id)
    } else {
        id.to_string()
    }
}

/// `authors` is the DTO's name for the field; the entry's is `author`.
fn field_label(field: &str) -> &str {
    if field == "authors" {
        "author"
    } else {
        field
    }
}

/// A string metadata field that holds something, trimmed.
fn text(field: &Option<MetaField<String>>) -> Option<&str> {
    field.as_ref().and_then(|f| nonblank(&f.value))
}

/// An optional string that holds something, trimmed.
fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().and_then(nonblank)
}

fn nonblank(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{MetaSource, PaperFile, PaperMeta};

    /// The default options, as every test before there were options.
    fn render_one(paper: &PaperDto) -> String {
        super::render_one(paper, &BibtexOptions::default())
    }

    fn render(papers: &[PaperDto], header: bool) -> String {
        super::render(papers, header, &BibtexOptions::default())
    }

    fn field<T>(value: T) -> Option<MetaField<T>> {
        Some(MetaField {
            value,
            source: MetaSource::Crossref,
        })
    }

    fn author(family: &str, given: &str) -> Author {
        Author {
            family: family.to_string(),
            given: given.to_string(),
        }
    }

    fn file(name: &str) -> PaperFile {
        PaperFile {
            path: format!("/papers/{name}"),
            name: name.to_string(),
            size: 1,
            mtime_ms: 0,
            fingerprint: None,
            missing: false,
            ..Default::default()
        }
    }

    fn paper(key: &str) -> PaperDto {
        PaperDto {
            id: format!("id-{key}"),
            files: vec![],
            doi: None,
            arxiv: None,
            meta: PaperMeta::default(),
            cite_key: key.to_string(),
            cite_key_pinned: false,
            project_ids: vec![],
            added_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            incomplete: vec![],
            isbn: None,
            is_paper: true,
            paper_reason: String::new(),
            paper_override: None,
            candidates: vec![],
            merged_count: 0,
            auto_merged: vec![],
            vouched: Default::default(),
            year_floor: None,
            year_hint: None,
            doi_source: None,
            arxiv_source: None,
            author_persons: vec![],
        }
    }

    fn jarzynski() -> PaperDto {
        let mut p = paper("jarzynski1997nonequilibrium");
        p.doi = Some("10.1103/PhysRevLett.78.2690".to_string());
        p.meta = PaperMeta {
            title: field("Nonequilibrium Equality for Free Energy Differences".to_string()),
            authors: field(vec![author("Jarzynski", "Christopher")]),
            year: field(1997),
            venue: field("Physical Review Letters".to_string()),
            volume: field("78".to_string()),
            issue: field("14".to_string()),
            pages: field("2690-2693".to_string()),
            publisher: field("American Physical Society".to_string()),
            abstract_: field("An equality is derived.".to_string()),
            kind: field(PaperKind::Article),
        };
        p
    }

    fn maldacena() -> PaperDto {
        let mut p = paper("maldacena1997large");
        p.arxiv = Some("hep-th/9711200".to_string());
        p.meta.title = field(
            "The Large N Limit of Superconformal Field Theories and Supergravity".to_string(),
        );
        p.meta.authors = field(vec![author("Maldacena", "Juan")]);
        p.meta.year = field(1997);
        p
    }

    #[test]
    fn article_with_every_field_is_byte_exact() {
        let expected = "\
@article{jarzynski1997nonequilibrium,
  author = {Jarzynski, Christopher},
  title = {{Nonequilibrium Equality for Free Energy Differences}},
  journal = {Physical Review Letters},
  year = {1997},
  volume = {78},
  number = {14},
  pages = {2690--2693},
  publisher = {American Physical Society},
  doi = {10.1103/PhysRevLett.78.2690}
}
";
        assert_eq!(render_one(&jarzynski()), expected);
    }

    #[test]
    fn inproceedings_uses_booktitle() {
        let mut p = paper("vaswani2017attention");
        p.doi = Some("10.5555/3295222.3295349".to_string());
        p.meta.kind = field(PaperKind::Inproceedings);
        p.meta.title = field("Attention Is All You Need".to_string());
        p.meta.authors = field(vec![author("Vaswani", "Ashish"), author("Shazeer", "Noam")]);
        p.meta.year = field(2017);
        p.meta.venue = field("Advances in Neural Information Processing Systems".to_string());
        p.meta.pages = field("5998–6008".to_string());
        assert_eq!(entry_type(&p), "inproceedings");
        assert_eq!(
            render_one(&p),
            "\
@inproceedings{vaswani2017attention,
  author = {Vaswani, Ashish and Shazeer, Noam},
  title = {{Attention Is All You Need}},
  booktitle = {Advances in Neural Information Processing Systems},
  year = {2017},
  pages = {5998--6008},
  doi = {10.5555/3295222.3295349}
}
"
        );
    }

    #[test]
    fn arxiv_only_is_misc_with_eprint() {
        let p = maldacena();
        assert_eq!(entry_type(&p), "misc");
        assert_eq!(
            render_one(&p),
            "\
@misc{maldacena1997large,
  author = {Maldacena, Juan},
  title = {{The Large N Limit of Superconformal Field Theories and Supergravity}},
  year = {1997},
  eprint = {hep-th/9711200},
  archivePrefix = {arXiv}
}
"
        );

        // A DOI or a venue means it is no longer arXiv-only: no eprint.
        let mut published = maldacena();
        published.doi = Some("10.1023/A:1026654312961".to_string());
        published.meta.venue =
            field("Advances in Theoretical and Mathematical Physics".to_string());
        let out = render_one(&published);
        assert_eq!(entry_type(&published), "article");
        assert!(out.contains("  doi = {10.1023/A:1026654312961}\n"));
        assert!(!out.contains("eprint"));
        assert!(!out.contains("archivePrefix"));
    }

    #[test]
    fn incomplete_entry_has_comment_line_and_note() {
        let mut p = paper("anonndscan");
        p.files = vec![file("scan_2024_final.pdf"), file("other.pdf")];
        p.meta.title = field("Scan 2024_final".to_string());
        p.incomplete = vec![
            "authors".to_string(),
            "year".to_string(),
            "venue".to_string(),
        ];
        assert_eq!(
            render_one(&p),
            "\
% incomplete: no author, no year, no venue
@misc{anonndscan,
  title = {{Scan 2024\\_final}},
  note = {Metadata incomplete; file scan\\_2024\\_final.pdf}
}
"
        );

        // No file at all: the note still says what is wrong.
        p.files.clear();
        assert!(render_one(&p).ends_with("  note = {Metadata incomplete}\n}\n"));

        // A complete paper gets neither.
        let out = render_one(&jarzynski());
        assert!(!out.contains("% incomplete"));
        assert!(!out.contains("note"));
    }

    #[test]
    fn escape_handles_every_special_and_passes_utf8_through() {
        assert_eq!(escape("A & B"), "A \\& B");
        assert_eq!(escape("100%"), "100\\%");
        assert_eq!(escape("$5"), "\\$5");
        assert_eq!(escape("#1"), "\\#1");
        assert_eq!(escape("a_b"), "a\\_b");
        assert_eq!(escape("{x}"), "\\textbraceleft{}x\\textbraceright{}");
        assert_eq!(escape("~"), "\\textasciitilde{}");
        assert_eq!(escape("^"), "\\textasciicircum{}");
        assert_eq!(escape("\\"), "\\textbackslash{}");
        // All together, in one string, in order.
        assert_eq!(
            escape("&%$#_{}~^\\"),
            "\\&\\%\\$\\#\\_\\textbraceleft{}\\textbraceright{}\\textasciitilde{}\\textasciicircum{}\\textbackslash{}"
        );
        // Non-ASCII is left alone.
        assert_eq!(escape("Ørsted"), "Ørsted");
        assert_eq!(escape("Schrödinger"), "Schrödinger");
        assert_eq!(escape("Łukasiewicz & 日本語"), "Łukasiewicz \\& 日本語");
        assert_eq!(escape(""), "");

        // Escaping reaches every text field, and the title's own braces stay
        // balanced inside the protecting double braces.
        let mut p = jarzynski();
        p.meta.title = field("H_2O & {CO2} at 100%".to_string());
        p.meta.authors = field(vec![
            author("Ørsted", "Hans Christian"),
            author("O'Brien_Smith", "P"),
        ]);
        p.meta.venue = field("J. Phys. $A$".to_string());
        let out = render_one(&p);
        assert!(out.contains(
            "  title = {{H\\_2O \\& \\textbraceleft{}CO2\\textbraceright{} at 100\\%}},\n"
        ));
        assert!(out.contains("  author = {Ørsted, Hans Christian and O'Brien\\_Smith, P},\n"));
        assert!(out.contains("  journal = {J. Phys. \\$A\\$},\n"));
    }

    #[test]
    fn identifiers_are_verbatim_unless_they_would_unbalance_the_file() {
        let mut p = jarzynski();
        p.doi = Some("10.1007/978-3-319-46448-0_2".to_string());
        assert!(render_one(&p).contains("  doi = {10.1007/978-3-319-46448-0_2}\n"));

        p.doi = Some("10.1/bad{id".to_string());
        assert!(render_one(&p).contains("  doi = {10.1/bad\\textbraceleft{}id}\n"));
    }

    #[test]
    fn author_formatting() {
        assert_eq!(format_authors(&[]), "");
        assert_eq!(
            format_authors(&[author("Jarzynski", "Christopher")]),
            "Jarzynski, Christopher"
        );
        assert_eq!(
            format_authors(&[author("Evans", "Denis J."), author("Cohen", "E. G. D.")]),
            "Evans, Denis J. and Cohen, E. G. D."
        );
        assert_eq!(
            format_authors(&[
                author("Evans", "Denis J."),
                author("Cohen", "E. G. D."),
                author("Morriss", "G. P."),
            ]),
            "Evans, Denis J. and Cohen, E. G. D. and Morriss, G. P."
        );
        // No given name: just the family name, no dangling comma.
        assert_eq!(
            format_authors(&[author("Plato", ""), author("van der Waals", "Johannes")]),
            "Plato and van der Waals, Johannes"
        );
        // No name at all is skipped rather than written as an empty author.
        assert_eq!(
            format_authors(&[author("", ""), author("Euler", "Leonhard")]),
            "Euler, Leonhard"
        );
        // Only a given name: written alone.
        assert_eq!(format_authors(&[author("", "Madonna")]), "Madonna");
    }

    #[test]
    fn pages_normalization() {
        assert_eq!(normalize_pages("2690-2693"), "2690--2693");
        assert_eq!(normalize_pages("2690–2693"), "2690--2693");
        assert_eq!(normalize_pages("2690 - 2693"), "2690--2693");
        assert_eq!(normalize_pages("e1234"), "e1234");
        assert_eq!(normalize_pages("e1234-e1240"), "e1234--e1240");
        assert_eq!(normalize_pages("2690--2693"), "2690--2693");
        assert_eq!(normalize_pages("12"), "12");
        assert_eq!(normalize_pages("-"), "-");
        assert_eq!(normalize_pages("2690-"), "2690-");
        assert_eq!(normalize_pages("1-2-3"), "1-2-3");
    }

    #[test]
    fn entry_type_follows_the_table_and_doi_goes_on_every_type() {
        // No kind: a venue makes an article, nothing makes misc.
        let mut p = paper("k");
        p.meta.venue = field("Nature".to_string());
        assert_eq!(entry_type(&p), "article");
        p.meta.venue = None;
        assert_eq!(entry_type(&p), "misc");
        p.meta.venue = field("   ".to_string());
        assert_eq!(entry_type(&p), "misc");

        // An explicit kind wins over the venue heuristic.
        p.meta.venue = field("Nature".to_string());
        p.meta.kind = field(PaperKind::Misc);
        assert_eq!(entry_type(&p), "misc");
        p.meta.kind = field(PaperKind::Article);
        p.meta.venue = None;
        assert_eq!(entry_type(&p), "article");

        // Book: publisher stays `publisher`.
        let mut book = paper("knuth1984texbook");
        book.doi = Some("10.0000/texbook".to_string());
        book.meta.kind = field(PaperKind::Book);
        book.meta.title = field("The TeXbook".to_string());
        book.meta.authors = field(vec![author("Knuth", "Donald E.")]);
        book.meta.year = field(1984);
        book.meta.publisher = field("Addison-Wesley".to_string());
        assert_eq!(
            render_one(&book),
            "\
@book{knuth1984texbook,
  author = {Knuth, Donald E.},
  title = {{The TeXbook}},
  year = {1984},
  publisher = {Addison-Wesley},
  doi = {10.0000/texbook}
}
"
        );

        // Thesis: the publisher is the school.
        let mut thesis = paper("shannon1940algebra");
        thesis.meta.kind = field(PaperKind::Thesis);
        thesis.meta.title = field("An Algebra for Theoretical Genetics".to_string());
        thesis.meta.authors = field(vec![author("Shannon", "Claude E.")]);
        thesis.meta.year = field(1940);
        thesis.meta.publisher = field("Massachusetts Institute of Technology".to_string());
        thesis.doi = Some("1721.1/11174".to_string());
        assert_eq!(
            render_one(&thesis),
            "\
@phdthesis{shannon1940algebra,
  author = {Shannon, Claude E.},
  title = {{An Algebra for Theoretical Genetics}},
  year = {1940},
  school = {Massachusetts Institute of Technology},
  doi = {1721.1/11174}
}
"
        );

        // An explicit misc with a venue keeps the venue as howpublished.
        let mut misc = paper("m");
        misc.meta.kind = field(PaperKind::Misc);
        misc.meta.title = field("Notes".to_string());
        misc.meta.venue = field("Lecture notes".to_string());
        assert!(render_one(&misc).contains("  howpublished = {Lecture notes}\n"));
    }

    #[test]
    fn render_joins_entries_in_order_with_one_blank_line_and_optional_header() {
        let papers = vec![maldacena(), jarzynski()];
        let a = render_one(&maldacena());
        let b = render_one(&jarzynski());

        let plain = render(&papers, false);
        assert_eq!(plain, format!("{a}\n{b}"));
        assert!(plain.ends_with("}\n"));
        assert!(!plain.ends_with("\n\n"));
        assert!(!plain.starts_with('%'));
        // Order is the caller's, not alphabetical.
        assert!(plain.find("maldacena").unwrap() < plain.find("jarzynski").unwrap());

        let with_header = render(&papers, true);
        assert_eq!(
            with_header,
            format!(
                "% Written by SpeedyF. Edits here are overwritten on the next sync.\n\n{a}\n{b}"
            )
        );
        // Exactly one blank line between entries: the only "\n\n" are the one
        // after the header and the one between the two entries.
        assert_eq!(with_header.matches("\n\n").count(), 2);
        assert_eq!(plain.matches("\n\n").count(), 1);

        // Reversing the input reverses the output.
        let reversed: Vec<PaperDto> = papers.iter().rev().cloned().collect();
        assert_eq!(render(&reversed, false), format!("{b}\n{a}"));

        // Nothing to write is nothing, or just the header.
        assert_eq!(render(&[], false), "");
        assert_eq!(
            render(&[], true),
            "% Written by SpeedyF. Edits here are overwritten on the next sync.\n"
        );

        // Same input, same bytes.
        assert_eq!(render(&papers, true), render(&papers, true));
    }

    #[test]
    fn each_option_changes_only_its_own_field() {
        let base = render_one(&jarzynski());
        let with = |change: fn(&mut BibtexOptions)| {
            let mut options = BibtexOptions::default();
            change(&mut options);
            super::render_one(&jarzynski(), &options)
        };
        // The default is the output from before there were options.
        assert_eq!(
            super::render_one(&jarzynski(), &BibtexOptions::default()),
            base
        );

        let abstract_ = with(|o| o.include_abstract = true);
        assert_eq!(
            abstract_,
            base.replace(
                "  doi = {10.1103/PhysRevLett.78.2690}\n}\n",
                "  doi = {10.1103/PhysRevLett.78.2690},\n  abstract = {An equality is derived.}\n}\n"
            )
        );

        let url = with(|o| o.include_url = true);
        assert!(url.ends_with(
            "  doi = {10.1103/PhysRevLett.78.2690},\n  url = {https://doi.org/10.1103/PhysRevLett.78.2690}\n}\n"
        ));
        let options = BibtexOptions {
            include_url: true,
            ..BibtexOptions::default()
        };
        assert!(super::render_one(&maldacena(), &options)
            .contains("  url = {https://arxiv.org/abs/hep-th/9711200}\n"));
        let mut bare = paper("bare");
        bare.meta.title = field("T".to_string());
        assert!(!super::render_one(&bare, &options).contains("url"));

        let mut filed = jarzynski();
        let mut missing = file("gone_1.pdf");
        missing.missing = true;
        filed.files = vec![missing, file("main_1.pdf")];
        let options = BibtexOptions {
            include_file: true,
            ..BibtexOptions::default()
        };
        assert!(super::render_one(&filed, &options).ends_with("  file = {/papers/main_1.pdf}\n}\n"));
        assert_eq!(
            super::render_one(&filed, &BibtexOptions::default()),
            base,
            "no file field by default"
        );

        let single = with(|o| o.protect_title = false);
        assert_eq!(
            single,
            base.replace(
                "{{Nonequilibrium Equality for Free Energy Differences}}",
                "{Nonequilibrium Equality for Free Energy Differences}"
            )
        );

        let short = with(|o| o.abbreviate_journals = true);
        assert_eq!(
            short,
            base.replace(
                "journal = {Physical Review Letters}",
                "journal = {Phys. Rev. Lett.}"
            )
        );
    }

    #[test]
    fn journal_names_abbreviate_from_the_table_only() {
        assert_eq!(
            abbreviate_journal("PHYSICAL  REVIEW letters"),
            Some("Phys. Rev. Lett.")
        );
        assert_eq!(
            abbreviate_journal("The Annals of Statistics"),
            Some("Ann. Statist.")
        );
        assert_eq!(
            abbreviate_journal("The Journal of Chemical Physics"),
            Some("J. Chem. Phys.")
        );
        assert_eq!(
            abbreviate_journal(
                "Proceedings of the National Academy of Sciences of the United States of America"
            ),
            Some("Proc. Natl. Acad. Sci. USA")
        );
        assert_eq!(abbreviate_journal("Physical Review"), None);
        assert_eq!(abbreviate_journal("Nature"), None);
        // An inproceedings venue is a booktitle, never abbreviated.
        let mut p = jarzynski();
        p.meta.kind = field(PaperKind::Inproceedings);
        let options = BibtexOptions {
            abbreviate_journals: true,
            ..BibtexOptions::default()
        };
        assert!(super::render_one(&p, &options).contains("booktitle = {Physical Review Letters}"));
        // The options round-trip in camelCase, and a missing key is its default.
        let json = serde_json::to_string(&options).unwrap();
        assert!(json.contains("\"abbreviateJournals\":true"), "{json}");
        let partial: BibtexOptions = serde_json::from_str("{\"includeUrl\":true}").unwrap();
        assert!(partial.include_url && partial.protect_title);
    }
}
