# SpeedyF PDF Viewer/Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Execution note for this session:** the user mandated fully autonomous single-session delivery and the harness forbids unprompted subagents, so this plan is executed **inline** by the planning agent itself. Task granularity below is phase-level with exact interfaces and verification commands; the authoritative requirements list is the user's spec (mirrored in `docs/architecture-decision.md`).

**Goal:** A working cross-platform desktop PDF viewer/editor (view, select, search, annotate, page-edit, save) with explicit memory bounds, virtualization, progressive rendering, cancellation, tests, benchmarks, and docs.

**Architecture:** Tauri 2 shell; SolidJS/TS UI holding only lightweight state; Rust host with a single dedicated PDFium engine thread (priority queue + generation cancellation), byte-budgeted LRU caches, `pdfr://` binary PNG delivery, EditPlan-based atomic save. See `docs/architecture-decision.md`.

**Tech Stack:** Tauri 2.9/2.5 (rust crate 2.x), tauri-plugin-dialog, pdfium-render 0.8.x + pdfium prebuilt (chromium/7961), SolidJS 1.9, Vite, TypeScript 5, Vitest, ESLint 9 flat + Prettier, crossbeam-channel, png, tempfile, thiserror, parking_lot, serde.

## Global Constraints

- Platforms: macOS, Windows, Linux (native build only for current OS: macOS arm64).
- No PDF bytes / bitmaps / large arrays in frontend state; no base64 pixel transport.
- Virtualized viewer: mount visible ± overscan only; placeholders elsewhere.
- Progressive: placeholder → low-res preview → bucketed full render → tiles (>16 Mpx device area, 1024 px tiles).
- Byte-budgeted caches (pages 256 MiB / thumbs 32 MiB / text 24 MiB; low-memory 96/16/12 MiB) with LRU + stale-first eviction.
- All engine work carries `generation`; stale render tasks are skipped and counted without cancelling search/save work.
- Scale buckets: 0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4 (device-pixel scale = zoom × DPR snapped up).
- Annotations stored in unrotated PDF crop-box space; stable IDs; undo/redo for every mutation; vector SVG overlay during interaction.
- Save: temp sibling → flush → reopen-verify → atomic rename; never silently overwrite.
- Security: narrow capabilities, no fs plugin, CSP on, no remote loads, PDF content untrusted.
- Honest limitations: no embedded-text rewriting, no OCR, no signatures, no secure redaction.
- Checks that must pass: `pnpm lint`, `pnpm format:check`, `pnpm typecheck`, `pnpm test`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (src-tauri), `pnpm tauri build`.

---

### Task 1: Repo scaffold + toolchain baseline

**Files:** `package.json`, `pnpm-workspace.yaml`(no — single package), `vite.config.ts`, `tsconfig.json`, `eslint.config.js`, `.prettierrc.json`, `.prettierignore`, `index.html`, `src/main.tsx`, `src/app/App.tsx` (shell stub), `src/styles/global.css`, `scripts/fetch-pdfium.mjs`, `scripts/make-icon.py`, `src-tauri/{Cargo.toml,build.rs,tauri.conf.json,capabilities/default.json,src/main.rs,src/lib.rs}`, `.gitignore`, `.editorconfig`.

**Interfaces produced:**
- `pnpm dev` / `pnpm tauri dev` run; `pnpm tauri icon` generated icons.
- `fetch-pdfium.mjs` downloads pinned `chromium/7961` archive for `{darwin,linux,win32}×{arm64,x64}` into `src-tauri/pdfium/` (gitignored), exposing `libpdfium.dylib|.so|pdfium.dll`.
- Steps: write configs → `pnpm install` → icon gen → fetch pdfium → `cargo check` in src-tauri → commit.

**Verify:** `pnpm typecheck` passes; `cargo check` passes; `node scripts/fetch-pdfium.mjs` idempotent.

### Task 2: Rust engine foundation (no UI yet)

**Files:** `src-tauri/src/{errors.rs, engine/mod.rs, engine/types.rs, engine/queue.rs, engine/worker.rs, engine/pdfium_init.rs, cache/mod.rs}`.

**Interfaces produced (consumed by Tasks 3–5, 8):**
```rust
// engine/types.rs
pub struct DocId(pub u32); pub struct Generation(pub u64);
pub enum Priority { VisiblePage=0, VisibleTile, AdjacentPage, VisibleThumb, NearThumb, TextExtract, Prefetch } // lower=more urgent
pub struct RenderSpec { pub doc: DocId, pub src_index: u16, pub rotation: u16, pub scale_milli: u32, pub tile: Option<TileRect>, pub kind: RenderKind }
pub enum EngineTask { OpenDoc{..}, CloseDoc{..}, Render{spec, reply}, PageSizes{..}, ExtractText{..}, Save{plan, reply}, FormFields{..}, SetFormValue{..} }
pub struct EngineHandle { /* Sender<PrioritizedTask> + generation registry + metrics */ }
impl EngineHandle { pub fn submit(&self, prio: Priority, gen: Generation, task: EngineTask); pub fn bump_generation(&self, doc: DocId) -> Generation; pub fn metrics(&self) -> EngineMetrics; }
```
- `cache::ByteLruCache<K>`: `insert(k, bytes, cost)`, `get`, `set_budget(bytes)`, `evict_to_budget()`, stale-first via `mark_stale(pred)`; unit-tested without pdfium.
- `queue::PriorityQueue`: pop order (priority, then FIFO seq); `skip_if_stale(current_gens) -> skipped_count`; unit-tested without pdfium.
- Worker thread: leaks one `Pdfium` binding (`Box::leak`) at startup; owns `HashMap<DocId, PdfDocument<'static>>`.

**Verify:** `cargo test` — queue ordering, generation skip counting, LRU cost/eviction/budget tests pass.

### Task 3: Open/render/protocol vertical slice

**Files:** `src-tauri/src/commands/{mod.rs,document.rs}`, protocol registration in `lib.rs`, `src/lib/transport/engine.ts`, `src/lib/rendering/renderSource.ts`, minimal `Viewer` that shows page 1.

**Interfaces produced:**
- Commands: `open_document(path) -> DocMeta{docId, name, path, pageCount, sizes: [(w,h)…first 64], encrypted?}`, `close_document(docId)`, `more_page_sizes(docId, from, count)`, `doc_generation(docId)`.
- Protocol: `pdfr://render/?doc&src&rot&scale&gen&kind=page|thumb|tile&tx&ty&tw&th` → `200 image/png` + `Cache-Control: immutable` | `204` stale | `404`.
- TS: `engine.open(path): Promise<DocMeta>`; `renderSource.pageUrl(spec): string`.

**Verify:** `pnpm tauri dev`, open fixture PDF, first page visible; second identical request served from cache (metrics hit count via `engine_metrics` command).

### Task 4: Document model + coordinates (pure TS, TDD)

**Files:** `src/types/model.ts`, `src/features/document/documentStore.ts`, `src/lib/coordinates/{coords.ts,layout.ts}`, tests beside each.

**Interfaces produced:**
```ts
type PageId = string;
interface PageEntry { id: PageId; srcIndex: number | null; /* null = blank page */ baseRotation: 0|90|180|270; userRotation: 0|90|180|270; widthPt: number; heightPt: number; sizeKnown: boolean }
interface DocumentModel { docId: number; path: string|null; name: string; pages: PageEntry[]; annotations: Record<PageId, Annotation[]>; formEdits: Record<string,string>; dirty: boolean; saving: boolean; generation: number; selected: Selection|null }
type Annotation = { id: string; pageId: PageId; kind: 'highlight'|'ink'|'rect'|'textbox'|'note'; rect: PdfRect; color: string; opacity: number; strokeWidth?: number; quads?: PdfQuad[]; points?: PdfPoint[][]; text?: string; fontSizePt?: number }
// store API (command pattern):
apply(op: EditOp): void; undo(): boolean; redo(): boolean; canUndo/RedoSig; markSaved(): void;
// ops: reorderPage, deletePage, duplicatePage, rotatePage, addBlankPage, addAnnotation, patchAnnotation, deleteAnnotation, setFormValue, addTextBox(=annotation kind textbox), addImage
interface AddedImage { id, pageId, rect, sourcePath, naturalW, naturalH }
// coords.ts — pure fns:
pdfToPageCss(pt, page, zoom): Css; cssToPdf(css, page, zoom): Pt;  // handles rotation 0/90/180/270 + cropbox origin + y-flip
pageLayout(pages, zoom, gap): Layout{offsets[], total}; visibleRange(layout, scrollTop, viewH, overscan): [i,j];
fitPageZoom(page, viewport): number; fitWidthZoom(...); tileGrid(pageDevW, pageDevH, tilePx): TileRect[];
anchorScroll(layout, prevZoom, nextZoom, anchorCss): number;
```

**Verify:** `pnpm test` — reorder/delete/dup/rotate/blank, undo/redo, dirty transitions, coords at zooms {0.5,1,2.5}, rotations {90,180,270}, DPR {1,2}, cropbox offsets, tile grids, fit modes, anchor stability.

### Task 5: Virtualized viewer + toolbar + sidebar + progressive rendering

**Files:** `src/features/viewer/{Viewer.tsx,PageView.tsx,TextLayer.tsx}`, `src/features/viewer/viewportStore.ts`, `src/components/{Toolbar.tsx,Sidebar.tsx,IconButton.tsx,StatusBar.tsx}`, `src/app/{App.tsx,shortcuts.ts,theme.ts}`.

**Behavior:** continuous scroll over `layout`; placeholders; `<img src=pdfr://…>` preview→full swap; tiles at high zoom; thumbnails lazy via same protocol (`kind=thumb`); current-page tracking; zoom controls + fit modes + Cmd/Ctrl±; rotate view; page input; drag-drop open; theme light/dark/system; all toolbar buttons functional with tooltips/aria/disabled/focus-visible.

**Verify:** manual dev-run against a many-page fixture: scroll stays responsive, far pages unmount (DOM count bounded), zoom keeps anchor, `engine_metrics` shows skipped stale tasks > 0 after fast scroll; keyboard shortcuts work; inputs don't leak shortcuts.

### Task 6: Text layer, selection, incremental search

**Files:** Rust `src-tauri/src/{search/mod.rs,commands/search.rs}` + `engine/text.rs`; TS `src/features/search/{searchStore.ts,SearchPanel.tsx}`, TextLayer wiring.

**Interfaces:**
- Rust: `get_text_layout(docId, srcIndex) -> {runs: [{str, x,y,w,h, fontSizePt}]}` (cached, budget-bounded); `ensure_indexed(docId, upTo?)` emits `search:progress {indexed, total}`; `search_query(docId, q, caseSensitive) -> [{srcIndex, matches:[{start,len,rects,snippet}]}]` over indexed pages; normalization NFKC+fold+ws-collapse+quote-fold with offset map.
- TS: debounced query (250 ms), incremental re-query on progress events, match list by page, n/N navigation, case toggle, highlight rects overlay, image-only message when indexing complete with 0 extractable chars.

**Verify:** `cargo test` normalization/matching/offset-map; dev-run: type query → first results before indexing completes on large fixture; selection copies text.

### Task 7: Annotations + editing UI + AcroForm fill

**Files:** `src/features/annotations/{AnnotationLayer.tsx,toolStore.ts}`, `src/features/editor/{EditMode.tsx pieces in Sidebar/Toolbar}`, Rust `commands/form.rs`.

**Behavior:** five tools (highlight from text selection or drag, ink smoothing path, rect, textbox with inline edit, note with popover); select/move/resize(rect,textbox,image)/delete; opacity + stroke width controls; Delete/Backspace; Escape exits tool; all via store ops (undo/redo, dirty). Edit mode: sidebar drag-reorder, per-page rotate/delete/duplicate buttons, add blank page, add image (dialog → path → Rust returns natural size), form field list panel with text inputs (if engine supports; else "unsupported" notice).

**Verify:** `pnpm test` annotation ops; dev-run alignment across zoom/rotation/scroll.

### Task 8: Save/export pipeline

**Files:** Rust `src-tauri/src/{engine/save.rs,commands/save.rs}`; TS `src/features/document/saveController.ts`, unsaved-changes close guard.

**Interfaces:**
```rust
pub struct EditPlan { pub pages: Vec<PlanPage>, pub form: Vec<(String,String)> }
pub struct PlanPage { pub src_index: Option<u16>, pub size_pt: (f32,f32), pub rotation: u16, pub annots: Vec<PlanAnnot>, pub texts: Vec<PlanText>, pub images: Vec<PlanImage> }
save_document(docId, plan, destPath?) -> SaveResult { path, bytes, durationMs }
```
- Pipeline: new doc → import pages by order (FPDF_ImportPages) → rotations → blank pages → annots (Highlight/Square/Text-note via FPDFAnnot; ink flattened to page path objects; textbox/added text as page text objects; images as image objects) → form values → save to temp sibling → reopen-verify (page count + openable) → atomic rename; on failure remove temp, original untouched. Dialogs: native save-as; overwrite confirm handled by OS dialog.

**Verify:** `cargo test` save pipeline on generated fixtures (page count/order/rotation verified by reopen; annotation count via FPDFPage_GetAnnotCount); atomicity test (verify-failure injection leaves original intact). Dev-run: edit → Save As → reopen output in app + `Preview.app`.

### Task 9: Bench harness

**Files:** `src-tauri/src/bin/bench.rs`, `bench/README.md`, `bench/corpus/.gitkeep` (gitignored contents), `scripts/bench.sh`.

**Behavior:** categories from spec (text-1000p, scanned-large, cad-page, image-100p, malformed, edited-save) matched by filename patterns in `bench/corpus/`; measures open ms, first-page render ms, p50/p95 page + tile render, text-extract ms/page, first-search-result ms, save ms + peak RSS delta, cache hit rate, skipped tasks; emits `bench/results/<timestamp>.json`; skips categories with no local file (no fabricated numbers).

**Verify:** `cargo run --bin bench` on programmatically generated corpus subset produces JSON.

### Task 10: Full check matrix + packaging + docs + CI

**Files:** `README.md`, `CONTRIBUTING.md`, `docs/{performance.md,security.md}`, `.github/workflows/ci.yml`.

- Run and fix: `pnpm lint`, `pnpm format:check`, `pnpm typecheck`, `pnpm test`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `pnpm tauri build`.
- Launch built app; confirm alive; kill.
- Final review sweep: object-URL/listener leaks, stale-task handling, fake controls, aria/tooltips, focus states.

**Verify:** every command above exits 0 (or documented external blocker); README matches reality.

## Self-review (performed)

- Spec coverage: toolbar list → Task 5; annotations list → Task 7; editing list → Task 7/8; model → Task 4; coords → Task 4; rendering API → Tasks 2/3; save → Task 8; shortcuts → Task 5; search → Task 6; caches/cancellation → Task 2/3; benches → Task 9; tests → Tasks 2,4,6,7,8; docs/CI → Task 10; security → Tasks 1,10.
- Type consistency: `RenderSpec/scale_milli` ↔ `renderSource.pageUrl`; `PageEntry.srcIndex:null` ↔ `PlanPage.src_index:Option` ↔ blank pages; `Annotation.kind` ↔ `PlanAnnot` variants.
- No placeholder steps remain; commands are exact.
