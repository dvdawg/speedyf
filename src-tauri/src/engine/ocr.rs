//! OCR for pages whose text layer is missing or garbage (old scanned papers).
//!
//! macOS: Apple's Vision framework (`VNRecognizeTextRequest`, accurate level,
//! language correction on) — on-device, no model to ship, and good on the
//! 300 dpi-ish journal scans this is for. Every other platform compiles
//! [`recognize_png`] to a no-op returning `None`; [`needs_ocr`] and
//! [`page_text`] are pure Rust and work everywhere.
//!
//! The caller renders the page itself (PDFium, ~2x) and hands over PNG bytes,
//! so this module knows nothing about PDFs. `recognize_png` is synchronous and
//! safe to call from the PDFium worker thread: Vision does not need the main
//! thread, and the whole call runs inside its own autorelease pool so nothing
//! piles up on a thread that never drains one.

/// One recognized line of text.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OcrLine {
    pub text: String,
    /// Vision's confidence for the top candidate, 0..=1.
    pub confidence: f32,
    /// Normalized to the image (0..1), top-left origin: `[x, y, w, h]`.
    pub bbox: [f32; 4],
}

/// Lines below this confidence are dropped by [`page_text`]. Vision gives
/// 0.3 for its "best guess on a smudge" and ≥0.5 for anything real; 0.3 keeps
/// faint-but-correct lines and drops specks read as `.` or `~`.
pub const MIN_LINE_CONFIDENCE: f32 = 0.3;

/// Whether this build can OCR at all. Callers check it before rendering a
/// page they would only hand to [`recognize_png`] to get `None` back.
pub const fn available() -> bool {
    cfg!(target_os = "macos")
}

/// Run text recognition on a PNG (any format ImageIO decodes works).
///
/// `languages` are BCP-47 codes such as `"en-US"`; empty means `["en-US"]`.
/// An unsupported code makes Vision fail the request, which surfaces as
/// `None`. Returns lines in reading order (top-to-bottom; lines sharing a row
/// left-to-right), or `None` on any failure. `Some(vec![])` means the image
/// decoded fine and simply has no text.
pub fn recognize_png(png: &[u8], languages: &[&str]) -> Option<Vec<OcrLine>> {
    if png.is_empty() {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let mut lines = vision::recognize(png, languages)?;
        sort_reading_order(&mut lines);
        Some(lines)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = languages;
        None
    }
}

/// The page's text as one string: lines joined with `\n`, dropping lines
/// under [`MIN_LINE_CONFIDENCE`] and lines that are only whitespace.
pub fn page_text(lines: &[OcrLine]) -> String {
    let mut out = String::new();
    for line in lines {
        if line.confidence < MIN_LINE_CONFIDENCE || line.text.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line.text.trim_end());
    }
    out
}

// --- needs_ocr thresholds -------------------------------------------------
//
// Calibrated against Elsevier scans from the 1980s/90s (Acrobat Capture text
// layers) and against clean born-digital papers, which must never trip.

/// (a) A full page with fewer non-whitespace characters than this, that has
/// images on it, is an image-only scan (or a scan with a stray page number).
/// Scaled down for small pages; see [`min_chars_for_area`].
const MIN_CHARS_FULL_PAGE: f32 = 40.0;
/// US Letter, 612 × 792 pt — the area at which `MIN_CHARS_FULL_PAGE` applies.
const LETTER_AREA_PTS: f32 = 612.0 * 792.0;
/// Floor for the scaled minimum, so a tiny page still needs *some* text.
const MIN_CHARS_FLOOR: f32 = 10.0;

/// Garbage checks only run with at least this much text; below it ratios are
/// noise (a page reading just "254" is not garbage, it's a page number).
const MIN_CHARS_FOR_RATIOS: usize = 12;
/// (b1) Letters / non-whitespace chars. English prose is ~0.80–0.85, a
/// formula-heavy page ~0.60–0.70; misread scans ("~ ,;l1 Il") fall to 0.2–0.4.
const MIN_LETTER_SHARE: f32 = 0.55;
/// (b2) Share of whitespace-separated tokens that are 1–2 chars long. Prose
/// sits at 0.20–0.30 (a, of, to, in, is…); a scan read as confetti is > 0.5.
const MAX_SHORT_TOKEN_SHARE: f32 = 0.5;
/// Need at least this many tokens before (b2) counts.
const MIN_TOKENS_FOR_SHORT_SHARE: usize = 8;
/// (b3) Share of 4+ letter runs that contain no vowel (a, e, i, o, u, y;
/// any non-ASCII letter counts as a vowel so other scripts never trip).
/// English has almost none outside acronyms (HTTP, NP-hard's "NP" is too
/// short to count); OCR noise like "tJlrn" / "Wllh" is full of them.
const MAX_VOWELLESS_SHARE: f32 = 0.35;
/// Need at least this many 4+ letter runs before (b3) counts.
const MIN_RUNS_FOR_VOWEL_CHECK: usize = 5;

/// Whether a page's existing text layer is unusable and the page should be
/// OCR'd instead.
///
/// - `extracted_text`: what PDFium's text layer gave for the page.
/// - `page_area_pts`: page width × height in PDF points (0 or negative =
///   unknown, treated as Letter).
/// - `has_images`: the page has at least one image object (PDFium
///   `FPDF_PAGEOBJ_IMAGE`). Distinguishes "scanned page with no text layer"
///   from "intentionally blank page": an empty page *without* images is left
///   alone, since there is nothing to read.
///
/// Rules (any one triggers):
/// - (a) fewer than ~40 non-whitespace chars (scaled by page area, floor 10)
///   on a page with images;
/// - (b) with ≥ 12 non-whitespace chars, garbage-shaped text:
///   letter share < 0.55, or > 50 % of tokens are 1–2 chars long (≥ 8
///   tokens), or > 35 % of 4+ letter runs have no vowel (≥ 5 runs).
pub fn needs_ocr(extracted_text: &str, page_area_pts: f32, has_images: bool) -> bool {
    let non_ws: Vec<char> = extracted_text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if has_images && (non_ws.len() as f32) < min_chars_for_area(page_area_pts) {
        return true;
    }
    if non_ws.len() < MIN_CHARS_FOR_RATIOS {
        return false;
    }

    let letters = non_ws.iter().filter(|c| c.is_alphabetic()).count();
    if (letters as f32) / (non_ws.len() as f32) < MIN_LETTER_SHARE {
        return true;
    }

    let tokens: Vec<&str> = extracted_text.split_whitespace().collect();
    if tokens.len() >= MIN_TOKENS_FOR_SHORT_SHARE {
        let short = tokens.iter().filter(|t| t.chars().count() <= 2).count();
        if (short as f32) / (tokens.len() as f32) > MAX_SHORT_TOKEN_SHARE {
            return true;
        }
    }

    let (runs, vowelless) = vowelless_runs(extracted_text);
    runs >= MIN_RUNS_FOR_VOWEL_CHECK && (vowelless as f32) / (runs as f32) > MAX_VOWELLESS_SHARE
}

fn min_chars_for_area(page_area_pts: f32) -> f32 {
    if !page_area_pts.is_finite() || page_area_pts <= 0.0 {
        return MIN_CHARS_FULL_PAGE;
    }
    (MIN_CHARS_FULL_PAGE * (page_area_pts / LETTER_AREA_PTS).min(1.0)).max(MIN_CHARS_FLOOR)
}

/// Counts maximal runs of ≥ 4 letters, and how many of those have no vowel.
fn vowelless_runs(text: &str) -> (usize, usize) {
    let (mut runs, mut vowelless) = (0usize, 0usize);
    let mut len = 0usize;
    let mut has_vowel = false;
    let mut close = |len: usize, has_vowel: bool| {
        if len >= 4 {
            runs += 1;
            if !has_vowel {
                vowelless += 1;
            }
        }
    };
    for c in text.chars() {
        if c.is_alphabetic() {
            len += 1;
            has_vowel |= !c.is_ascii()
                || matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        } else {
            close(len, has_vowel);
            len = 0;
            has_vowel = false;
        }
    }
    close(len, has_vowel);
    (runs, vowelless)
}

/// Top-to-bottom, and left-to-right within a row. A line joins the current
/// row when its vertical center falls inside the row's first line, so a
/// two-column header ("Received …" | "Accepted …") stays left before right
/// even when the two boxes differ by a pixel in y.
fn sort_reading_order(lines: &mut Vec<OcrLine>) {
    lines.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    let mut out = Vec::with_capacity(lines.len());
    let mut row: Vec<OcrLine> = Vec::new();
    for line in lines.drain(..) {
        if let Some(first) = row.first() {
            let center = line.bbox[1] + line.bbox[3] / 2.0;
            if center > first.bbox[1] + first.bbox[3] {
                row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
                out.append(&mut row);
            }
        }
        row.push(line);
    }
    row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    out.append(&mut row);
    *lines = out;
}

#[cfg(target_os = "macos")]
mod vision {
    use super::OcrLine;
    use objc2::rc::autoreleasepool;
    use objc2::AllocAnyThread;
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    pub(super) fn recognize(png: &[u8], languages: &[&str]) -> Option<Vec<OcrLine>> {
        // Vision and Foundation autorelease intermediates (the decoded image,
        // result arrays). The worker thread has no pool of its own, so drain
        // one per call.
        autoreleasepool(|_| recognize_in_pool(png, languages))
    }

    fn recognize_in_pool(png: &[u8], languages: &[&str]) -> Option<Vec<OcrLine>> {
        // NSData copies the bytes; Vision decodes them itself (CIImage /
        // ImageIO), so a corrupt PNG becomes a perform error, not a crash.
        let data = NSData::with_bytes(png);
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &NSDictionary::new(),
        );

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let langs: Vec<_> = if languages.is_empty() {
            vec![NSString::from_str("en-US")]
        } else {
            languages.iter().map(|l| NSString::from_str(l)).collect()
        };
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&langs));

        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|e| log::debug!("ocr: Vision request failed: {e:?}"))
            .ok()?;

        let observations = request.results()?;
        let mut lines = Vec::with_capacity(observations.count());
        for obs in observations.iter() {
            let Some(top) = obs.topCandidates(1).firstObject() else {
                continue;
            };
            // SAFETY: `boundingBox` is a plain property getter returning a
            // CGRect by value on a live, fully-initialized observation; the
            // binding is only `unsafe` because it is generated from a header
            // without nullability/thread annotations.
            let r = unsafe { obs.boundingBox() };
            // Vision: normalized, bottom-left origin → ours: top-left origin.
            let (x, w, h) = (r.origin.x as f32, r.size.width as f32, r.size.height as f32);
            let y = 1.0 - (r.origin.y as f32 + h);
            lines.push(OcrLine {
                text: top.string().to_string(),
                confidence: top.confidence(),
                bbox: [x, y.clamp(0.0, 1.0), w, h],
            });
        }
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, confidence: f32, bbox: [f32; 4]) -> OcrLine {
        OcrLine {
            text: text.into(),
            confidence,
            bbox,
        }
    }

    const LETTER: f32 = LETTER_AREA_PTS;
    /// The JCSS scan's page size (453 × 669 pt).
    const JCSS: f32 = 453.28 * 669.0;

    const CLEAN_ABSTRACT: &str =
        "One can view NP as the complexity class that captures the notion of \
        efficient provability by classical (formal) proofs. We consider broader complexity classes \
        (still \u{201c}just above NP\u{201d}), in the hope to formalize the notion of efficient \
        provability by overwhelming statistical evidence.";

    #[test]
    fn running_head_with_one_misread_word_is_fine() {
        // Real Acrobat Capture output: "SCIENCES" read as "KXNCES". Mostly fine.
        let t = "JOURNAL OF COMPUTER AND SYSTEM KXNCES 36";
        assert!(!needs_ocr(t, JCSS, true));
        assert!(!needs_ocr(t, JCSS, false));
    }

    #[test]
    fn confetti_triggers() {
        let t = "~ ,;l1 Il ~ ' .. ,; 1~ I";
        assert!(needs_ocr(t, JCSS, true));
        // Letter share alone catches it, images or not.
        let longer = "~ ,;l1 Il ~ ' .. ,; 1~ I ~ ,;l1 Il ~ ' .. ,; 1~ I";
        assert!(needs_ocr(longer, JCSS, false));
    }

    #[test]
    fn clean_prose_does_not_trigger() {
        assert!(!needs_ocr(CLEAN_ABSTRACT, LETTER, true));
        assert!(!needs_ocr(CLEAN_ABSTRACT, JCSS, false));
    }

    #[test]
    fn formula_heavy_but_real_text_does_not_trigger() {
        let t = "where t(n) is the number of rounds of interaction on inputs of length n. \
            The Collapse Theorem states that for t(n) > 2, AM(t(n) + 1) = AM(t(n)). \
            In particular, the finite levels of the hierarchy collapse to AM = AM(2).";
        assert!(!needs_ocr(t, JCSS, true));
    }

    #[test]
    fn empty_scanned_page_triggers_blank_page_does_not() {
        assert!(needs_ocr("", JCSS, true));
        assert!(needs_ocr("  254 \n", JCSS, true)); // just a page number
        assert!(!needs_ocr("", JCSS, false));
        assert!(!needs_ocr("  254 \n", JCSS, false));
    }

    #[test]
    fn min_chars_scale_with_page_area() {
        let thirty = "abcdefghij abcdefghij abcdefghij"; // 30 non-ws chars, fine prose-shape
        assert!(needs_ocr(thirty, LETTER, true));
        // A quarter-letter page (a slide thumbnail, an index card) needs 10.
        assert!(!needs_ocr(thirty, LETTER / 4.0, true));
        // Unknown area behaves like Letter.
        assert!(needs_ocr(thirty, 0.0, true));
        assert_eq!(min_chars_for_area(f32::NAN), MIN_CHARS_FULL_PAGE);
        assert_eq!(min_chars_for_area(1.0), MIN_CHARS_FLOOR);
    }

    #[test]
    fn mostly_short_tokens_trigger() {
        // Letters dominate, but it's been shredded into 1–2 char pieces.
        let t = "Th e qu ic k br ow n fo x ju mp s ov er th e la zy do g an d ru ns";
        assert!(needs_ocr(t, JCSS, true));
    }

    #[test]
    fn vowelless_tokens_trigger() {
        let t = "Thc rcsnlt nnd thcrcfrc wlth Wllh tJlrn prcvf frrnl cxtcnsl proofs follow";
        assert!(needs_ocr(t, JCSS, false));
        // A couple of acronyms in otherwise normal prose do not.
        let ok = "We implement the HTTP and SMTP handlers following the standard protocol \
            specifications and evaluate their throughput under realistic workloads.";
        assert!(!needs_ocr(ok, LETTER, true));
    }

    #[test]
    fn non_latin_text_does_not_trip_the_vowel_rule() {
        let t = "Доказательства интерактивные системы сложность классы иерархия";
        assert!(!needs_ocr(t, LETTER, false));
    }

    #[test]
    fn page_text_joins_and_drops_low_confidence() {
        let lines = vec![
            line("Arthur-Merlin Games", 0.9, [0.1, 0.1, 0.5, 0.03]),
            line("~", 0.1, [0.1, 0.15, 0.01, 0.01]),
            line("   ", 1.0, [0.1, 0.16, 0.01, 0.01]),
            line("LÁSZLÓ BABAI  ", 0.3, [0.1, 0.2, 0.3, 0.02]),
        ];
        assert_eq!(page_text(&lines), "Arthur-Merlin Games\nLÁSZLÓ BABAI");
        assert_eq!(page_text(&[]), "");
    }

    #[test]
    fn reading_order_rows_then_columns() {
        let mut lines = vec![
            line("body", 1.0, [0.1, 0.5, 0.8, 0.02]),
            line("right", 1.0, [0.6, 0.201, 0.3, 0.02]),
            line("title", 1.0, [0.2, 0.05, 0.6, 0.04]),
            line("left", 1.0, [0.1, 0.2, 0.3, 0.02]),
        ];
        sort_reading_order(&mut lines);
        let order: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(order, ["title", "left", "right", "body"]);
    }

    #[test]
    fn garbage_input_is_none() {
        assert_eq!(recognize_png(&[], &[]), None);
        assert_eq!(recognize_png(b"definitely not a png", &["en-US"]), None);
    }

    #[cfg(target_os = "macos")]
    fn normalize(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn recognizes_fixture_sentence_off_main_thread() {
        let png: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ocr-sample.png"
        ));
        // Run on a spawned thread, as the PDFium worker will.
        let lines = std::thread::spawn(move || recognize_png(png, &[]))
            .join()
            .unwrap()
            .expect("Vision should read the fixture");
        let text = normalize(&page_text(&lines));
        assert_eq!(
            text,
            "arthurmerlin games the quick brown fox jumps over the lazy dog"
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
        for l in &lines {
            assert!(l.confidence >= MIN_LINE_CONFIDENCE);
            let [x, y, w, h] = l.bbox;
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
            assert!(w > 0.0 && h > 0.0 && x + w <= 1.01 && y + h <= 1.01);
        }
        // Title is in the top half, sentence in the bottom half.
        assert!(lines[0].bbox[1] < 0.5 && lines[1].bbox[1] > 0.4);
    }

    /// Manual: `OCR_PNG=/path/page.png cargo test ocr::tests::ocr_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ocr_file() {
        let Ok(path) = std::env::var("OCR_PNG") else {
            return;
        };
        let png = std::fs::read(path).unwrap();
        let t0 = std::time::Instant::now();
        let lines = recognize_png(&png, &["en-US"]);
        let dt = t0.elapsed();
        let lines = lines.expect("ocr failed");
        eprintln!("{} lines in {dt:?}", lines.len());
        for l in &lines {
            eprintln!("{:.2} {:?} {}", l.confidence, l.bbox, l.text);
        }
        let text = page_text(&lines);
        eprintln!("needs_ocr(ocr text) = {}", needs_ocr(&text, 0.0, true));
    }
}
