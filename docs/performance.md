# Performance model and measurement

SpeedyF optimizes perceived document latency under explicit memory bounds. It
does not try to maximize aggregate PDFium throughput by rendering every page
in parallel; PDFium is serialized on one engine thread and the queue ensures
that the next useful visible artifact wins.

## Runtime pipeline

Opening a document parses its cross-reference data, reads page count, and
hydrates at most the first 64 page sizes. Remaining sizes arrive in batches.
The first mounted page immediately requests a low-resolution preview and its
bucketed full render. Thumbnails and incremental text extraction use lower
priorities.

The viewer mounts only the visible range plus one viewport of overscan. A zoom
gesture CSS-scales the current raster, snaps the device scale to one of
`0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4`, waits briefly for the gesture to
settle, bumps the document generation, and requests that bucket. Old queued
jobs become stale and are skipped before rendering.

Pages larger than roughly 16.7 million pixels at the target scale use
1024-pixel tiles. Only viewport-adjacent tiles mount over a backdrop capped
near 2 million pixels, so a CAD sheet or deep zoom cannot require one enormous
bitmap allocation.

## Memory budgets

Rust stores encoded PNG bytes but charges each raster by its approximate
decoded RGBA cost (`width × height × 4`). This is deliberately conservative.

| Cache             |  Normal | Low-memory |
| ----------------- | ------: | ---------: |
| Pages and tiles   | 256 MiB |     96 MiB |
| Thumbnails        |  32 MiB |     16 MiB |
| Text/search index |  24 MiB |     12 MiB |

The two raster caches use LRU eviction, preferring entries flagged stale after
a generation bump. The search index stops adding pages when its byte budget is
exhausted and reports partial coverage instead of growing without limit.

These budgets do not represent total process RSS. PDFium document structures,
the OS webview, decoded images held by mounted `<img>` elements, native
libraries, and allocator fragmentation are additional. Virtualization bounds
the webview portion structurally, but only process-level measurement captures
the whole application.

## Engine diagnostics

The status bar polls the narrow `engine_metrics` command and exposes:

- cumulative cache hits/lookups;
- stale tasks skipped;
- total successful render requests; and
- current queue depth in the tooltip.

The command also reports page/thumbnail/text bytes and budgets plus indexed
page count. Counters are process-wide and cumulative, so compare deltas around
a controlled interaction rather than treating the current total as a rate.

## Benchmark harness

`pnpm bench:release` runs `src-tauri/src/bin/bench.rs` against local files in
`bench/corpus/` and writes `bench/results/<unix-ms>.json`. It uses the same
`EngineHandle`, priority queue, caches, rendering path, text extraction, search
index, and EditPlan save path as the app.

Measured fields include:

- PDFium open latency;
- first-page render latency at a 1.5 device scale;
- p50/p95 whole-page renders spread across up to 20 pages;
- p50/p95 768-pixel tiles spread across up to 12 pages;
- extraction time per page for up to 1,000 pages;
- time/pages to a known first search result;
- cache-hit rate from exact render-key replays;
- skipped work after an intentional generation bump; and
- identity EditPlan save time/output size and peak-RSS high-water delta.

See `bench/README.md` for corpus naming and query sidecars. Categories without a
local file are emitted as `skipped`. A missing search query omits only that
field. The harness never fills missing values with targets or estimates.

### Comparison rules

Only compare reports when all of these match:

- `buildProfile` is `release`;
- OS, architecture, CPU power mode, and available memory;
- exact corpus bytes and query sidecars;
- warm/cold filesystem-cache state;
- PDFium tag and SpeedyF revision; and
- background load and thermal state.

Peak RSS is a process high-water mark. Its save delta can correctly be zero if
an earlier render established a higher peak. It is not the instantaneous
allocation made by save.

No third-party corpus or canonical performance baseline is committed. The
latest local verification values belong in the task/release report with their
JSON path; they should not be generalized to other PDFs or machines.

## Manual profiling

Use an optimized app for representative profiling:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
pnpm tauri build
```

On macOS, Instruments’ Time Profiler and Allocations templates can separate
PDFium raster time, PNG encoding, webview decode/layout, and save-time peaks.
On Linux, use `perf`/heaptrack; on Windows, use Windows Performance Recorder or
Visual Studio Profiler. Preserve the same corpus and interaction script when
comparing changes.

For scroll/zoom investigations:

1. Record a metrics snapshot.
2. Rapidly cross several scale buckets and pages.
3. Wait for queue depth to reach zero.
4. Record the next snapshot.
5. Confirm stale skips increased and replaying an exact page increases both
   lookups and hits.

## Known performance risks

- PNG encoding costs CPU even with fast compression/filter settings.
- WKWebView/WebView2/WebKitGTK custom-protocol delivery and image decode may
  dominate once PDFium rendering is very fast.
- One PDFium thread limits aggregate throughput, especially during large saves,
  but prevents unsupported concurrent access and enables deterministic
  priority.
- Import-based save can temporarily hold both source and output documents.
- The webview still has a non-zero baseline even though Tauri does not bundle
  Chromium.

Architecture changes require evidence:

- protocol/decode dominating p95 → evaluate shared memory/native surfaces;
- repeatable PDFium crashes → move the message-shaped engine behind helpers;
- webview baseline dominating the budget → reconsider a native Rust UI; or
- significant corpus compatibility gaps → evaluate a renderer fallback behind
  the existing transport abstraction.
