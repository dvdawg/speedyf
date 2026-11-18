//! `minimal_update` on hand-written files: an update that re-serialises
//! half the document while changing one field keeps only that field and
//! what it newly refers to.

use super::*;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

const CONTENT: &[u8] = b"BT /F1 12 Tf 20 370 Td (Signed text) Tj ET";

/// Catalog with an inline AcroForm, one page with a content stream and a
/// text field widget, a font.
fn source() -> Vec<u8> {
    let objects: Vec<(u32, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Contents 4 0 R /Annots [5 0 R] /Resources << /Font << /F1 6 0 R >> >> >>".to_vec()),
        (4, stream_body(b"", CONTENT)),
        (5, b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Before) /Rect [40 300 250 320] /P 3 0 R >>".to_vec()),
        (6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
    ];
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    append_section(&mut out, &objects, None, 7);
    out
}

fn stream_body(extra: &[u8], data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {}", data.len()).into_bytes();
    body.extend_from_slice(extra);
    body.extend_from_slice(b" >>\nstream\n");
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn startxref(bytes: &[u8]) -> usize {
    let at = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
    std::str::from_utf8(&bytes[at + 9..])
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Append objects and a classic cross-reference section, one subsection
/// per object, chained to the previous one when there is one.
fn append_section(out: &mut Vec<u8>, objects: &[(u32, Vec<u8>)], prev: Option<usize>, size: u32) {
    let mut offsets = Vec::new();
    for (n, body) in objects {
        offsets.push((*n, out.len()));
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let start = out.len();
    out.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
    for (n, at) in offsets {
        out.extend_from_slice(format!("{n} 1\n{at:010} 00000 n \n").as_bytes());
    }
    let prev = prev.map(|p| format!(" /Prev {p}")).unwrap_or_default();
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R{prev} >>\nstartxref\n{start}\n%%EOF\n")
            .as_bytes(),
    );
}

/// What PDFium appends for a form fill: the catalog, page tree and page
/// written back as they were (with hex strings and other spacing), the page
/// content re-compressed, the widget changed and pointing at a new
/// appearance, and a resource dictionary nothing uses.
fn pdfium_like_update(source: &[u8], n: u32) -> Vec<u8> {
    let mut out = source.to_vec();
    let objects: Vec<(u32, Vec<u8>)> = vec![
        (1, b"<</AcroForm<</Fields[ 5 0 R ]>>/Pages 2 0 R /Type/Catalog>>".to_vec()),
        (2, b"<</Count 1/Kids[ 3 0 R ]/Type/Pages>>".to_vec()),
        (3, b"<</Annots[ 5 0 R ]/Contents 4 0 R /MediaBox[ 0 0 300.0 400]/Parent 2 0 R /Resources<</Font<</F1 6 0 R >>>>/Type/Page>>".to_vec()),
        (4, stream_body(b" /Filter /FlateDecode", &deflate(CONTENT))),
        (5, format!("<</AP<</N {n} 0 R >>/FT/Tx/P 3 0 R /Rect[ 40 300 250 320]/Subtype/Widget/T<6e616d65>/Type/Annot/V(After)>>").into_bytes()),
        (6, b"<</BaseFont/Helvetica/Subtype/Type1/Type/Font>>".to_vec()),
        (n, stream_body(format!(" /Type /XObject /Subtype /Form /BBox [0 0 210 20] /Resources {} 0 R", n + 2).as_bytes(), b"/Tx BMC BT /F1 10 Tf 2 5 Td (After) Tj ET EMC")),
        (n + 1, b"<< /Unused true >>".to_vec()),
        (n + 2, b"<< /Font << /F1 6 0 R >> >>".to_vec()),
    ];
    let prev = startxref(source);
    append_section(&mut out, &objects, Some(prev), n + 3);
    out
}

/// The /Size the file's last trailer declares.
fn declared_size(bytes: &[u8]) -> u32 {
    lopdf::Document::load_mem(bytes)
        .unwrap()
        .trailer
        .get(b"Size")
        .unwrap()
        .as_i64()
        .unwrap() as u32
}

/// Object numbers written in `bytes[from..]`.
fn objects_written(bytes: &[u8], from: usize) -> Vec<u32> {
    let text = String::from_utf8_lossy(&bytes[from..]);
    let tokens: Vec<&str> = text.split_ascii_whitespace().collect();
    let mut found: Vec<u32> = tokens
        .windows(3)
        .filter(|w| w[2] == "obj" && w[1].parse::<u32>().is_ok())
        .filter_map(|w| w[0].parse().ok())
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn strings_numbers_key_order_and_compression_do_not_count_as_changes() {
    use lopdf::{Dictionary, Object, Stream, StringFormat};
    let lit = Object::String(b"name".to_vec(), StringFormat::Literal);
    let hex = Object::String(b"name".to_vec(), StringFormat::Hexadecimal);
    assert!(equivalent(&lit, &hex));
    assert!(equivalent(&Object::Integer(300), &Object::Real(300.0)));
    assert!(!equivalent(&Object::Integer(300), &Object::Real(300.5)));
    let mut a = Dictionary::new();
    a.set("A", 1);
    a.set("B", lit.clone());
    let mut b = Dictionary::new();
    b.set("B", hex);
    b.set("A", 1);
    assert!(equivalent(
        &Object::Dictionary(a.clone()),
        &Object::Dictionary(b)
    ));
    a.set("C", true);
    let mut c = Dictionary::new();
    c.set("A", 1);
    c.set("B", lit);
    assert!(!equivalent(&Object::Dictionary(a), &Object::Dictionary(c)));

    let plain = Stream::new(Dictionary::new(), CONTENT.to_vec());
    let mut packed = Stream::new(Dictionary::new(), deflate(CONTENT));
    packed
        .dict
        .set("Filter", Object::Name(b"FlateDecode".to_vec()));
    assert!(equivalent(
        &Object::Stream(plain.clone()),
        &Object::Stream(packed)
    ));
    let other = Stream::new(Dictionary::new(), b"BT ET".to_vec());
    assert!(!equivalent(&Object::Stream(plain), &Object::Stream(other)));
}

#[test]
fn keeps_only_the_changed_widget_and_what_it_newly_refers_to() {
    let source = source();
    let output = pdfium_like_update(&source, 7);
    assert_eq!(
        objects_written(&output, source.len()),
        (1..=9).collect::<Vec<_>>()
    );

    let minimal = minimal_update(&source, &output)
        .unwrap()
        .expect("an update");
    assert_eq!(&minimal[..source.len()], &source[..], "source is a prefix");
    assert_eq!(
        objects_written(&minimal, source.len()),
        vec![5, 7, 9],
        "the widget, its new appearance and that appearance's resources"
    );

    let doc = lopdf::Document::load_mem(&minimal).unwrap();
    let widget = doc.get_dictionary((5, 0)).unwrap();
    assert_eq!(widget.get(b"V").unwrap().as_str().unwrap(), b"After");
    assert!(doc.get_object((7, 0)).unwrap().as_stream().is_ok());
    assert!(doc.get_object((8, 0)).is_err(), "the unused object is gone");
    // The page content is the signed revision's own, uncompressed bytes.
    let content = doc.get_object((4, 0)).unwrap().as_stream().unwrap();
    assert!(!content.dict.has(b"Filter"));
    assert_eq!(content.content, CONTENT);
    let tail = String::from_utf8_lossy(&minimal[source.len()..]).to_string();
    assert!(
        tail.contains(&format!("/Prev {}", startxref(&source))),
        "chained to the source: {tail}"
    );
}

#[test]
fn an_update_that_changed_nothing_leaves_the_source() {
    let source = source();
    let mut output = source.clone();
    let objects: Vec<(u32, Vec<u8>)> = vec![(
        1,
        b"<</AcroForm<</Fields[ 5 0 R ]>>/Pages 2 0 R /Type/Catalog>>".to_vec(),
    )];
    append_section(&mut output, &objects, Some(startxref(&source)), 7);
    assert_eq!(minimal_update(&source, &output).unwrap(), Some(source));
}

#[test]
fn a_file_that_does_not_extend_the_source_is_left_alone() {
    let source = source();
    let mut other = source.clone();
    other[20] ^= 1;
    other.extend_from_slice(b"\n");
    assert_eq!(minimal_update(&source, &other).unwrap(), None);
    assert_eq!(minimal_update(&source, &source).unwrap(), None);
}

/// Appending to a file whose cross-reference data is a stream writes a
/// stream too, numbered past everything, and lopdf reads the chain back.
#[test]
fn a_stream_xref_source_gets_a_well_formed_stream_update() {
    let mut doc = lopdf::Document::load_mem(&source()).unwrap();
    let mut modern = Vec::new();
    doc.save_modern(&mut modern).unwrap();
    let n = declared_size(&modern);
    let output = pdfium_like_update(&modern, n);
    let minimal = minimal_update(&modern, &output)
        .unwrap()
        .expect("an update");
    assert_eq!(&minimal[..modern.len()], &modern[..]);
    let written = objects_written(&minimal, modern.len());
    assert_eq!(&written[..3], &[5, n, n + 2]);
    assert_eq!(written.len(), 4, "plus the cross-reference stream itself");
    assert!(written[3] > n + 2);
    let tail = String::from_utf8_lossy(&minimal[modern.len()..]).to_string();
    assert!(tail.contains("/Type/XRef") || tail.contains("/Type /XRef"));
    let reread = lopdf::Document::load_mem(&minimal).unwrap();
    assert_eq!(
        reread
            .get_dictionary((5, 0))
            .unwrap()
            .get(b"V")
            .unwrap()
            .as_str()
            .unwrap(),
        b"After"
    );
    assert_eq!(reread.get_pages().len(), 1);
}
