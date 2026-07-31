# SpeedyF benchmark corpus

The `bench` binary drives the same queued Rust/PDFium engine as the desktop app.
It writes a machine-readable report to `bench/results/<unix-ms>.json`; corpus
PDFs and reports stay local and are gitignored.

## Corpus contract

Put PDFs directly in `bench/corpus/`. A filename must contain one of these
case-insensitive category tokens:

| Token           | Intended document                                       |
| --------------- | ------------------------------------------------------- |
| `text-1000p`    | About 1,000 text-heavy pages                            |
| `scanned-large` | A large, scanned/image-only document                    |
| `cad-page`      | A complex single-page vector/CAD drawing                |
| `image-100p`    | About 100 image-heavy pages                             |
| `malformed`     | A deliberately corrupt or truncated PDF                 |
| `edited-save`   | A valid document used for identity EditPlan save timing |
| `text-70000p`   | Optional >65,535-page source-index overflow probe       |

For first-result search timing, add a same-stem UTF-8 sidecar containing a
known query on its first non-empty line. For example:

```text
bench/corpus/text-1000p-contract.pdf
bench/corpus/text-1000p-contract.query
```

The harness never chooses a convenient query from already-extracted text. If
the sidecar is absent, only that metric is omitted and the JSON explains why.
If a category is absent, its report has `status: "skipped"` rather than made-up
numbers. When several PDFs match a category, the lexically first is used and
the report records that selection.

## Run

From the repository root:

```bash
pnpm bench:release
```

`pnpm bench` is useful for a quick harness check, but its JSON is labeled
`buildProfile: "debug"` and should not be used for performance comparisons.

To use directories outside the repository:

```bash
pnpm bench:release -- --corpus /absolute/corpus --results /absolute/results
```

For a deterministic, small local verification subset:

```bash
pnpm fixture:smoke -- bench/corpus/edited-save-smoke.pdf --with-query
pnpm bench:release
```

## Measurements

For each valid category, the report contains:

- PDFium open latency and first-page render latency;
- p50/p95 whole-page and 768-pixel tile render latency;
- incremental text-extraction time per page (up to 1,000 pages);
- selectable run/character counts on the last measured page and its cached
  replay latency;
- compact search-index page coverage, truncation state, and retained bytes;
- time and page count to the first real indexed search result;
- cache-hit count/rate from exact render replays plus encoded cache occupancy;
- for `text-1000p`, a 160-page 2× forward sweep followed by a reverse
  scroll-back pass, with actual hit count/rate;
- stale queued tasks skipped after an explicit generation bump; and
- for `cad-page`, a generation bump during an uncached whole-page render,
  whether PDFium aborted it as stale, and time until a new visible tile
  completed; and
- for `edited-save`, identity EditPlan save time, output bytes, and process
  peak-RSS before/after/delta when the OS exposes it.

Page render samples are spread across the document (up to 20 pages); tile
samples use up to 12 pages. Peak RSS is a process high-water mark, so its delta
can truthfully be zero when an earlier operation established a higher peak.
Compare reports only when machine, build profile, corpus bytes, and thermal
conditions are equivalent.

When `text-70000p` is present, `maxPageRenderSource` proves whether the sampler
successfully reached the document tail using a 32-bit source index. The
category also times first and cached form-field discovery, which should return
immediately for a document with no AcroForm instead of walking 70,000 pages.
Save is still intentionally limited to 65,535 output pages.
