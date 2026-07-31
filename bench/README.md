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
- time and page count to the first real indexed search result;
- cache-hit count/rate from exact render replays;
- stale queued tasks skipped after an explicit generation bump; and
- for `edited-save`, identity EditPlan save time, output bytes, and process
  peak-RSS before/after/delta when the OS exposes it.

Page render samples are spread across the document (up to 20 pages); tile
samples use up to 12 pages. Peak RSS is a process high-water mark, so its delta
can truthfully be zero when an earlier operation established a higher peak.
Compare reports only when machine, build profile, corpus bytes, and thermal
conditions are equivalent.
