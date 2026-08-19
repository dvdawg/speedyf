mod cache;
mod commands;
pub mod engine;
pub mod errors;
mod library;
mod search;

use commands::{EngineState, PendingOpens};
use engine::types::{DocId, Priority, RenderKey, RenderKind, TileRect};
use engine::{EngineHandle, Work};
use tauri::http::{header, Response, StatusCode};
use tauri::{Emitter, Manager};

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

pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    tauri::Builder::default()
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
            let doc = q.key.doc;
            engine.submit_at_gen(
                q.prio,
                doc,
                q.gen,
                Work::Render {
                    key: q.key,
                    respond: Box::new(move |result| match result {
                        Ok(bytes) => responder.respond(png_response(&bytes)),
                        Err(crate::errors::AppError::Stale) => {
                            responder.respond(status_response(StatusCode::NO_CONTENT))
                        }
                        Err(e) => {
                            log::warn!("render failed: {e}");
                            responder.respond(status_response(StatusCode::INTERNAL_SERVER_ERROR))
                        }
                    }),
                },
            );
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
            commands::get_form_fields,
            commands::get_outline,
            commands::image_size,
            commands::image_preview,
            commands::engine_metrics,
            commands::set_low_memory,
            commands::doc_generation,
            commands::bump_generation,
            commands::cancel_renders,
        ])
        .build(tauri::generate_context!())
        .expect("error while building SpeedyF")
        .run(|app_handle, event| {
            // Fired when the OS asks us to open a file: double-click, "Open
            // With", or a file dropped on the Dock icon. Also fires once on
            // a cold launch if SpeedyF was opened by opening a PDF.
            if let tauri::RunEvent::Opened { urls } = event {
                let paths: Vec<String> = urls
                    .into_iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if paths.is_empty() {
                    return;
                }
                if let Some(state) = app_handle.try_state::<PendingOpens>() {
                    state.0.lock().unwrap().extend(paths.clone());
                }
                let _ = app_handle.emit("file-open-requested", &paths);

                // The OS hands us the file but does not raise us. Without this,
                // an already-running SpeedyF loads the PDF behind whatever the
                // user was looking at, and the open looks like it did nothing.
                #[cfg(target_os = "macos")]
                let _ = app_handle.show();
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
