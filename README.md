# SpeedyF

SpeedyF is a local-first desktop PDF viewer and focused editor built with
Tauri 2, Rust/PDFium, and SolidJS. It is designed around fast first-page
rendering, bounded caches, smooth large-document navigation, and safe saves
without moving PDF bytes or page bitmaps through JavaScript.

The current `0.1.0` foundation is usable on macOS and has cross-platform build
coverage for Windows and Linux. The full native interaction smoke test has
been performed on macOS 15.5 arm64; Windows and Linux still need equivalent
manual QA before a public release.

## What works

- Native open/save dialogs and PDF drag-and-drop
- Continuous virtualized scrolling with progressive page rendering
- Fit page, fit width, anchored zoom, whole-view rotation, and large-page tiles
- Virtualized thumbnails and direct page navigation
- Selectable PDF text and incremental, normalized search
- Hover previews for internal PDF links and DOI/arXiv citations, with optional
  local-library first-page previews
- Highlights, freehand ink, rectangle annotations, text boxes, and notes
- Annotation select, move, resize, delete, undo, and redo
- Page drag-reorder, delete, duplicate, rotate, and add blank page
- Add PNG/JPEG images and real PDF text objects
- Best-effort AcroForm text-field editing
- Save As followed by automatic reopen of the materialized document
- Light/dark/system themes and a lower-memory cache mode
- Live status counters for cache hits and skipped stale render work

## Quick start

Prerequisites:

- Node.js 20 or newer (the project is developed with Node 24)
- pnpm 9
- Rust 1.82 or newer
- `curl` and `tar` for the pinned PDFium download
- macOS: Xcode Command Line Tools
- Windows: Microsoft C++ Build Tools and WebView2
- Linux: WebKitGTK 4.1 and the other
  [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/)

On Debian/Ubuntu, the current Tauri prerequisites are:

```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Install dependencies and fetch the pinned `chromium/7961` PDFium binary:

```bash
pnpm install --frozen-lockfile
pnpm fetch-pdfium
```

Launch the desktop app:

```bash
export PATH="$HOME/.cargo/bin:$PATH" && cd /Users/dvdkm/Documents/code/speedyf && pnpm tauri dev
```

The Vite development port is fixed at `1420`. To load PDFium from a custom
location, set `SPEEDYF_PDFIUM_PATH` to either the library file or its
directory. Set `SPEEDYF_LOW_MEMORY=1` to start with reduced cache budgets.

Build native bundles for the current operating system:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
pnpm tauri build
```

Unsigned local bundles are written below
`src-tauri/target/release/bundle/`. Distribution builds must be signed on
their target platform; on macOS the bundled `libpdfium.dylib` must be covered
by the app signature and the final app/DMG notarized.

## Interaction guide

Open a PDF with the toolbar, the system dialog, or by dropping it onto the
window. Enable **Edit mode** to reveal per-thumbnail page controls and image
insertion. Page rotation in the sidebar changes saved page rotation; **Rotate
view** in the main toolbar is temporary display state.

Annotation tools use PDF page coordinates, so marks stay aligned through zoom,
scroll, and 90°/180°/270° rotations. Double-click text boxes or notes to edit
their content. Select an annotation and press Delete/Backspace to remove it.

| Action                     | macOS               | Windows/Linux             |
| -------------------------- | ------------------- | ------------------------- |
| Open                       | `⌘O`                | `Ctrl+O`                  |
| Save / Save As             | `⌘S` / `⇧⌘S`        | `Ctrl+S` / `Ctrl+Shift+S` |
| Search                     | `⌘F`                | `Ctrl+F`                  |
| Undo / redo                | `⌘Z` / `⇧⌘Z`        | `Ctrl+Z` / `Ctrl+Y`       |
| Zoom in/out                | `⌘+` / `⌘−`         | `Ctrl++` / `Ctrl+−`       |
| Fit page / 100%            | `⌘0` / `⌘1`         | `Ctrl+0` / `Ctrl+1`       |
| Previous/next page         | Page Up / Page Down | Page Up / Page Down       |
| Close panel/tool/selection | Escape              | Escape                    |

Search starts returning results from the pages already indexed instead of
waiting for the whole document. Scanned PDFs without an embedded text layer
are viewable but not searchable because OCR is not implemented.

Hover a linked citation, section, figure, or table for a destination crop.
Choose **Citation library…** in the status bar to index a local folder of PDFs;
recognized DOI/arXiv links can then show the matching paper's first page. The
library is local-only and can be disabled from the same control.

## Architecture

```text
SolidJS webview (lightweight model, virtualized DOM, vector overlays)
       │ typed invoke/events                    ▲ binary PNG responses
       ▼                                        │
Tauri Rust host ── pdfr:// protocol ── byte-budgeted raster/text caches
       │ priority + generation-stamped messages
       ▼
one dedicated engine thread (sole owner of every PDFium handle)
```

The engine boundary is deliberately message-shaped:

1. Visible pages and tiles outrank hover previews; hover previews outrank
   thumbnails, text extraction, prefetch, and idle library scanning.
2. A zoom bucket change increments the document generation; queued work from
   older generations is skipped before PDFium touches it, while PDFium's
   progressive-render pause callback aborts a stale render already in flight.
3. Page, thumbnail, and hover-preview caches charge the encoded PNG bytes they
   actually retain and use stale-first O(log n) LRU eviction. Hover traffic has
   a separate cache, so it cannot evict pages being read. Mounted-page
   virtualization separately bounds decoded webview images.
4. Rust serves encoded PNG bytes through `pdfr://`; no base64, JSON pixels, or
   PDF buffers are held in frontend state.
5. The frontend mounts only visible pages plus an overscan window. Pages over
   about 4 million device pixels use 1024-pixel tiles culled in both axes.
   Complex tiled sheets skip the blocking whole-page preview.
6. Save serializes a small EditPlan. Rust imports source pages in the requested
   order, creates blanks, applies rotation/annotations/text/images/forms,
   writes a sibling temporary file, reopens it with PDFium, verifies the page
   count, and atomically replaces the destination.
7. Search keeps compact normalized UTF-8 text and sparse source offsets under
   its own hard budget. Selectable text geometry lives in an independent
   foreground LRU, so search truncation never disables selection or highlights.
   Query responses are globally capped and report when results were omitted.

See [the architecture decision](docs/architecture-decision.md) for the full
trade study and [the implementation plan](docs/superpowers/plans/2026-07-20-speedyf-pdf-viewer-editor.md)
for interface-level details.

### Why this stack is leaner

Tauri uses the operating system webview instead of shipping Chromium and Node
as Electron does. SolidJS updates the DOM without a virtual-DOM render loop.
PDFium renders and edits in Rust, so the app avoids PDF.js/Canvas heap spikes
and avoids parsing the same document again in a separate JavaScript editing
library. A pure-native Rust UI could reduce webview overhead further, but it
would make high-quality text selection, IME, accessibility, and controls much
more expensive to build. MuPDF was rejected because its AGPL/commercial
licensing does not fit this foundation.

The engine is currently a single thread because PDFium is not thread-safe.
That is intentional serialization, not a throughput oversight: priority and
cancellation keep the thread focused on visible work. If crash reports justify
isolation later, the same message boundary can move behind helper processes.

## Verification

The release gate is:

```bash
pnpm lint
pnpm format:check
pnpm typecheck
pnpm test

export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml

pnpm tauri build
```

Generate the deterministic UI smoke fixture with:

```bash
pnpm fixture:smoke -- /tmp/speedyf-smoke.pdf
```

The Rust save tests generate their own PDFs in code and reopen the output with
PDFium to verify page order, duplication, blanks, rotation, annotations, added
text, flattened ink paths, and atomic failure behavior.

## Benchmarks

Put private/local PDFs in `bench/corpus/` and run:

```bash
pnpm bench:release
```

Reports are written to the gitignored `bench/results/` directory. Missing
categories are explicitly skipped, never synthesized. See
[bench/README.md](bench/README.md) for naming, search-query sidecars, fields,
and interpretation, and [docs/performance.md](docs/performance.md) for the
performance model.

## Project map

- `src-tauri/src/engine/` — PDFium worker, queue, rendering, text, and save
- `src-tauri/src/cache/` — encoded-byte-cost, stale-first LRU implementation
- `src-tauri/src/search/` — incremental NFKC search index and source offsets
- `src-tauri/src/library/` — contained local-library index and citation resolver
- `src-tauri/src/lib.rs` — Tauri app, custom protocol, and command registration
- `src/features/viewer/` — virtualization, progressive raster, tiles, text layer
- `src/features/document/` — document model, undo/redo, EditPlan, workflows
- `src/features/annotations/` — pointer mapping and vector edit overlays
- `src/features/citations/` — link hotspots, dwell state machine, and popover
- `src/lib/coordinates/` — the only page/PDF/CSS rotation conversion layer
- `src/lib/transport/engine.ts` — the frontend’s typed engine boundary
- `docs/` — architecture, performance, security, and implementation plan

## Current limitations

SpeedyF is a focused editor, not a full PDF authoring or forensic tool:

- Existing embedded text cannot be rewritten; text boxes add new text objects.
- Citation previews require real PDF link annotations or explicit DOI/arXiv
  text. External-paper previews resolve only against the configured local
  library; SpeedyF performs no metadata lookup or download.
- There is no OCR, signature creation/validation, encryption-preserving save,
  or secure redaction. Drawing an opaque shape does **not** remove underlying
  text or image data.
- Saving a signed PDF creates a new document and should be assumed to invalidate
  signatures. Password protection is not preserved in the output.
- Existing annotations remain in imported pages and render, but are not loaded
  into the editable overlay.
- Viewing, rendering, selection, and search use 32-bit source indexes. Save is
  limited to 65,535 output pages; the toolbar and status bar disclose and
  enforce that limit as soon as a larger document opens.
- Ink is intentionally flattened to page path objects on save; it is not saved
  as an editable PDF Ink annotation.
- Only AcroForm text fields are best-effort editable. Other widgets are listed
  read-only, and advanced forms/XFA are unsupported.
- Because save rebuilds a PDF from imported pages, preservation of bookmarks,
  attachments, document JavaScript, metadata, advanced forms, and unusual
  document-level structures is not guaranteed.
- PDFium and image decoding run in-process. A native decoder crash terminates
  the app; helper-process isolation is a future hardening milestone.
- No autosave, collaboration, cloud sync, or recovery journal is included.

Read [docs/security.md](docs/security.md) before using untrusted documents in a
high-assurance environment.

## License and contribution

Project code is MIT-licensed. PDFium is BSD-3-Clause with permissively licensed
dependencies; `pdfium-render`, Tauri, and SolidJS are permissively licensed.
See [CONTRIBUTING.md](CONTRIBUTING.md) for invariants and the pull-request
checklist.
