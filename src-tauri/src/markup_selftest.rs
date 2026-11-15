//! Debug-build self-test of the whole markup export, WebKit capture included.
//!
//!   SPEEDYF_MARKUP_SELFTEST='/path/src.pdf|/path/out.pdf' pnpm tauri dev
//!
//! Once the app is up it copies `src.pdf` into a temp folder, exports it with
//! two margin sheets and two feedback sheets (plain text, KaTeX inline and
//! display math pre-rendered by the katex package, a link, long wrapped text),
//! writes `out.pdf`, logs the result and exits (status 0 on success). The
//! source is only ever read. Compiled into debug builds only.

use crate::engine::markup_export::{
    LinkTarget, MarkKind, MarkupExportRequest, MarkupMark, MarkupSheet, SheetKind, SheetLink,
};
use crate::engine::types::RectDto;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{Emitter, Listener};

const ENV: &str = "SPEEDYF_MARKUP_SELFTEST";
/// KaTeX output for `\int_0^1 x^2\,dx = \frac{1}{3}` (inline) and
/// `\sum_{n\ge 1} \frac{1}{n^2} = \frac{\pi^2}{6}` (display), from
/// `katex.renderToString`, non-ASCII as character references.
const KATEX_INLINE: &str = include_str!("markup_selftest/katex-inline.txt");
const KATEX_DISPLAY: &str = include_str!("markup_selftest/katex-display.txt");
const MARGIN: f32 = 180.0;
/// Text each sheet carries, so the output can be checked by extraction.
pub const MARKERS: [&str; 4] = [
    "SELFTEST MARGIN ONE",
    "SELFTEST MARGIN TWO",
    "SELFTEST FEEDBACK ONE",
    "SELFTEST FEEDBACK TWO",
];

const LONG: &str = "This is a long comment that has to wrap over several lines inside \
    the narrow margin column, so that line breaking, hyphen-free wrapping and the \
    sheet's fixed width are all exercised by the capture. It keeps going for a \
    while longer to make sure the paragraph spans many lines.";

fn sheet_html(marker: &str, wide: bool) -> String {
    let pad = if wide { 48 } else { 10 };
    let size = if wide { 12 } else { 9 };
    format!(
        "<div style=\"padding:{pad}px;font:{size}px/1.35 -apple-system, Helvetica, sans-serif;color:#222\">\
         <p style=\"margin:0 0 6px;font-weight:600\">{marker}</p>\
         <p style=\"margin:0 0 6px\">Inline math {KATEX_INLINE} in a sentence.</p>\
         <div style=\"margin:0 0 6px\">{KATEX_DISPLAY}</div>\
         <p style=\"margin:0 0 6px\"><a href=\"#\" style=\"color:#0645ad\">See feedback page</a></p>\
         <p style=\"margin:0\">{LONG}</p>\
         <div style=\"position:absolute;left:0;bottom:0;width:20px;height:20px;background:#c00\"></div>\
         </div>"
    )
}

fn request(src: &str, dest: &str, page_w: f32, page_h: f32) -> MarkupExportRequest {
    let link = |sheet: usize| SheetLink {
        rect: RectDto {
            x: 10.0,
            y: 120.0,
            w: 120.0,
            h: 14.0,
        },
        target: LinkTarget::Sheet { sheet, y: 0.0 },
    };
    let margin = |page: u32, marker: &str| MarkupSheet {
        kind: SheetKind::Margin,
        page: Some(page),
        width_pt: MARGIN,
        height_pt: page_h,
        html: sheet_html(marker, false),
        links: vec![link(2)],
    };
    let feedback = |marker: &str| MarkupSheet {
        kind: SheetKind::Feedback,
        page: None,
        width_pt: page_w + MARGIN,
        height_pt: page_h,
        html: sheet_html(marker, true),
        links: vec![SheetLink {
            rect: RectDto {
                x: 48.0,
                y: 150.0,
                w: 200.0,
                h: 16.0,
            },
            target: LinkTarget::Page { page: 0, y: 72.0 },
        }],
    };
    MarkupExportRequest {
        source: src.into(),
        dest: dest.into(),
        margin_pt: MARGIN,
        marks: vec![MarkupMark {
            page: 0,
            kind: MarkKind::Highlight,
            quads: None,
            rect: Some(RectDto {
                x: 72.0,
                y: page_h - 120.0,
                w: 200.0,
                h: 12.0,
            }),
            color: "#ffd400".into(),
            contents: "1. Self-test comment".into(),
        }],
        sheets: vec![
            margin(0, MARKERS[0]),
            margin(1, MARKERS[1]),
            feedback(MARKERS[2]),
            feedback(MARKERS[3]),
        ],
        css: None,
    }
}

/// Start the self-test if the environment asks for one.
pub fn maybe_run(app: &tauri::AppHandle) {
    let Ok(spec) = std::env::var(ENV) else {
        return;
    };
    let Some((src, out)) = spec.split_once('|') else {
        eprintln!("{ENV}: expected '<src.pdf>|<out.pdf>'");
        return;
    };
    let (src, out) = (PathBuf::from(src), PathBuf::from(out));
    // The frontend half announces itself once the main window has loaded;
    // listen from the start so that is never missed.
    let (ready_tx, mut ready_rx) = tauri::async_runtime::channel::<()>(1);
    app.listen_any("markup-selftest:ready", move |_| {
        let _ = ready_tx.try_send(());
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Let the main window finish loading first, as a user would.
        tokio::time::sleep(Duration::from_secs(4)).await;
        let mut failed = false;
        match run(&app, &src, &out).await {
            Ok(summary) => eprintln!("markup selftest: capture OK {summary}"),
            Err(e) => {
                eprintln!("markup selftest: capture FAILED {e}");
                failed = true;
            }
        }
        let ready = tokio::time::timeout(Duration::from_secs(60), ready_rx.recv()).await;
        if !matches!(ready, Ok(Some(()))) {
            eprintln!("markup selftest: frontend FAILED never became ready");
            failed = true;
        } else {
            match run_frontend(&app, &src, &out).await {
                Ok(summary) => eprintln!("markup selftest: frontend OK {summary}"),
                Err(e) => {
                    eprintln!("markup selftest: frontend FAILED {e}");
                    failed = true;
                }
            }
        }
        eprintln!("markup selftest: {}", if failed { "FAILED" } else { "OK" });
        app.exit(i32::from(failed));
    });
}

/// `out.pdf` -> `out-frontend.pdf` beside it.
fn sibling(out: &std::path::Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    out.with_file_name(format!("{stem}-{suffix}.{ext}"))
}

/// The same export driven from the main window through the real frontend
/// path (`src/features/markup/selftest.ts`). Writes `<out>-frontend.pdf`,
/// and `<out>-frontend.json` with the report (anchors, sizes, result).
async fn run_frontend(
    app: &tauri::AppHandle,
    src: &std::path::Path,
    out: &std::path::Path,
) -> Result<String, String> {
    let dir = std::env::temp_dir().join("speedyf-markup-selftest");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = dir.join("frontend-source.pdf");
    std::fs::copy(src, &copy).map_err(|e| format!("copy source: {e}"))?;
    let dest = sibling(out, "frontend", "pdf");
    let _ = std::fs::remove_file(&dest);
    let (tx, mut rx) = tauri::async_runtime::channel::<String>(1);
    let id = app.listen_any("markup-selftest:done", move |event| {
        let _ = tx.try_send(event.payload().to_string());
    });
    app.emit_to(
        "main",
        "markup-selftest:run",
        serde_json::json!({
            "source": copy.to_string_lossy(),
            "dest": dest.to_string_lossy(),
        }),
    )
    .map_err(|e| e.to_string())?;
    let report = tokio::time::timeout(Duration::from_secs(120), rx.recv()).await;
    app.unlisten(id);
    let report = match report {
        Ok(Some(report)) => report,
        _ => return Err("no report from the main window".into()),
    };
    let value: serde_json::Value = serde_json::from_str(&report).map_err(|e| e.to_string())?;
    std::fs::write(sibling(out, "frontend", "json"), &report).map_err(|e| e.to_string())?;
    let message = value["message"].as_str().unwrap_or("").to_string();
    if value["ok"].as_bool() == Some(true) {
        Ok(format!("{message} -> {}", dest.display()))
    } else {
        Err(message)
    }
}

async fn run(
    app: &tauri::AppHandle,
    src: &std::path::Path,
    out: &std::path::Path,
) -> Result<String, String> {
    let dir = std::env::temp_dir().join("speedyf-markup-selftest");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = dir.join("source.pdf");
    std::fs::copy(src, &copy).map_err(|e| format!("copy source: {e}"))?;
    let (w, h) = std::env::var("SPEEDYF_MARKUP_SELFTEST_PAGE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((612.0, 792.0));
    let req = request(&copy.to_string_lossy(), &out.to_string_lossy(), w, h);
    let started = std::time::Instant::now();
    let result = crate::commands::markup::run_export(app, req)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "{} pages, {} links, {:?} -> {}",
        result.pages,
        result.links,
        started.elapsed(),
        result.path
    ))
}
