//! Dump what a quote is built from, for real pages: the text runs (with
//! their character edges and math stretches), the page's display equations
//! with the glyphs an equation conversion reads for each, and every glyph
//! on the page, as JSON for `src/features/markup/quoteEval.test.ts`. Read
//! only: each PDF is opened from where it is and nothing is written beside
//! it. Run with:
//!
//!   SPEEDYF_QUOTE_PAGES="~/Downloads/mot.pdf#3;~/Downloads/x.pdf#0" \
//!   SPEEDYF_QUOTE_OUT=/tmp/quotes \
//!     cargo test --manifest-path src-tauri/Cargo.toml quote_eval_dump -- --ignored
//!
//! Pages are zero-based. SPEEDYF_QUOTE_PAGES may also name a file listing
//! them, one `path#page` a line.

use crate::engine::{glyphs, pdfium_init, render, text, vision};
use pdfium_render::prelude::*;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Display {
    bbox: [f32; 4],
    number: Option<String>,
    layout: glyphs::GlyphLayoutDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageDump {
    path: String,
    page: u32,
    size: [f32; 2],
    runs: Vec<crate::engine::types::TextRun>,
    displays: Vec<Display>,
    glyphs: Vec<glyphs::GlyphDto>,
}

#[test]
#[ignore]
fn quote_eval_dump() {
    let Ok(mut list) = std::env::var("SPEEDYF_QUOTE_PAGES") else {
        return;
    };
    // Or a file listing them, one a line.
    if let Ok(listed) = std::fs::read_to_string(&list) {
        list = listed.lines().collect::<Vec<_>>().join(";");
    }
    let out = std::env::var("SPEEDYF_QUOTE_OUT").expect("set SPEEDYF_QUOTE_OUT");
    std::fs::create_dir_all(&out).expect("output directory");
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    for (n, item) in list.split(';').filter(|s| !s.trim().is_empty()).enumerate() {
        let (path, page) = item.trim().rsplit_once('#').expect("path#page");
        let page: u32 = page.parse().expect("page number");
        let raw = render::open_document(b, path, None).expect("open");
        // past the last page: nothing to dump
        let Some([w, h]) = render::page_display_size(b, raw, page as i32) else {
            continue;
        };
        let handle = b.FPDF_LoadPage(raw, page as i32);
        assert!(!handle.is_null(), "load page");
        let runs = text::extract_page(b, handle, w, h).runs;
        let all = glyphs::extract_page_glyphs(b, handle, w, h);
        let raster = vision::render_raster(b, handle, w, h);
        let blocks = glyphs::equation_blocks_with_raster(&all, raster.as_ref(), w, h);
        let displays = blocks
            .into_iter()
            .map(|block| Display {
                layout: glyphs::glyph_layout(&all, raster.as_ref(), &[block.bbox], true),
                bbox: block.bbox,
                number: block.number,
            })
            .collect();
        b.FPDF_ClosePage(handle);
        let dump = PageDump {
            path: path.to_string(),
            page,
            size: [w, h],
            runs,
            displays,
            glyphs: all,
        };
        let file = std::path::Path::new(&out).join(format!("page{n}.json"));
        std::fs::write(file, serde_json::to_string(&dump).expect("json")).expect("write");
    }
}
