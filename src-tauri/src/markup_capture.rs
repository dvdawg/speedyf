//! HTML -> PDF for markup export: each comment sheet becomes one vector PDF
//! page, text selectable.
//!
//! A hidden webview window loads the minimal `markup-print.html` entry
//! (`src/features/markup/printEntry.ts`). That page asks for its job with
//! `markup_print_payload`, then for each sheet: lays it out at its exact size
//! (CSS px == pt), waits for fonts, and calls `markup_print_capture`, which
//! has WKWebView `createPDF` capture the sheet's rectangle. When it is done
//! (or has failed) it calls `markup_print_finish`. The window is destroyed on
//! every path out, including the timeout.
//!
//! Sits beside `engine`, like `printing`: nothing here touches PDFium. The
//! captured pages go to `engine::markup_export` on the worker thread.

use crate::engine::markup_export::MarkupExportRequest;
use crate::errors::{AppError, AppResult};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Labels of capture windows. Outside the `speedyf-*` document-window
/// namespace on purpose: these are not documents, and no capability grants
/// them anything beyond the app's own commands.
pub const LABEL_PREFIX: &str = "markup-print-";
/// The page served to capture windows (a separate Vite entry).
const ENTRY: &str = "markup-print.html";
/// Base allowance for the whole capture, plus a little per sheet.
const BASE_TIMEOUT: Duration = Duration::from_secs(20);
const PER_SHEET: Duration = Duration::from_millis(500);
/// One createPDF call.
#[cfg(target_os = "macos")]
const SHEET_TIMEOUT: Duration = Duration::from_secs(15);

pub const NEEDS_MACOS: &str = "Markup export needs macOS for now";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrintSheet {
    pub width_pt: f32,
    pub height_pt: f32,
    pub html: String,
}

/// What the print page renders.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrintPayload {
    pub sheets: Vec<PrintSheet>,
    pub css: String,
}

impl PrintPayload {
    pub fn from_request(req: &MarkupExportRequest) -> PrintPayload {
        PrintPayload {
            sheets: req
                .sheets
                .iter()
                .map(|s| PrintSheet {
                    width_pt: s.width_pt,
                    height_pt: s.height_pt,
                    html: s.html.clone(),
                })
                .collect(),
            css: req.css.clone().unwrap_or_default(),
        }
    }

    /// The capture window's size: big enough for every sheet.
    pub fn window_size(&self) -> (f64, f64) {
        self.sheets.iter().fold((1.0f64, 1.0f64), |(w, h), s| {
            (w.max(s.width_pt as f64), h.max(s.height_pt as f64))
        })
    }
}

type Done = tauri::async_runtime::Sender<Result<(), String>>;

struct Job {
    payload: PrintPayload,
    pdfs: Vec<Option<Vec<u8>>>,
    done: Option<Done>,
}

/// Capture jobs in flight, by window label.
#[derive(Default)]
pub struct PrintJobs(Mutex<HashMap<String, Job>>);

impl PrintJobs {
    fn with_job<T>(&self, label: &str, f: impl FnOnce(&mut Job) -> AppResult<T>) -> AppResult<T> {
        let mut jobs = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let job = jobs
            .get_mut(label)
            .ok_or_else(|| AppError::Unsupported("not a markup print window".into()))?;
        f(job)
    }
}

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

/// Removes the job and destroys its window however the capture ends.
struct Teardown {
    app: tauri::AppHandle,
    label: String,
}

impl Drop for Teardown {
    fn drop(&mut self) {
        use tauri::Manager;
        if let Some(jobs) = self.app.try_state::<PrintJobs>() {
            jobs.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.label);
        }
        if let Some(window) = self.app.get_webview_window(&self.label) {
            let _ = window.destroy();
        }
    }
}

/// Render every sheet of `req` to a one-page PDF, in request order.
pub async fn capture_sheets(
    app: &tauri::AppHandle,
    req: &MarkupExportRequest,
) -> AppResult<Vec<Vec<u8>>> {
    if req.sheets.is_empty() {
        return Ok(Vec::new());
    }
    if !cfg!(target_os = "macos") {
        return Err(AppError::Unsupported(NEEDS_MACOS.into()));
    }
    use tauri::Manager;
    let payload = PrintPayload::from_request(req);
    let (width, height) = payload.window_size();
    let count = payload.sheets.len();
    let label = format!("{LABEL_PREFIX}{}", NEXT_JOB.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = tauri::async_runtime::channel::<Result<(), String>>(1);
    app.state::<PrintJobs>()
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            label.clone(),
            Job {
                payload,
                pdfs: vec![None; count],
                done: Some(tx),
            },
        );
    let teardown = Teardown {
        app: app.clone(),
        label: label.clone(),
    };

    tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App(ENTRY.into()))
        .title("SpeedyF Markup")
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .resizable(false)
        .inner_size(width, height)
        .build()
        .map_err(|e| AppError::Internal(format!("cannot open the print view: {e}")))?;

    let limit = BASE_TIMEOUT + PER_SHEET * count as u32;
    let outcome = tokio::time::timeout(limit, rx.recv()).await;
    let result = match outcome {
        Err(_) => Err(AppError::Internal(
            "rendering the comment sheets timed out".into(),
        )),
        Ok(None) => Err(AppError::Internal("the print view went away".into())),
        Ok(Some(Err(message))) => Err(AppError::Internal(format!(
            "rendering the comment sheets failed: {message}"
        ))),
        Ok(Some(Ok(()))) => app.state::<PrintJobs>().with_job(&label, |job| {
            job.pdfs
                .iter_mut()
                .enumerate()
                .map(|(i, pdf)| {
                    pdf.take().ok_or_else(|| {
                        AppError::Internal(format!("sheet {} was not captured", i + 1))
                    })
                })
                .collect()
        }),
    };
    drop(teardown);
    result
}

/// The calling print window's job.
#[tauri::command]
pub fn markup_print_payload(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, PrintJobs>,
) -> AppResult<PrintPayload> {
    jobs.with_job(window.label(), |job| Ok(job.payload.clone()))
}

/// Capture sheet `index`, which the calling print window is now showing at
/// its top-left corner. Sizes come from the job, never from the page.
#[tauri::command]
pub async fn markup_print_capture(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, PrintJobs>,
    index: usize,
) -> AppResult<()> {
    let (w, h) = jobs.with_job(window.label(), |job| {
        job.payload
            .sheets
            .get(index)
            .map(|s| (s.width_pt as f64, s.height_pt as f64))
            .ok_or_else(|| AppError::Unsupported("no such sheet".into()))
    })?;
    let pdf = native::create_pdf(&window, w, h).await?;
    jobs.with_job(window.label(), |job| {
        job.pdfs[index] = Some(pdf);
        Ok(())
    })
}

/// The print window is done: every sheet captured, or `error`.
#[tauri::command]
pub fn markup_print_finish(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, PrintJobs>,
    error: Option<String>,
) -> AppResult<()> {
    let done = jobs.with_job(window.label(), |job| Ok(job.done.take()))?;
    if let Some(done) = done {
        let _ = done.try_send(error.map_or(Ok(()), Err));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use super::SHEET_TIMEOUT;
    use crate::errors::{AppError, AppResult};
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSData, NSError};
    use objc2_web_kit::WKPDFConfiguration;
    use std::sync::Mutex;

    type Reply = tauri::async_runtime::Sender<Result<Vec<u8>, String>>;

    /// One vector PDF page of the web view's content in `(0, 0, w, h)`
    /// (page coordinates, so content outside the visible viewport counts).
    pub async fn create_pdf(window: &tauri::WebviewWindow, w: f64, h: f64) -> AppResult<Vec<u8>> {
        let (tx, mut rx) = tauri::async_runtime::channel::<Result<Vec<u8>, String>>(1);
        window
            .with_webview(move |webview| {
                // SAFETY: runs on the main thread, where Tauri calls this
                // closure; `inner()` is the window's live WKWebView.
                unsafe { start(webview.inner(), w, h, tx) }
            })
            .map_err(|e| AppError::Internal(format!("print view unavailable: {e}")))?;
        match tokio::time::timeout(SHEET_TIMEOUT, rx.recv()).await {
            Ok(Some(Ok(bytes))) if !bytes.is_empty() => Ok(bytes),
            Ok(Some(Ok(_))) => Err(AppError::Internal("WebKit produced an empty PDF".into())),
            Ok(Some(Err(message))) => Err(AppError::Internal(format!("WebKit: {message}"))),
            Ok(None) => Err(AppError::Internal("WebKit did not answer".into())),
            Err(_) => Err(AppError::Internal("WebKit took too long".into())),
        }
    }

    /// # Safety
    /// Main thread only; `webview` must be a live WKWebView.
    unsafe fn start(webview: *mut std::ffi::c_void, w: f64, h: f64, tx: Reply) {
        // SAFETY: the caller guarantees a live WKWebView (or null).
        let Some(view) = (unsafe { (webview as *const AnyObject).as_ref() }) else {
            let _ = tx.try_send(Err("no web view".into()));
            return;
        };
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            let _ = tx.try_send(Err("not on the main thread".into()));
            return;
        };
        let config = unsafe { WKPDFConfiguration::new(mtm) };
        unsafe {
            config.setRect(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w, h)));
        }
        // The block is `Fn`; the reply is sent once.
        let slot = Mutex::new(Some(tx));
        let handler = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
            // SAFETY: WebKit passes either valid objects or null.
            let result = match unsafe { data.as_ref() } {
                Some(data) => Ok(data.to_vec()),
                None => Err(unsafe { error.as_ref() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "createPDF failed".into())),
            };
            if let Some(tx) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = tx.try_send(result);
            }
        });
        // SAFETY: createPDFWithConfiguration:completionHandler: (macOS 11+)
        // takes a configuration and a block of exactly this signature.
        unsafe {
            let _: () = msg_send![
                view,
                createPDFWithConfiguration: &*config,
                completionHandler: &*handler
            ];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use crate::errors::{AppError, AppResult};

    pub async fn create_pdf(
        _window: &tauri::WebviewWindow,
        _w: f64,
        _h: f64,
    ) -> AppResult<Vec<u8>> {
        Err(AppError::Unsupported(super::NEEDS_MACOS.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::markup_export::{MarkupSheet, SheetKind};

    #[test]
    fn payload_carries_sheets_in_order_and_sizes_the_window() {
        let sheet = |w: f32, h: f32, html: &str| MarkupSheet {
            kind: SheetKind::Feedback,
            page: None,
            width_pt: w,
            height_pt: h,
            html: html.into(),
            links: Vec::new(),
        };
        let req = MarkupExportRequest {
            source: "/a.pdf".into(),
            dest: "/b.pdf".into(),
            margin_pt: 180.0,
            marks: Vec::new(),
            sheets: vec![
                sheet(180.0, 792.0, "<p>1</p>"),
                sheet(792.0, 612.0, "<p>2</p>"),
            ],
            css: None,
        };
        let payload = PrintPayload::from_request(&req);
        assert_eq!(payload.sheets.len(), 2);
        assert_eq!(payload.sheets[1].html, "<p>2</p>");
        assert_eq!(payload.css, "");
        assert_eq!(payload.window_size(), (792.0, 792.0));
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["sheets"][0]["widthPt"], 180.0);
    }

    #[test]
    fn unknown_windows_have_no_job() {
        let jobs = PrintJobs::default();
        assert!(jobs.with_job("main", |_| Ok(())).is_err());
    }
}
