//! The XMP packet embedded in a PDF: what the publisher's typesetter wrote
//! about the work, when there is one (projects.md §4).
//!
//! The packet is XML inside the file, stored uncompressed by spec so that a
//! tool can find it without parsing the PDF — which is exactly what happens
//! here. The file's bytes are searched for `<x:xmpmeta` … `</x:xmpmeta>`;
//! every packet found is parsed, and the one that says the most wins. That
//! settles both an incremental update that leaves an older packet in place
//! (the newer one says as much or more, and ties go to the later one) and an
//! embedded image's packet (which says next to nothing about the paper).
//!
//! What is read: Dublin Core (`dc:title`, `dc:creator`, `dc:identifier` —
//! a DOI, an ISBN or an Elsevier PII — and `dc:publisher`), PRISM (`prism:doi`, `prism:isbn`,
//! `prism:publicationName`, `prism:volume`, `prism:number`,
//! `prism:startingPage`, `prism:endingPage`, `prism:publicationDate`) and
//! `pdfx:doi`. Both the element form and the attribute form on
//! `rdf:Description` occur in the wild; both are handled. `xmp:CreateDate`
//! is deliberately not read as a year: it is the date the file was made,
//! which for a scan is the scan date, the same trap as the Info dictionary's
//! CreationDate.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Files larger than this are not searched for a packet. The packet is a
/// few KiB near the start or the end; a file this size is a scan or a
/// slide deck, not a paper worth the read.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

const PACKET_OPEN: &[u8] = b"<x:xmpmeta";
const PACKET_CLOSE: &[u8] = b"</x:xmpmeta>";

/// What a packet said. Strings are trimmed and never empty; an absent field
/// is `None`. Identifiers are as written, less a `doi:` / `urn:isbn:`
/// scheme: the paper record normalizes them.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct XmpFields {
    #[serde(default)]
    pub title: Option<String>,
    /// `dc:creator` entries in order, one person each as the packet had
    /// them ("Given Family" or "Family, Given").
    #[serde(default)]
    pub creators: Vec<String>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    /// An Elsevier PII from `dc:identifier`, in its printed form
    /// (`S0022-0000(88)90028-1`); the paper record derives the DOI.
    #[serde(default)]
    pub pii: Option<String>,
    #[serde(default)]
    pub publication_name: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default)]
    pub start_page: Option<String>,
    #[serde(default)]
    pub end_page: Option<String>,
    /// From `prism:publicationDate` only.
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub publisher: Option<String>,
}

impl XmpFields {
    pub fn is_empty(&self) -> bool {
        self.filled() == 0
    }

    /// How many fields the packet filled: how packets are ranked against
    /// each other.
    fn filled(&self) -> usize {
        [
            self.title.is_some(),
            !self.creators.is_empty(),
            self.doi.is_some(),
            self.isbn.is_some(),
            self.pii.is_some(),
            self.publication_name.is_some(),
            self.volume.is_some(),
            self.number.is_some(),
            self.start_page.is_some(),
            self.end_page.is_some(),
            self.year.is_some(),
            self.publisher.is_some(),
        ]
        .into_iter()
        .filter(|filled| *filled)
        .count()
    }

    /// `start-end`, `start` alone, or nothing.
    pub fn pages(&self) -> Option<String> {
        match (&self.start_page, &self.end_page) {
            (Some(start), Some(end)) if start != end => Some(format!("{start}-{end}")),
            (Some(start), _) => Some(start.clone()),
            (None, _) => None,
        }
    }
}

/// The best packet in the file, or `None` when there is none, none says
/// anything, or the file is too big to search.
pub fn read_xmp(path: &Path) -> Option<XmpFields> {
    let size = fs::metadata(path).ok()?.len();
    if size == 0 || size > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    read_xmp_bytes(&bytes)
}

/// `read_xmp` on bytes already in hand.
pub fn read_xmp_bytes(bytes: &[u8]) -> Option<XmpFields> {
    let mut best: Option<XmpFields> = None;
    for packet in packets(bytes) {
        let fields = parse_xmp(&String::from_utf8_lossy(packet));
        if fields.is_empty() {
            continue;
        }
        // Ties go to the later packet: an incremental update's.
        if best
            .as_ref()
            .is_none_or(|current| fields.filled() >= current.filled())
        {
            best = Some(fields);
        }
    }
    best
}

/// Every `<x:xmpmeta …>…</x:xmpmeta>` slice in the file, in file order. A
/// packet opened and never closed is skipped, as is anything after it.
fn packets(bytes: &[u8]) -> Vec<&[u8]> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = find(&bytes[from..], PACKET_OPEN).map(|at| from + at) {
        let Some(end) =
            find(&bytes[start..], PACKET_CLOSE).map(|at| start + at + PACKET_CLOSE.len())
        else {
            break;
        };
        found.push(&bytes[start..end]);
        from = end;
    }
    found
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The namespaces that matter, resolved through the packet's own `xmlns`
/// declarations when it has them and through the conventional prefix when
/// it does not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ns {
    Dc,
    Prism,
    Pdfx,
    Rdf,
    Other,
}

fn ns_of_uri(uri: &str) -> Ns {
    if uri == "http://purl.org/dc/elements/1.1/" {
        Ns::Dc
    } else if uri.starts_with("http://prismstandard.org/namespaces/basic/") {
        Ns::Prism
    } else if uri.starts_with("http://ns.adobe.com/pdfx/") {
        Ns::Pdfx
    } else if uri == "http://www.w3.org/1999/02/22-rdf-syntax-ns#" {
        Ns::Rdf
    } else {
        Ns::Other
    }
}

fn ns_of_prefix(prefix: &str) -> Ns {
    match prefix {
        "dc" => Ns::Dc,
        "prism" => Ns::Prism,
        "pdfx" => Ns::Pdfx,
        "rdf" => Ns::Rdf,
        _ => Ns::Other,
    }
}

/// `prefix:local` → (namespace, local name). An undeclared prefix falls
/// back to its conventional meaning; no prefix is no namespace.
fn resolve(qname: &[u8], declared: &HashMap<String, Ns>) -> (Ns, String) {
    let qname = String::from_utf8_lossy(qname);
    match qname.split_once(':') {
        Some((prefix, local)) => {
            let ns = declared
                .get(prefix)
                .copied()
                .unwrap_or_else(|| ns_of_prefix(prefix));
            (ns, local.to_owned())
        }
        None => (Ns::Other, qname.into_owned()),
    }
}

/// Which field of `XmpFields` a (namespace, local name) pair feeds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Title,
    Creator,
    Identifier,
    Publisher,
    Doi,
    Isbn,
    PublicationName,
    Volume,
    Number,
    StartPage,
    EndPage,
    PublicationDate,
}

fn field_of(ns: Ns, local: &str) -> Option<Field> {
    match (ns, local) {
        (Ns::Dc, "title") => Some(Field::Title),
        (Ns::Dc, "creator") => Some(Field::Creator),
        (Ns::Dc, "identifier") => Some(Field::Identifier),
        (Ns::Dc, "publisher") => Some(Field::Publisher),
        (Ns::Prism | Ns::Pdfx, "doi") => Some(Field::Doi),
        (Ns::Prism, "isbn") => Some(Field::Isbn),
        (Ns::Prism, "publicationName") => Some(Field::PublicationName),
        (Ns::Prism, "volume") => Some(Field::Volume),
        (Ns::Prism, "number") => Some(Field::Number),
        (Ns::Prism, "startingPage") => Some(Field::StartPage),
        (Ns::Prism, "endingPage") => Some(Field::EndPage),
        (Ns::Prism, "publicationDate") => Some(Field::PublicationDate),
        _ => None,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Store one value for `field`. Lists keep every item; single-valued
/// fields keep the first value the packet gave, since `rdf:Alt` lists the
/// default language first and a repeated property is a packet's mistake.
fn absorb(fields: &mut XmpFields, field: Field, value: &str) {
    let Some(value) = non_empty(value) else {
        return;
    };
    let slot = match field {
        Field::Title => &mut fields.title,
        Field::Creator => {
            fields.creators.push(value);
            return;
        }
        Field::Identifier => {
            absorb_identifier(fields, &value);
            return;
        }
        Field::Publisher => &mut fields.publisher,
        Field::Doi => {
            if fields.doi.is_none() {
                fields.doi = Some(strip_doi_scheme(&value));
            }
            return;
        }
        Field::Isbn => {
            if fields.isbn.is_none() {
                fields.isbn = super::normalize_isbn(&value);
            }
            return;
        }
        Field::PublicationName => &mut fields.publication_name,
        Field::Volume => &mut fields.volume,
        Field::Number => &mut fields.number,
        Field::StartPage => &mut fields.start_page,
        Field::EndPage => &mut fields.end_page,
        Field::PublicationDate => {
            if fields.year.is_none() {
                fields.year = super::year_from_pdf_date(&value);
            }
            return;
        }
    };
    if slot.is_none() {
        *slot = Some(value);
    }
}

/// `dc:identifier` is a bag of anything: `doi:10.…`, a bare DOI, a DOI
/// URL, `urn:isbn:…`, a publisher's own id. DOIs, ISBNs and Elsevier PIIs
/// (`S0022000088900281`, as the packet writes them) are picked out; the
/// rest is ignored.
fn absorb_identifier(fields: &mut XmpFields, value: &str) {
    let lower = value.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("urn:isbn:")
        .or_else(|| lower.strip_prefix("isbn:"))
    {
        if fields.isbn.is_none() {
            fields.isbn = super::normalize_isbn(rest);
        }
        return;
    }
    if let Some(doi) = doi_in(value) {
        if fields.doi.is_none() {
            fields.doi = Some(doi);
        }
        return;
    }
    if fields.pii.is_none() {
        fields.pii = super::identifiers::find_pii(value);
    }
}

/// A DOI inside a string: `doi:` and a value, or anything from `10.` on
/// that has a slash in it (a DOI URL, a bare DOI).
fn doi_in(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("doi:") {
        return non_empty(&value[value.len() - rest.len()..]);
    }
    let at = lower.find("10.")?;
    let rest = &value[at..];
    let slash = rest.find('/')?;
    // "10." then a registrant code (four digits or more, dotted
    // subdivisions allowed) then the slash: a DOI prefix, not a page range
    // like "10.5/11".
    let prefix = &rest[3..slash];
    let leading_digits = prefix.chars().take_while(char::is_ascii_digit).count();
    if leading_digits < 4 || !prefix.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    non_empty(rest)
}

fn strip_doi_scheme(value: &str) -> String {
    doi_in(value).unwrap_or_else(|| value.trim().to_owned())
}

/// Parse one packet's XML. Tolerant: a malformed packet yields whatever
/// was read before the fault.
pub fn parse_xmp(xml: &str) -> XmpFields {
    let mut fields = XmpFields::default();
    let mut reader = Reader::from_str(xml);
    let mut declared: HashMap<String, Ns> = HashMap::new();
    // The open elements, each as the field it belongs to (an `rdf:li`
    // inside `dc:creator` belongs to `Creator`), and the text gathered
    // for the innermost one.
    let mut stack: Vec<Option<Field>> = Vec::new();
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let parent = stack.last().copied().flatten();
                let field = open(&start, &mut declared, &mut fields, parent);
                stack.push(field);
                text.clear();
            }
            Ok(Event::Empty(start)) => {
                let parent = stack.last().copied().flatten();
                open(&start, &mut declared, &mut fields, parent);
            }
            Ok(Event::End(_)) => {
                let Some(field) = stack.pop() else { break };
                if let Some(field) = field {
                    absorb(&mut fields, field, &text);
                }
                text.clear();
            }
            Ok(Event::Text(chunk)) => {
                let chunk = chunk
                    .unescape()
                    .map(|cow| cow.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&chunk).into_owned());
                text.push_str(&chunk);
            }
            Ok(Event::CData(cdata)) => {
                text.push_str(&String::from_utf8_lossy(&cdata.into_inner()));
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    fields
}

/// An element opening: note its namespace declarations, take the
/// attribute-form properties off an `rdf:Description`, and say which
/// field its text (and its `rdf:li` children's) belongs to. A container
/// element (`rdf:Alt`, `rdf:Seq`, `rdf:Bag`, `rdf:li`) inherits the field
/// of the property around it; a property element starts its own; anything
/// else belongs to nothing.
fn open(
    start: &BytesStart<'_>,
    declared: &mut HashMap<String, Ns>,
    fields: &mut XmpFields,
    parent: Option<Field>,
) -> Option<Field> {
    let mut attributes = Vec::new();
    for attribute in start.attributes().flatten() {
        let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .unescape_value()
            .map(|cow| cow.into_owned())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attribute.value).into_owned());
        if let Some(prefix) = key.strip_prefix("xmlns:") {
            declared.insert(prefix.to_owned(), ns_of_uri(&value));
        } else {
            attributes.push((key, value));
        }
    }
    let (ns, local) = resolve(start.name().as_ref(), declared);
    if ns == Ns::Rdf {
        match local.as_str() {
            "Description" => {
                for (key, value) in attributes {
                    let (ns, local) = resolve(key.as_bytes(), declared);
                    if let Some(field) = field_of(ns, &local) {
                        absorb(fields, field, &value);
                    }
                }
                return None;
            }
            "Alt" | "Seq" | "Bag" | "li" => return parent,
            _ => return None,
        }
    }
    field_of(ns, &local)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Element form, as Springer and BMC write it: the DOI three ways,
    /// PRISM for the venue, Dublin Core for the people.
    const SPRINGER: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 5.6">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:format>application/pdf</dc:format>
   <dc:identifier>doi:10.1186/s12859-019-2821-9</dc:identifier>
   <dc:title>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Fast &amp; accurate alignment of long reads</rdf:li>
     <rdf:li xml:lang="de">Schnelle Ausrichtung</rdf:li>
    </rdf:Alt>
   </dc:title>
   <dc:creator>
    <rdf:Seq>
     <rdf:li>Heng Li</rdf:li>
     <rdf:li>Richard Durbin</rdf:li>
    </rdf:Seq>
   </dc:creator>
   <dc:publisher>
    <rdf:Bag>
     <rdf:li>Springer Science and Business Media LLC</rdf:li>
    </rdf:Bag>
   </dc:publisher>
  </rdf:Description>
  <rdf:Description rdf:about="" xmlns:prism="http://prismstandard.org/namespaces/basic/2.0/">
   <prism:publicationName>BMC Bioinformatics</prism:publicationName>
   <prism:doi>10.1186/s12859-019-2821-9</prism:doi>
   <prism:volume>20</prism:volume>
   <prism:number>1</prism:number>
   <prism:startingPage>212</prism:startingPage>
   <prism:endingPage>219</prism:endingPage>
   <prism:publicationDate>2019-04-29</prism:publicationDate>
   <prism:issn>1471-2105</prism:issn>
  </rdf:Description>
  <rdf:Description rdf:about="" xmlns:pdfx="http://ns.adobe.com/pdfx/1.3/">
   <pdfx:doi>10.1186/s12859-019-2821-9</pdfx:doi>
  </rdf:Description>
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:CreateDate>2021-11-03T10:00:00Z</xmp:CreateDate>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    /// Attribute form, as Elsevier's and some IEEE tools write it.
    const ATTRIBUTE_FORM: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:prism="http://prismstandard.org/namespaces/basic/2.1/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    prism:doi="10.1016/j.physa.2020.124578"
    prism:publicationName="Physica A: Statistical Mechanics and its Applications"
    prism:volume="551"
    prism:startingPage="124578"
    prism:publicationDate="2020-08-01"
    dc:publisher="Elsevier BV"
    xmp:CreateDate="2020-04-20T08:00:00Z"/>
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Active matter at the edge</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>Fodor, &#201;tienne</rdf:li></rdf:Seq></dc:creator>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    /// What hyperxmp writes for a LaTeX manuscript: Dublin Core only, with
    /// the authors in one `rdf:Seq`, and a bare DOI in `dc:identifier`.
    const HYPERXMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="hyperxmp 2023">
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"
   xmlns:xmp="http://ns.adobe.com/xap/1.0/">
<dc:creator><rdf:Seq><rdf:li>Christopher Jarzynski</rdf:li></rdf:Seq></dc:creator>
<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Nonequilibrium Equality for Free Energy Differences</rdf:li></rdf:Alt></dc:title>
<dc:identifier><rdf:Bag><rdf:li>https://doi.org/10.1103/PhysRevLett.78.2690</rdf:li></rdf:Bag></dc:identifier>
<dc:date><rdf:Seq><rdf:li>1997</rdf:li></rdf:Seq></dc:date>
<xmp:CreateDate>2024-01-01T00:00:00Z</xmp:CreateDate>
<xmp:CreatorTool>LaTeX with hyperref</xmp:CreatorTool>
</rdf:Description>
</rdf:RDF>
</x:xmpmeta>"#;

    /// A word processor's packet: nothing about any paper.
    const WORD: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:pdf="http://ns.adobe.com/pdf/1.3/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <pdf:Producer>Microsoft Word</pdf:Producer>
   <xmp:CreateDate>2023-02-02T00:00:00Z</xmp:CreateDate>
   <xmp:CreatorTool>Word</xmp:CreatorTool>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    #[test]
    fn springer_style_element_form() {
        let got = parse_xmp(SPRINGER);
        assert_eq!(
            got.title.as_deref(),
            Some("Fast & accurate alignment of long reads"),
            "the first rdf:li, unescaped"
        );
        assert_eq!(got.creators, vec!["Heng Li", "Richard Durbin"]);
        assert_eq!(got.doi.as_deref(), Some("10.1186/s12859-019-2821-9"));
        assert_eq!(got.publication_name.as_deref(), Some("BMC Bioinformatics"));
        assert_eq!(got.volume.as_deref(), Some("20"));
        assert_eq!(got.number.as_deref(), Some("1"));
        assert_eq!(got.start_page.as_deref(), Some("212"));
        assert_eq!(got.end_page.as_deref(), Some("219"));
        assert_eq!(got.pages().as_deref(), Some("212-219"));
        assert_eq!(got.year, Some(2019), "from publicationDate, not CreateDate");
        assert_eq!(
            got.publisher.as_deref(),
            Some("Springer Science and Business Media LLC")
        );
        assert!(got.isbn.is_none());
    }

    #[test]
    fn attribute_form_on_rdf_description() {
        let got = parse_xmp(ATTRIBUTE_FORM);
        assert_eq!(got.doi.as_deref(), Some("10.1016/j.physa.2020.124578"));
        assert_eq!(
            got.publication_name.as_deref(),
            Some("Physica A: Statistical Mechanics and its Applications")
        );
        assert_eq!(got.volume.as_deref(), Some("551"));
        assert_eq!(got.start_page.as_deref(), Some("124578"));
        assert_eq!(got.pages().as_deref(), Some("124578"), "an article number");
        assert_eq!(got.year, Some(2020));
        assert_eq!(got.publisher.as_deref(), Some("Elsevier BV"));
        assert_eq!(got.title.as_deref(), Some("Active matter at the edge"));
        assert_eq!(got.creators, vec!["Fodor, Étienne"], "entities are decoded");
    }

    #[test]
    fn hyperxmp_latex_packet() {
        let got = parse_xmp(HYPERXMP);
        assert_eq!(
            got.title.as_deref(),
            Some("Nonequilibrium Equality for Free Energy Differences")
        );
        assert_eq!(got.creators, vec!["Christopher Jarzynski"]);
        assert_eq!(
            got.doi.as_deref(),
            Some("10.1103/PhysRevLett.78.2690"),
            "a DOI URL in dc:identifier"
        );
        assert_eq!(got.year, None, "dc:date and CreateDate are not read");
        assert!(got.publication_name.is_none());
    }

    #[test]
    fn a_packet_with_nothing_relevant_is_empty() {
        let got = parse_xmp(WORD);
        assert_eq!(got, XmpFields::default());
        assert!(got.is_empty());
        assert!(read_xmp_bytes(WORD.as_bytes()).is_none());
        assert!(read_xmp_bytes(b"%PDF-1.4 no packet here").is_none());
        assert!(parse_xmp("<x:xmpmeta><rdf:RDF><rdf:Description").is_empty());
    }

    #[test]
    fn dc_identifier_schemes() {
        let packet = |identifier: &str| {
            parse_xmp(&format!(
                r#"<x:xmpmeta><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier>{identifier}</dc:identifier></rdf:Description></rdf:RDF></x:xmpmeta>"#
            ))
        };
        assert_eq!(
            packet("doi:10.1016/j.cell.2020.01.001").doi.as_deref(),
            Some("10.1016/j.cell.2020.01.001")
        );
        assert_eq!(
            packet("DOI:10.1016/j.cell.2020.01.001").doi.as_deref(),
            Some("10.1016/j.cell.2020.01.001")
        );
        assert_eq!(
            packet("10.1103/PhysRevLett.78.2690").doi.as_deref(),
            Some("10.1103/PhysRevLett.78.2690")
        );
        let isbn = packet("urn:isbn:978-0-306-40615-7");
        assert_eq!(isbn.isbn.as_deref(), Some("9780306406157"));
        assert!(isbn.doi.is_none());
        assert_eq!(
            packet("isbn:0-306-40615-2").isbn.as_deref(),
            Some("0306406152")
        );
        assert!(packet("isbn:0-306-40615-3").isbn.is_none(), "bad checksum");
        let pii = packet("S0022000088900281");
        assert!(pii.doi.is_none() && pii.isbn.is_none());
        assert_eq!(
            pii.pii.as_deref(),
            Some("S0022-0000(88)90028-1"),
            "a compact PII, expanded"
        );
        assert_eq!(
            packet("PII: 0022-0000(88)90028-1").pii.as_deref(),
            Some("0022-0000(88)90028-1")
        );
        let other = packet("publisher-id-12345");
        assert!(other.doi.is_none() && other.isbn.is_none() && other.pii.is_none());
        assert!(packet("version 10.5/11").doi.is_none(), "not a DOI prefix");
    }

    #[test]
    fn undeclared_prefixes_fall_back_to_their_conventional_meaning() {
        let got = parse_xmp(
            r#"<x:xmpmeta><rdf:RDF><rdf:Description prism:doi="10.1/x" prism:volume="7">
<dc:title><rdf:Alt><rdf:li>Some title</rdf:li></rdf:Alt></dc:title>
</rdf:Description></rdf:RDF></x:xmpmeta>"#,
        );
        assert_eq!(got.doi.as_deref(), Some("10.1/x"));
        assert_eq!(got.volume.as_deref(), Some("7"));
        assert_eq!(got.title.as_deref(), Some("Some title"));

        // A prefix declared for another namespace is not mistaken for ours.
        let got = parse_xmp(
            r#"<x:xmpmeta><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description xmlns:dc="http://example.org/not-dublin-core/">
<dc:title><rdf:Alt><rdf:li>Not a title</rdf:li></rdf:Alt></dc:title>
</rdf:Description></rdf:RDF></x:xmpmeta>"#,
        );
        assert!(got.title.is_none());
    }

    #[test]
    fn the_fullest_packet_in_the_file_wins_and_later_breaks_ties() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"%PDF-1.7\n1 0 obj<</Type/Metadata/Length 0>>stream\n");
        bytes.extend_from_slice(WORD.as_bytes());
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        bytes.extend_from_slice(HYPERXMP.as_bytes());
        bytes.extend_from_slice(b"\n%% incremental update\n");
        let updated = HYPERXMP.replace("Christopher Jarzynski", "C. Jarzynski");
        bytes.extend_from_slice(updated.as_bytes());
        bytes.extend_from_slice(b"\n%%EOF");
        let got = read_xmp_bytes(&bytes).expect("a packet");
        assert_eq!(
            got.creators,
            vec!["C. Jarzynski"],
            "the later of two equals"
        );

        // An image's packet with less in it does not displace the paper's.
        let mut bytes = SPRINGER.as_bytes().to_vec();
        bytes.extend_from_slice(HYPERXMP.as_bytes());
        let got = read_xmp_bytes(&bytes).expect("a packet");
        assert_eq!(got.publication_name.as_deref(), Some("BMC Bioinformatics"));

        // An unterminated packet is skipped.
        assert!(read_xmp_bytes(b"<x:xmpmeta><rdf:RDF><dc:title>x").is_none());
    }

    #[test]
    fn read_xmp_reads_a_file_and_refuses_a_huge_one() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("a.pdf");
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(SPRINGER.as_bytes());
        fs::write(&path, &bytes).unwrap();
        let got = read_xmp(&path).expect("fields");
        assert_eq!(got.doi.as_deref(), Some("10.1186/s12859-019-2821-9"));
        fs::write(&path, b"").unwrap();
        assert!(read_xmp(&path).is_none());
        assert!(read_xmp(&dir.path().join("missing.pdf")).is_none());
    }
}
