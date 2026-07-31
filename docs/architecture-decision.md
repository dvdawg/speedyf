# Architecture Decision Record: SpeedyF Desktop PDF Viewer/Editor

- **Status:** Accepted
- **Date:** 2026-07-20
- **Environment at decision time:** macOS 15.5 (arm64, 8 cores, 16 GB RAM), full Xcode 17 toolchain,
  Node 24 + pnpm 9, Rust 1.97 (installed during setup), network access to npm/GitHub,
  `pdfium-binaries` prebuilts available (tag `chromium/7961`).

## Problem

Build a local-first, cross-platform desktop PDF viewer/editor foundation optimized for: fast startup,
low steady-state memory, fast time-to-first-page, smooth scroll/zoom, large-document handling, and a
modular base for future development. Editing scope: page reorder/delete/duplicate/rotate, blank pages,
added text/images, annotations (highlight/ink/rectangle/text box/note), AcroForm text fill where
supported, safe save/export.

## Alternatives considered

| #   | Stack                                                                         | Verdict                               |
| --- | ----------------------------------------------------------------------------- | ------------------------------------- |
| 1   | **Tauri 2 shell + Rust core owning PDFium (`pdfium-render`) + SolidJS/TS UI** | **Chosen**                            |
| 2   | Tauri 2 + PDF.js as primary renderer (wasm in webview) + `lopdf` for save     | Rejected                              |
| 3   | Electron + PDF.js                                                             | Rejected                              |
| 4   | Pure-native Rust UI (egui/iced/Slint) + PDFium                                | Rejected for v1; credible future path |
| 5   | Same as #1 but MuPDF instead of PDFium                                        | Rejected (licensing)                  |
| 6   | Swift + PDFKit                                                                | Rejected (not cross-platform)         |

### Evaluation against the required criteria

**1. Tauri 2 + Rust/PDFium + SolidJS (chosen)**

- _Startup:_ Tauri uses the OS webview (WKWebView/WebView2/WebKitGTK) — no bundled browser. Binaries
  are ~10–20 MB; cold start is dominated by webview init, typically well under a second on modern
  hardware. Solid compiles to fine-grained DOM updates with a ~7 KB runtime; no virtual-DOM churn
  during scroll/zoom state updates.
- _Renderer performance:_ PDFium is the C++ engine inside Chrome's PDF viewer — fast rasterization,
  battle-tested against malformed files.
- _Memory:_ All page bitmaps live in Rust behind byte-budgeted LRU caches. The webview holds only
  `<img>` elements for mounted (visible ± overscan) pages, so decoded-image memory is bounded by
  virtualization. The document itself is opened by PDFium from the file (no full copy in JS).
- _Text extraction:_ PDFium text APIs (char boxes, rects) — same quality as Chrome's viewer.
- _Annotations:_ `FPDFAnnot_*` supports Highlight/Square/Ink/Text annots. The implemented save path
  creates standard Highlight/Square/Text annotations; ink is flattened to page path objects for
  portable appearance. FreeText appearance generation is weak → added text is written as real page
  text objects instead.
- _Forms:_ PDFium has form APIs; `pdfium-render` exposes field enumeration and (partially) value
  setting. Treated as best-effort with graceful degradation.
- _Editing/save:_ Page import/delete/rotate/new-page + text/image page objects + `FPDF_SaveAsCopy`.
  Reliable for the required edit set.
- _Parallelism:_ PDFium is **not** thread-safe. All PDFium calls are serialized on one dedicated
  engine thread with a priority queue + generation-based cancellation. The task interface is
  message-based so a process pool can replace the thread later without touching callers.
- _Crash isolation:_ v1 runs the engine thread in-process (a PDFium crash kills the app). Accepted
  for v1; the message-based engine boundary is the seam for a helper-process pool later. Documented
  as a known risk.
- _Packaging:_ `tauri bundler` produces .app/.dmg, .msi/.exe, .deb/.rpm/.AppImage.
- _Licensing:_ PDFium BSD-3-Clause + Apache-2.0 deps; Tauri MIT/Apache-2.0; Solid MIT;
  `pdfium-render` MIT/Apache-2.0. All permissive.
- _Complexity:_ Moderate. `pdfium-render` is a mature safe wrapper; raw FFI escape hatch
  (`bindings()`) covers gaps.

**2. Tauri 2 + PDF.js primary**

- Rendering and bitmaps live in the JS heap → byte-budgeted memory control is much weaker (GC-driven,
  per-canvas), and large scanned pages regularly spike webview memory. Saving edits requires a second
  full PDF engine (`pdf-lib`/`lopdf`) parsing the same bytes — explicitly discouraged by the spec.
  wasm init adds startup latency. PDF.js's excellent text layer is the main loss; we replicate the
  technique (positioned transparent spans) over PDFium char boxes. Kept as a _documented optional
  fallback path_ (the renderer abstraction does not expose PDFium types), not as a second bundled engine.

**3. Electron + PDF.js**

- Ships Chromium + Node (~200 MB installs, ~150–250 MB baseline RSS before any document, slower cold
  start). Violates the startup/memory goals outright; still needs a second engine for editing.

**4. Native Rust UI + PDFium**

- Best theoretical startup/memory (no webview at all), but text selection, IME, accessibility,
  clipboard, menus, and polished widgets must be hand-built (egui/iced), or licensed (Slint).
  Development cost and a11y regression risk are too high for the foundation milestone. Re-evaluate
  if webview overhead is _measured_ to dominate (see "Evidence to revisit").

**5. MuPDF core**

- Excellent renderer, but AGPL-3.0 (or commercial license). Viral copyleft is unacceptable for a
  foundation intended for future proprietary work. Rejected on licensing alone.

**6. Swift + PDFKit**

- macOS-only. Fails the cross-platform requirement. Not pursued.

## Chosen architecture

```
┌────────────────────────── Tauri 2 process ──────────────────────────┐
│  ┌───────────── OS WebView (SolidJS + TS) ─────────────┐            │
│  │ UI components · virtualized viewer · SVG annotation │            │
│  │ overlay · text-selection layer · document model     │            │
│  │ store + undo/redo (metadata only, no bitmaps/bytes) │            │
│  └───────┬───────────────────────────────▲─────────────┘            │
│   invoke │ commands / events             │ pdfr:// binary PNG       │
│  ┌───────▼───────────────────────────────┴─────────────┐            │
│  │ Rust host: commands, pdfr:// protocol handler,      │            │
│  │ byte-budgeted LRU caches (pages/thumbs/text),       │            │
│  │ search index, EditPlan save pipeline (atomic)       │            │
│  └───────────────────┬──────────────────────────────────┘           │
│              message │ channel (EngineTask, priority, generation)   │
│  ┌───────────────────▼──────────────────────────────────┐           │
│  │ Dedicated engine thread — sole owner of PDFium:      │           │
│  │ open/close, render page/tile/thumb, text extraction, │           │
│  │ page sizes, save. Priority queue, stale-task skip.   │           │
│  └───────────────────────────────────────────────────────┘          │
└──────────────────────────────────────────────────────────────────────┘
```

Key decisions:

1. **PDFium binding:** `pdfium-render` with **dynamic linking**; `scripts/fetch-pdfium.mjs` downloads
   the platform dylib/so/dll from `bblanchon/pdfium-binaries` (pinned tag) into `src-tauri/pdfium/`.
   Bundled as a Tauri resource for production builds.
2. **One engine thread** owns every PDFium call (PDFium is not thread-safe). Tasks carry
   `(priority, generation, seq)`; the queue pops highest priority, and render tasks whose
   generation is stale (doc closed or zoom bucket changed) are skipped and counted, not executed.
   Text indexing and save are not cancelled by view-only generation changes.
3. **Binary delivery:** rendered pages/tiles/thumbnails are PNG-encoded in Rust and served over a
   custom `pdfr://` scheme as binary HTTP-style responses with immutable cache headers. No base64,
   no JSON pixels, no document bytes in the frontend.
4. **Byte-budgeted caches** (Rust): rendered pages+tiles, thumbnails, extracted text — separate
   budgets, LRU with stale-generation-first eviction, costs = `w*h*4` pre-encode estimate for
   bitmaps / byte length for encoded+text entries. Defaults: 256 MiB pages, 32 MiB thumbs,
   24 MiB text; low-memory mode (env `SPEEDYF_LOW_MEMORY=1` or settings) uses 96 MiB pages,
   16 MiB thumbs, and 12 MiB text.
5. **Document model in a framework-agnostic TS store** (not component state): stable page IDs,
   page order, per-page rotation, annotations (PDF-space coords), added text/image objects, form
   edits, selection, dirty/save state, undo/redo command stack. Bitmaps and PDF bytes never enter
   the store. On save, the store serializes a small **EditPlan** JSON; Rust materializes it with
   PDFium (import pages in order → rotations → annotations → text/image objects → form values).
6. **Safe save:** write to a temp sibling file → flush/close → reopen-verify with PDFium → atomic
   rename over destination; original preserved on any failure.
7. **Scale quantization:** device-pixel render scales snap to buckets
   (0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4 × base); pinch/zoom transforms existing rasters via CSS,
   then a debounced request fetches the bucketed raster.
8. **Tiling:** pages whose device-pixel area at the target scale exceeds a configurable threshold
   (default 16 Mpx ≈ 4096×4096) render as 1024-px tiles over a low-res whole-page backdrop instead
   of one huge bitmap.
9. **Progressive pipeline:** placeholder → low-res preview (thumbnail-scale raster, upscaled) →
   full bucketed raster → tiles when zoomed; neighbors prefetch at idle priority.
10. **Search:** engine thread extracts page text incrementally (low priority, visible-first);
    Rust normalizes (NFKC, case-fold, whitespace collapse, quote folding) with an offset map back
    to original char indices; queries run over whatever is indexed so far and re-run as indexing
    progresses; match rects come from PDFium char boxes.

## Expected performance characteristics

(Structural expectations, to be validated by the benchmark harness — no fabricated numbers.)

- **Startup:** small binary + system webview; no document work at launch; first paint is the empty
  shell. No bundled Chromium/Node.
- **Time to first page:** open = PDFium parse of xref + page count + first ~64 page sizes; first
  visible page renders immediately at current scale; thumbnails, remaining page sizes, and text
  indexing trail at lower priority.
- **Scroll:** only visible ± overscan pages are mounted; placeholders elsewhere; render results
  arrive priority-ordered; stale requests are skipped in the queue.
- **Zoom:** CSS-transform interim + debounced bucketed re-render; historical scales evicted by LRU
  budget, wrong-scale entries evicted first.
- **Memory:** hard byte budgets on all raster/text caches; webview image memory bounded by
  virtualization window size; no PDF byte buffers in JS.
- **Large docs:** page sizes hydrate lazily in batches; search/index work never outranks visible
  renders; 300–500 MB scanned files are read through PDFium's own on-demand object loading, and
  tiling caps single-allocation size.

## Licensing implications

| Component                                                                           | License                                 |
| ----------------------------------------------------------------------------------- | --------------------------------------- |
| PDFium (+ prebuilt binaries)                                                        | BSD-3-Clause (Apache-2.0 for some deps) |
| pdfium-render                                                                       | MIT OR Apache-2.0                       |
| Tauri 2 (+ plugins dialog/opener)                                                   | MIT OR Apache-2.0                       |
| SolidJS                                                                             | MIT                                     |
| Vite / Vitest / TypeScript                                                          | MIT / MIT / Apache-2.0                  |
| Rust crates (serde, crossbeam-channel, png, tempfile, thiserror, parking_lot, uuid) | MIT OR Apache-2.0                       |

No copyleft components. MuPDF explicitly avoided because of AGPL.

## Known risks and mitigations

1. **PDFium crash takes down the app (no process isolation in v1).** Mitigation: engine boundary is
   already message-based; roadmap item to move the engine behind a helper process pool. PDFium is
   hardened by Chrome-scale fuzzing, lowering (not eliminating) the risk.
2. **`pdfium-render` API gaps** (some annotation/form setters). Mitigation: raw `bindings()` FFI
   escape hatch; feature degrades gracefully (e.g., form fill reports unsupported).
3. **Highlight/annotation appearance streams:** some third-party viewers render standard annots
   without `/AP` fine (Acrobat regenerates), some don't. Mitigation: set full properties
   (color, CA, quad points, border); verify by reopen-render in tests. Ink is deliberately flattened
   to page path objects in the current save implementation.
4. **PNG encode adds CPU per render.** Mitigation: fast filter/compression settings; tiles bound
   worst-case size; revisit with shared-memory delivery if measured to matter (see below).
5. **pdfium dylib distribution:** fetch script pins a release tag; binary is a bundled resource;
   macOS notarization must include it in the signature (documented in README).
6. **WKWebView custom-scheme throughput** could bottleneck very fast scrolling. Mitigation:
   immutable caching headers, bucketed scales, prefetch; measured by the bench harness.

## Evidence that would justify changing the architecture later

- Benchmarks showing protocol delivery + PNG decode (not rasterization) dominating p95 page latency
  → move to shared-memory or native surface compositing.
- Crash reports attributing meaningful instability to in-process PDFium → helper-process render pool
  (interface already message-shaped).
- Webview baseline RSS or cold-start measured to dominate the budget on target hardware → revisit
  native Rust UI (alternative #4).
- PDFium compatibility failures on real-world corpora that PDF.js handles → enable the documented
  PDF.js fallback path behind the renderer abstraction.
