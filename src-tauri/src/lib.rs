mod cache;
mod commands;
pub mod engine;
pub mod errors;
mod external;
mod library;
mod printing;
mod search;

use commands::{EngineState, PendingOpens};
use engine::types::{DocId, Priority, RenderKey, RenderKind, TileRect};
use engine::{EngineHandle, Work};
use errors::AppError;
use std::path::Path;
use std::path::PathBuf;
use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager, UriSchemeResponder};

fn bad_request(msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

fn png_response(bytes: &[u8]) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        // URLs embed the generation, so responses are immutable by construction.
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(bytes.to_vec())
        .unwrap()
}

fn status_response(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(status).body(Vec::new()).unwrap()
}

struct RenderQuery {
    key: RenderKey,
    gen: u64,
    prio: Priority,
}

fn parse_render_query(uri: &str) -> Option<RenderQuery> {
    let parsed = url::Url::parse(uri).ok()?;
    if !parsed.path().ends_with("/render") && parsed.path() != "/render" {
        return None;
    }
    let mut doc: Option<DocId> = None;
    let (mut src, mut rot, mut scale, mut gen) = (0u32, 0u16, 1000u32, 0u64);
    let mut kind = RenderKind::Page;
    let (mut tx, mut ty, mut tw, mut th) = (0u32, 0u32, 0u32, 0u32);
    let mut prio: Option<u8> = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "doc" => doc = v.parse().ok(),
            "src" => src = v.parse().ok()?,
            "rot" => rot = v.parse().ok()?,
            "scale" => scale = v.parse().ok()?,
            "gen" => gen = v.parse().ok()?,
            "kind" => {
                kind = match v.as_ref() {
                    "thumb" => RenderKind::Thumb,
                    "tile" => RenderKind::Tile,
                    "preview" => RenderKind::Preview,
                    _ => RenderKind::Page,
                }
            }
            "tx" => tx = v.parse().ok()?,
            "ty" => ty = v.parse().ok()?,
            "tw" => tw = v.parse().ok()?,
            "th" => th = v.parse().ok()?,
            "p" => prio = v.parse().ok(),
            _ => {}
        }
    }
    let doc = doc?;
    if rot % 90 != 0 || rot >= 360 || scale == 0 || scale > 12_000 {
        return None;
    }
    let tile = if matches!(kind, RenderKind::Tile | RenderKind::Preview) {
        if tw == 0 || th == 0 || tw > 4096 || th > 4096 {
            return None;
        }
        Some(TileRect {
            x: tx,
            y: ty,
            w: tw,
            h: th,
        })
    } else {
        None
    };
    let default_prio = match kind {
        RenderKind::Page => Priority::VisiblePage,
        RenderKind::Tile => Priority::VisibleTile,
        RenderKind::Thumb => Priority::VisibleThumb,
        RenderKind::Preview => Priority::HoverPreview,
    };
    Some(RenderQuery {
        key: RenderKey {
            doc,
            src,
            rot,
            scale_milli: scale,
            kind,
            tile,
        },
        gen,
        prio: prio.map(Priority::from_u8).unwrap_or(default_prio),
    })
}

/// How many times a render abandoned by a *cancel* stamp is put back on the
/// queue before the request is answered empty.
///
/// A cancel abandons queued and in-flight renders, but the webview has no way
/// to learn that. The `<img>` the request came from holds a src that is a pure
/// function of (doc, src, rotation, scale, generation, tile), so answering 204
/// leaves a page or tile permanently blank: the URL never changes, so nothing
/// ever asks again. Cancellation is a scheduling signal — "not right now", not
/// "never" — so a cancelled render goes back on the queue at the *current*
/// stamp instead. Aborting still hands PDFium straight back to visible work,
/// which is the point of cancelling; the abandoned request just finishes later.
///
/// The URL generation check stays authoritative: a superseded URL is genuinely
/// dead, and the frontend mints a replacement for it.
const CANCEL_RETRIES: u8 = 2;

fn submit_render(
    engine: EngineHandle,
    prio: Priority,
    gen: u64,
    key: RenderKey,
    responder: UriSchemeResponder,
    retries_left: u8,
) {
    let doc = key.doc;
    let retry = (retries_left > 0).then(|| (engine.clone(), key.clone()));
    engine.submit_at_gen(
        prio,
        doc,
        gen,
        Work::Render {
            key,
            respond: Box::new(move |result| match result {
                Ok(bytes) => responder.respond(png_response(&bytes)),
                Err(AppError::Stale) => match retry {
                    // Re-queued at the cancel stamp current *now*, so it only
                    // dies again if a further cancel lands after this point.
                    Some((engine, key)) => {
                        submit_render(engine, prio, gen, key, responder, retries_left - 1)
                    }
                    None => responder.respond(status_response(StatusCode::NO_CONTENT)),
                },
                Err(e) => {
                    log::warn!("render failed: {e}");
                    responder.respond(status_response(StatusCode::INTERNAL_SERVER_ERROR))
                }
            }),
        },
    );
}

pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let builder = tauri::Builder::default();

    // Must be registered before anything else so a duplicate launch is caught
    // before it can start building a second app.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(
        |app_handle, argv, cwd| {
            // A second process was started to open a document — most likely a
            // double-click while SpeedyF was already running. Take its
            // arguments, then raise the window that already exists.
            let paths = file_arguments(argv, Path::new(&cwd));
            if !paths.is_empty() {
                deliver_opens(app_handle, paths);
            }
            raise_main_window(app_handle);
        },
    ));

    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let mut hints = Vec::new();
            if let Ok(resources) = app.path().resource_dir() {
                hints.push(resources.join("pdfium"));
                hints.push(resources);
            }
            let low_memory = std::env::var("SPEEDYF_LOW_MEMORY").is_ok_and(|v| v == "1");
            let app_data_dir = app.path().app_data_dir().ok();
            let engine = EngineHandle::start(hints, low_memory, app_data_dir);

            let emitter = app.handle().clone();
            engine.set_progress_callback(Box::new(move |doc, status| {
                let _ = emitter.emit(
                    "search:progress",
                    serde_json::json!({
                        "docId": doc,
                        "indexed": status.indexed,
                        "total": status.total,
                        "truncated": status.truncated,
                    }),
                );
            }));

            app.manage(EngineState(engine));
            app.manage(PendingOpens::default());

            // Print jobs a crash left behind. Best effort — never a reason
            // the app fails to open.
            printing::sweep_stale_prints();

            // How Windows and Linux deliver "open with SpeedyF", and how a
            // terminal launch delivers it anywhere. macOS additionally raises
            // RunEvent::Opened for a double-click; its GUI argv carries a
            // process-serial-number flag, which is filtered out below.
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let opened_with = file_arguments(std::env::args(), &cwd);
            if !opened_with.is_empty() {
                deliver_opens(app.handle(), opened_with);
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("pdfr", |ctx, request, responder| {
            let uri = request.uri().to_string();
            let Some(q) = parse_render_query(&uri) else {
                responder.respond(bad_request("malformed render request"));
                return;
            };
            let engine = ctx.app_handle().state::<EngineState>().0.clone();

            // Only the exact current generation is valid. This rejects both
            // stale URLs and forged future generations before queueing.
            if q.gen != engine.current_generation(q.key.doc) {
                responder.respond(status_response(StatusCode::NO_CONTENT));
                return;
            }
            // Fast path: serve straight from the byte-budgeted cache.
            if let Some(hit) = engine.cached_render(&q.key) {
                responder.respond(png_response(&hit));
                return;
            }
            // Slow path: enqueue; the worker calls back with bytes or an error.
            submit_render(engine, q.prio, q.gen, q.key, responder, CANCEL_RETRIES);
        })
        .invoke_handler(tauri::generate_handler![
            commands::take_pending_opens,
            commands::file_metadata,
            commands::open_document,
            commands::close_document,
            commands::set_active_document,
            commands::request_page_sizes,
            commands::get_text_layout,
            commands::get_page_links,
            commands::get_preview_rect,
            commands::resolve_citation,
            commands::set_library_root,
            commands::library_status,
            commands::start_indexing,
            commands::search_query,
            commands::get_match_rects,
            commands::save_document,
            commands::build_print_pdf,
            commands::discard_print_pdf,
            commands::export_print_pdf,
            commands::list_printers,
            commands::printer_options,
            commands::submit_print,
            commands::get_form_fields,
            commands::get_outline,
            commands::get_formal_envs,
            commands::get_figures,
            commands::image_size,
            commands::image_preview,
            commands::engine_metrics,
            commands::set_low_memory,
            commands::doc_generation,
            commands::bump_generation,
            commands::cancel_renders,
            commands::open_external_url,
        ])
        .build(tauri::generate_context!())
        .expect("error while building SpeedyF")
        .run(handle_run_event);
}

/// The OS asking us to open a file: double-click, "Open With", or a file
/// dropped on the Dock icon. Also fires once on a cold launch when SpeedyF was
/// started by opening a PDF.
///
/// macOS and iOS only. `RunEvent::Opened` does not exist on the other
/// platforms — referring to it there is a compile error, not a no-op — and
/// they do not need it: Windows and Linux hand the path to a fresh process on
/// the command line instead. See `file_arguments`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn handle_run_event(app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
    let tauri::RunEvent::Opened { urls } = event else {
        return;
    };
    let paths: Vec<String> = urls
        .into_iter()
        .filter_map(|u| u.to_file_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if paths.is_empty() {
        return;
    }
    deliver_opens(app_handle, paths);

    raise_main_window(app_handle);
}

/// Bring the window forward. The OS hands us a file but does not raise us:
/// without this an already-running SpeedyF loads the PDF behind whatever the
/// user was looking at, and the open looks like it did nothing.
fn raise_main_window(app_handle: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app_handle.show();
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn handle_run_event(_app_handle: &tauri::AppHandle, _event: tauri::RunEvent) {}

/// Hand opened paths to the frontend, and stash them for a frontend that is
/// not listening yet — a cold launch reaches here before the webview exists.
fn deliver_opens(app_handle: &tauri::AppHandle, paths: Vec<String>) {
    if let Some(state) = app_handle.try_state::<PendingOpens>() {
        state.0.lock().unwrap().extend(paths.clone());
    }
    let _ = app_handle.emit("file-open-requested", &paths);
}

/// Files named on the command line, which is how every platform except macOS
/// delivers "open this document with SpeedyF" — and how a terminal launch
/// delivers it on macOS too.
///
/// `base` is the directory a relative path is relative *to*. For a launch that
/// is our own working directory; for a second instance handing its arguments
/// over, it is that process's, which is not ours and is the whole reason this
/// is a parameter.
///
/// Flags are skipped, and so is anything that is not a file that exists: a
/// mistyped path should leave the app open on the home screen rather than
/// reporting a failure the user did not ask for.
fn file_arguments<I: IntoIterator<Item = String>>(args: I, base: &Path) -> Vec<String> {
    args.into_iter()
        .skip(1)
        .filter(|arg| !arg.is_empty() && !arg.starts_with('-'))
        .filter_map(|arg| {
            let path = Path::new(&arg);
            let resolved = if path.is_absolute() {
                path.to_path_buf()
            } else {
                base.join(path)
            };
            resolved
                .is_file()
                .then(|| resolved.to_string_lossy().into_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_opens_keep_only_real_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pdf = dir.path().join("paper.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n").expect("write");
        let missing = dir.path().join("gone.pdf");

        let args = vec![
            "speedyf".to_string(),
            "--some-flag".to_string(),
            String::new(),
            missing.to_string_lossy().into_owned(),
            pdf.to_string_lossy().into_owned(),
        ];
        assert_eq!(
            file_arguments(args, dir.path()),
            vec![pdf.to_string_lossy().into_owned()],
            "only the file that exists, and never the flag"
        );
    }

    #[test]
    fn a_relative_path_resolves_against_the_callers_directory() {
        // A second instance hands over its own working directory, which is not
        // ours. Resolving against the wrong one silently opens nothing.
        let dir = tempfile::tempdir().expect("temp dir");
        let pdf = dir.path().join("paper.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n").expect("write");

        let args = vec!["speedyf".to_string(), "paper.pdf".to_string()];
        assert_eq!(
            file_arguments(args.clone(), dir.path()),
            vec![dir.path().join("paper.pdf").to_string_lossy().into_owned()]
        );
        // ...and the same argument means nothing from somewhere else.
        let elsewhere = tempfile::tempdir().expect("temp dir");
        assert!(file_arguments(args, elsewhere.path()).is_empty());
    }

    #[test]
    fn an_argument_free_launch_opens_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(file_arguments(vec!["speedyf".to_string()], dir.path()).is_empty());
    }

    #[test]
    fn parses_native_custom_scheme_render_urls() {
        let query = parse_render_query(
            "pdfr://localhost/render?doc=7&src=12&rot=90&scale=1750&gen=4&kind=page&p=2",
        )
        .expect("valid native render URL");
        assert_eq!(query.key.doc, 7);
        assert_eq!(query.key.src, 12);
        assert_eq!(query.key.rot, 90);
        assert_eq!(query.key.scale_milli, 1750);
        assert_eq!(query.key.kind, RenderKind::Page);
        assert_eq!(query.gen, 4);
        assert_eq!(query.prio, Priority::HoverPreview);
    }

    #[test]
    fn parses_windows_http_rewrite_and_tile_query() {
        let query = parse_render_query(
            "http://pdfr.localhost/render?doc=2&src=3&rot=270&scale=4000&gen=9&kind=tile&tx=1024&ty=2048&tw=1024&th=900",
        )
        .expect("valid Windows render URL");
        assert_eq!(query.key.kind, RenderKind::Tile);
        assert_eq!(
            query.key.tile,
            Some(TileRect {
                x: 1024,
                y: 2048,
                w: 1024,
                h: 900,
            })
        );
        assert_eq!(query.prio, Priority::VisibleTile);
    }

    #[test]
    fn parses_preview_crop_with_hover_priority() {
        let query = parse_render_query(
            "pdfr://localhost/render?doc=2&src=4&rot=0&scale=2000&gen=3&kind=preview&tx=20&ty=40&tw=840&th=600",
        )
        .expect("valid preview URL");
        assert_eq!(query.key.kind, RenderKind::Preview);
        assert_eq!(
            query.key.tile,
            Some(TileRect {
                x: 20,
                y: 40,
                w: 840,
                h: 600,
            })
        );
        assert_eq!(query.prio, Priority::HoverPreview);
    }

    #[test]
    fn parses_deep_zoom_and_source_indices_above_u16() {
        let query = parse_render_query(
            "pdfr://localhost/render?doc=4&src=69999&rot=0&scale=12000&gen=2&kind=tile&tx=0&ty=0&tw=1024&th=1024",
        )
        .expect("large source indices and the maximum sharp zoom are valid");
        assert_eq!(query.key.src, 69_999);
        assert_eq!(query.key.scale_milli, 12_000);
    }

    #[test]
    fn cache_busting_retry_param_does_not_change_the_render_key() {
        // The frontend retries a blank <img> by appending a throwaway param:
        // it defeats the webview's request cache without minting a different
        // raster, so the retry must resolve to the byte-identical RenderKey.
        let base = "pdfr://localhost/render?doc=3&src=8&rot=0&scale=2000&gen=5&kind=tile&tx=0&ty=1024&tw=1024&th=1024";
        let plain = parse_render_query(base).expect("valid render URL");
        let retried =
            parse_render_query(&format!("{base}&retry=2")).expect("retry URL stays valid");
        assert_eq!(plain.key, retried.key);
        assert_eq!(plain.gen, retried.gen);
        assert_eq!(plain.prio, retried.prio);
    }

    #[test]
    fn rejects_unsafe_or_malformed_render_queries() {
        assert!(
            parse_render_query("pdfr://localhost/render?doc=1&src=0&rot=45&scale=1000&gen=0")
                .is_none()
        );
        assert!(
            parse_render_query("pdfr://localhost/render?doc=1&src=0&rot=0&scale=13000&gen=0")
                .is_none()
        );
        assert!(parse_render_query(
            "pdfr://localhost/render?doc=1&src=0&rot=0&scale=1000&gen=0&kind=tile&tw=0&th=1024"
        )
        .is_none());
        assert!(parse_render_query(
            "pdfr://localhost/render?doc=1&src=0&rot=0&scale=2000&gen=0&kind=preview"
        )
        .is_none());
        assert!(parse_render_query(
            "pdfr://localhost/not-render?doc=1&src=0&rot=0&scale=1000&gen=0"
        )
        .is_none());
    }
}
