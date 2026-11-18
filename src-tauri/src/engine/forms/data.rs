//! Form data files: FDF and XFDF (Acrobat's own), JSON and CSV.
//!
//! Only values travel: a field's fully qualified name and what it holds -
//! text, a button's export value ("Off" when off), or a list's selections.
//! The frontend says what each field is and applies imported values as
//! ordinary form edits; nothing here touches a PDF.

use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One field's value(s). `kind` is the field kind on export (it decides
/// how FDF writes the value) and empty on import.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormDataField {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub values: Vec<String>,
}

fn field(name: &str, values: Vec<String>) -> FormDataField {
    FormDataField {
        name: name.to_string(),
        kind: String::new(),
        values,
    }
}

/// The format a file name asks for.
fn format_of(path: &std::path::Path) -> AppResult<&'static str> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "fdf" => Ok("fdf"),
        "xfdf" => Ok("xfdf"),
        "json" => Ok("json"),
        "csv" => Ok("csv"),
        _ => Err(AppError::Unsupported(
            "form data must be an .fdf, .xfdf, .json or .csv file".into(),
        )),
    }
}

pub fn write_data_file(
    path: &std::path::Path,
    pdf_name: &str,
    fields: &[FormDataField],
) -> AppResult<()> {
    let bytes = match format_of(path)? {
        "fdf" => write_fdf(pdf_name, fields),
        "xfdf" => write_xfdf(pdf_name, fields).into_bytes(),
        "json" => write_json(fields).into_bytes(),
        _ => write_csv(fields).into_bytes(),
    };
    std::fs::write(path, bytes)?;
    Ok(())
}

pub fn read_data_file(path: &std::path::Path) -> AppResult<Vec<FormDataField>> {
    let format = format_of(path)?;
    let bytes = std::fs::read(path)?;
    let parsed = match format {
        "fdf" => parse_fdf(&bytes),
        "xfdf" => parse_xfdf(&String::from_utf8_lossy(&bytes)),
        "json" => parse_json(&String::from_utf8_lossy(&bytes)),
        _ => parse_csv(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}')),
    };
    parsed.ok_or_else(|| AppError::Malformed("not a form data file this reader understands".into()))
}

// ---------------------------------------------------------------------------
// Names as a tree
// ---------------------------------------------------------------------------

/// Fields nested by the parts of their names, as FDF and XFDF write them.
#[derive(Default)]
struct Node<'f> {
    value: Option<&'f FormDataField>,
    kids: BTreeMap<String, Node<'f>>,
    order: Vec<String>,
}

fn tree(fields: &[FormDataField]) -> Node<'_> {
    let mut root = Node::default();
    for f in fields {
        let mut node = &mut root;
        for part in f.name.split('.') {
            if !node.kids.contains_key(part) {
                node.order.push(part.to_string());
            }
            node = node.kids.entry(part.to_string()).or_default();
        }
        node.value = Some(f);
    }
    root
}

// ---------------------------------------------------------------------------
// FDF
// ---------------------------------------------------------------------------

/// A PDF string: a literal for printable ASCII, else UTF-16BE in hex.
fn pdf_string(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        let mut out = String::from("(");
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for unit in s.encode_utf16() {
            out.push_str(&format!("{unit:04X}"));
        }
        out.push('>');
        out
    }
}

fn pdf_name(s: &str) -> String {
    let mut out = String::from("/");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.+".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

fn fdf_value(f: &FormDataField) -> String {
    match f.kind.as_str() {
        "checkbox" | "radio" => pdf_name(f.values.first().map_or("Off", String::as_str)),
        _ if f.values.len() > 1 => {
            let items: Vec<String> = f.values.iter().map(|v| pdf_string(v)).collect();
            format!("[{}]", items.join(" "))
        }
        _ => pdf_string(f.values.first().map_or("", String::as_str)),
    }
}

fn fdf_fields(node: &Node, out: &mut String) {
    for name in &node.order {
        let kid = &node.kids[name];
        out.push_str("<< /T ");
        out.push_str(&pdf_string(name));
        if let Some(f) = kid.value {
            out.push_str(" /V ");
            out.push_str(&fdf_value(f));
        }
        if !kid.kids.is_empty() {
            out.push_str(" /Kids [");
            fdf_fields(kid, out);
            out.push(']');
        }
        out.push_str(" >>\n");
    }
}

pub fn write_fdf(pdf_name: &str, fields: &[FormDataField]) -> Vec<u8> {
    let mut body = String::from("1 0 obj\n<< /FDF << /Fields [\n");
    fdf_fields(&tree(fields), &mut body);
    body.push(']');
    if !pdf_name.is_empty() {
        body.push_str(" /F ");
        body.push_str(&pdf_string(pdf_name));
    }
    body.push_str(" >> >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
    let mut out = b"%FDF-1.2\n%\xE2\xE3\xCF\xD3\n".to_vec();
    out.extend_from_slice(body.as_bytes());
    out
}

/// The PDF objects an FDF file is made of.
#[derive(Clone, Debug, PartialEq)]
enum Pdf {
    Null,
    Bool(bool),
    Num(f64),
    Name(String),
    Str(Vec<u8>),
    Array(Vec<Pdf>),
    Dict(Vec<(String, Pdf)>),
    Ref(u32),
}

impl Pdf {
    fn get(&self, key: &str) -> Option<&Pdf> {
        match self {
            Pdf::Dict(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'%' => {
                    while self.i < self.s.len() && !matches!(self.s[self.i], b'\r' | b'\n') {
                        self.i += 1;
                    }
                }
                c if c.is_ascii_whitespace() || c == 0 => self.i += 1,
                _ => break,
            }
        }
    }

    fn delimiter(c: u8) -> bool {
        c.is_ascii_whitespace() || b"()<>[]{}/%".contains(&c)
    }

    fn word(&mut self) -> &[u8] {
        let start = self.i;
        while self.i < self.s.len() && !Self::delimiter(self.s[self.i]) {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    fn object(&mut self, depth: usize) -> Option<Pdf> {
        if depth > 64 {
            return None;
        }
        self.skip();
        let c = *self.s.get(self.i)?;
        match c {
            b'/' => {
                self.i += 1;
                let raw = self.word().to_vec();
                Some(Pdf::Name(decode_name(&raw)))
            }
            b'(' => self.literal().map(Pdf::Str),
            b'<' if self.s.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut entries = Vec::new();
                loop {
                    self.skip();
                    if self.s.get(self.i..self.i + 2) == Some(b">>") {
                        self.i += 2;
                        return Some(Pdf::Dict(entries));
                    }
                    let Pdf::Name(key) = self.object(depth + 1)? else {
                        return None;
                    };
                    let value = self.object(depth + 1)?;
                    entries.push((key, value));
                }
            }
            b'<' => {
                self.i += 1;
                let mut hex = Vec::new();
                while let Some(&c) = self.s.get(self.i) {
                    self.i += 1;
                    if c == b'>' {
                        break;
                    }
                    if c.is_ascii_hexdigit() {
                        hex.push(c);
                    }
                }
                if hex.len() % 2 == 1 {
                    hex.push(b'0');
                }
                let bytes = hex
                    .chunks(2)
                    .filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
                    .collect();
                Some(Pdf::Str(bytes))
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    if self.s.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Some(Pdf::Array(items));
                    }
                    items.push(self.object(depth + 1)?);
                }
            }
            _ => {
                let w = self.word().to_vec();
                if w.is_empty() {
                    self.i += 1;
                    return Some(Pdf::Null);
                }
                match w.as_slice() {
                    b"true" => Some(Pdf::Bool(true)),
                    b"false" => Some(Pdf::Bool(false)),
                    b"null" => Some(Pdf::Null),
                    _ => {
                        let n: f64 = std::str::from_utf8(&w).ok()?.parse().ok()?;
                        // "12 0 R" is a reference.
                        let save = self.i;
                        self.skip();
                        let generation = self.word().to_vec();
                        self.skip();
                        if !generation.is_empty()
                            && generation.iter().all(u8::is_ascii_digit)
                            && self.s.get(self.i) == Some(&b'R')
                            && self.s.get(self.i + 1).is_none_or(|c| Self::delimiter(*c))
                        {
                            self.i += 1;
                            return Some(Pdf::Ref(n as u32));
                        }
                        self.i = save;
                        Some(Pdf::Num(n))
                    }
                }
            }
        }
    }

    fn literal(&mut self) -> Option<Vec<u8>> {
        self.i += 1;
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(&c) = self.s.get(self.i) {
            self.i += 1;
            match c {
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.s.get(self.i) == Some(&b'\n') {
                                self.i += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.s.get(self.i) {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(out);
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        None
    }
}

fn decode_name(raw: &[u8]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'#' && i + 2 < raw.len() {
            if let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(&raw[i + 1..i + 3]), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A PDF text string's bytes as text: UTF-16BE with a BOM, else
/// PDFDocEncoding (taken as Latin-1, which agrees on everything printable).
fn text_of(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&bytes[3..]).into_owned()
    } else {
        bytes.iter().map(|b| *b as char).collect()
    }
}

fn parse_fdf(bytes: &[u8]) -> Option<Vec<FormDataField>> {
    if !bytes.starts_with(b"%FDF") {
        return None;
    }
    // Every "n g obj ... endobj".
    let mut objects: BTreeMap<u32, Pdf> = BTreeMap::new();
    let mut lx = Lexer { s: bytes, i: 0 };
    let mut root: Option<Pdf> = None;
    loop {
        lx.skip();
        if lx.i >= bytes.len() {
            break;
        }
        let start = lx.i;
        let w = lx.word().to_vec();
        if w == b"trailer" {
            let trailer = lx.object(0)?;
            root = trailer.get("Root").cloned();
            break;
        }
        if !w.is_empty() && w.iter().all(u8::is_ascii_digit) {
            lx.skip();
            let _generation = lx.word().to_vec();
            lx.skip();
            if lx.word() == b"obj" {
                let n: u32 = std::str::from_utf8(&w).ok()?.parse().ok()?;
                if let Some(object) = lx.object(0) {
                    objects.insert(n, object);
                }
                continue;
            }
        }
        if lx.i == start {
            lx.i += 1;
        }
    }
    let resolve = |o: &Pdf| -> Pdf {
        let mut o = o.clone();
        for _ in 0..16 {
            match o {
                Pdf::Ref(n) => o = objects.get(&n).cloned().unwrap_or(Pdf::Null),
                _ => break,
            }
        }
        o
    };
    let catalog = match root {
        Some(r) => resolve(&r),
        None => objects.values().find(|o| o.get("FDF").is_some())?.clone(),
    };
    let fdf = resolve(catalog.get("FDF")?);
    let fields = resolve(fdf.get("Fields")?);
    let mut out = Vec::new();
    fn walk(
        list: &Pdf,
        prefix: &str,
        resolve: &dyn Fn(&Pdf) -> Pdf,
        out: &mut Vec<FormDataField>,
        depth: usize,
    ) {
        let Pdf::Array(items) = list else {
            return;
        };
        if depth > 32 {
            return;
        }
        for item in items {
            let item = resolve(item);
            let t = match item.get("T").map(resolve) {
                Some(Pdf::Str(b)) => text_of(&b),
                Some(Pdf::Name(n)) => n,
                _ => String::new(),
            };
            let name = match (prefix.is_empty(), t.is_empty()) {
                (true, _) => t.clone(),
                (false, true) => prefix.to_string(),
                (false, false) => format!("{prefix}.{t}"),
            };
            if let Some(v) = item.get("V").map(resolve) {
                let values = match v {
                    Pdf::Str(b) => vec![text_of(&b)],
                    Pdf::Name(n) => vec![n],
                    Pdf::Num(n) => vec![format!("{n}")],
                    Pdf::Array(a) => a
                        .iter()
                        .map(resolve)
                        .filter_map(|x| match x {
                            Pdf::Str(b) => Some(text_of(&b)),
                            Pdf::Name(n) => Some(n),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                if !name.is_empty() {
                    out.push(field(&name, values));
                }
            }
            if let Some(kids) = item.get("Kids") {
                walk(&resolve(kids), &name, resolve, out, depth + 1);
            }
        }
    }
    walk(&fields, "", &resolve, &mut out, 0);
    Some(out)
}

// ---------------------------------------------------------------------------
// XFDF
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out
}

fn xfdf_fields(node: &Node, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for name in &node.order {
        let kid = &node.kids[name];
        out.push_str(&format!("{pad}<field name=\"{}\">\n", xml_escape(name)));
        if let Some(f) = kid.value {
            for v in &f.values {
                out.push_str(&format!("{pad}  <value>{}</value>\n", xml_escape(v)));
            }
            if f.values.is_empty() {
                out.push_str(&format!("{pad}  <value></value>\n"));
            }
        }
        xfdf_fields(kid, indent + 1, out);
        out.push_str(&format!("{pad}</field>\n"));
    }
}

pub fn write_xfdf(pdf_name: &str, fields: &[FormDataField]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n",
    );
    if !pdf_name.is_empty() {
        out.push_str(&format!("  <f href=\"{}\"/>\n", xml_escape(pdf_name)));
    }
    out.push_str("  <fields>\n");
    xfdf_fields(&tree(fields), 2, &mut out);
    out.push_str("  </fields>\n</xfdf>\n");
    out
}

fn parse_xfdf(text: &str) -> Option<Vec<FormDataField>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut names: Vec<String> = Vec::new();
    let mut values: Vec<Option<Vec<String>>> = Vec::new();
    let mut in_value = false;
    let mut text_buf = String::new();
    let mut out = Vec::new();
    let mut saw_root = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"xfdf" => saw_root = true,
                b"field" => {
                    let name = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == b"name")
                        .and_then(|a| a.unescape_value().ok())
                        .map(|v| v.into_owned())
                        .unwrap_or_default();
                    names.push(name);
                    values.push(None);
                }
                b"value" => {
                    in_value = true;
                    text_buf.clear();
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"xfdf" => saw_root = true,
                b"value" => {
                    if let Some(slot) = values.last_mut() {
                        slot.get_or_insert_with(Vec::new).push(String::new());
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_value => {
                text_buf.push_str(&t.unescape().ok()?);
            }
            Event::CData(t) if in_value => {
                text_buf.push_str(&String::from_utf8_lossy(&t));
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"value" => {
                    in_value = false;
                    if let Some(slot) = values.last_mut() {
                        slot.get_or_insert_with(Vec::new).push(text_buf.clone());
                    }
                }
                b"field" => {
                    let full = names.join(".");
                    names.pop();
                    if let Some(Some(v)) = values.pop() {
                        out.push(field(&full, v));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    saw_root.then_some(out)
}

// ---------------------------------------------------------------------------
// JSON and CSV
// ---------------------------------------------------------------------------

/// `{ "name": "value", "list": ["a", "b"] }`, in field order.
pub fn write_json(fields: &[FormDataField]) -> String {
    let mut map = serde_json::Map::new();
    for f in fields {
        let value = if f.values.len() == 1 {
            serde_json::Value::String(f.values[0].clone())
        } else {
            serde_json::Value::Array(
                f.values
                    .iter()
                    .map(|v| serde_json::Value::String(v.clone()))
                    .collect(),
            )
        };
        map.insert(f.name.clone(), value);
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(map)).unwrap_or_default() + "\n"
}

fn parse_json(text: &str) -> Option<Vec<FormDataField>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let as_text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b { "Yes" } else { "Off" }.to_string()),
        serde_json::Value::Null => Some(String::new()),
        _ => None,
    };
    Some(
        object
            .iter()
            .map(|(name, v)| {
                let values = match v {
                    serde_json::Value::Array(items) => items.iter().filter_map(as_text).collect(),
                    other => as_text(other).into_iter().collect(),
                };
                field(name, values)
            })
            .collect(),
    )
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One header row of field names and one row of values; several values
/// of one field are joined with ";".
pub fn write_csv(fields: &[FormDataField]) -> String {
    let header: Vec<String> = fields.iter().map(|f| csv_cell(&f.name)).collect();
    let row: Vec<String> = fields
        .iter()
        .map(|f| csv_cell(&f.values.join(";")))
        .collect();
    format!("{}\r\n{}\r\n", header.join(","), row.join(","))
}

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => cell.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

fn parse_csv(text: &str) -> Option<Vec<FormDataField>> {
    let rows = csv_rows(text);
    let header = rows.first()?;
    let values = rows.get(1).cloned().unwrap_or_default();
    Some(
        header
            .iter()
            .enumerate()
            .filter(|(_, name)| !name.is_empty())
            .map(|(i, name)| field(name, vec![values.get(i).cloned().unwrap_or_default()]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<FormDataField> {
        let f = |name: &str, kind: &str, values: &[&str]| FormDataField {
            name: name.into(),
            kind: kind.into(),
            values: values.iter().map(|v| v.to_string()).collect(),
        };
        vec![
            f("Name", "text", &["Ada (Countess), \"L\""]),
            f("topmostSubform.Address", "text", &["10 Main St\nFlat 2"]),
            f("topmostSubform.City", "text", &["Z\u{fc}rich \u{20ac}"]),
            f("Married", "checkbox", &["Yes"]),
            f("Colour", "radio", &["Off"]),
            f("Langs", "list", &["en", "fr"]),
        ]
    }

    fn strip(fields: Vec<FormDataField>) -> Vec<(String, Vec<String>)> {
        fields.into_iter().map(|f| (f.name, f.values)).collect()
    }

    #[test]
    fn fdf_round_trips() {
        let bytes = write_fdf("form.pdf", &sample());
        assert!(bytes.starts_with(b"%FDF-1.2"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Kids"), "names nest: {text}");
        assert!(text.contains("/V /Yes"));
        let back = parse_fdf(&bytes).unwrap();
        assert_eq!(strip(back), strip(sample()));
    }

    #[test]
    fn fdf_reads_acrobat_shapes() {
        let acrobat = b"%FDF-1.2\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<</FDF<</Fields[<</T(a)/V(x\\051y)>><</T(b)/Kids 2 0 R>>]/F(f.pdf)>>>>\nendobj\n2 0 obj\n[<</T(c)/V/On>>]\nendobj\ntrailer\n<</Root 1 0 R>>\n%%EOF\n";
        let back = strip(parse_fdf(acrobat).unwrap());
        assert_eq!(
            back,
            vec![
                ("a".to_string(), vec!["x)y".to_string()]),
                ("b.c".to_string(), vec!["On".to_string()]),
            ]
        );
        assert!(parse_fdf(b"%PDF-1.7").is_none());
    }

    #[test]
    fn xfdf_round_trips() {
        let xml = write_xfdf("form.pdf", &sample());
        assert!(xml.contains("<field name=\"topmostSubform\">"));
        let back = parse_xfdf(&xml).unwrap();
        assert_eq!(strip(back), strip(sample()));
        assert!(parse_xfdf("<html/>").is_none());
    }

    #[test]
    fn json_and_csv_round_trip() {
        let json = write_json(&sample());
        let mut back = strip(parse_json(&json).unwrap());
        let mut want = strip(sample());
        back.sort();
        want.sort();
        assert_eq!(back, want);
        let csv = write_csv(&sample());
        let back = strip(parse_csv(&csv).unwrap());
        assert_eq!(back.len(), 6);
        assert_eq!(back[0].1, vec!["Ada (Countess), \"L\"".to_string()]);
        assert_eq!(back[1].1, vec!["10 Main St\nFlat 2".to_string()]);
        assert_eq!(back[5].1, vec!["en;fr".to_string()]);
    }
}
