# Contributing to SpeedyF

SpeedyF’s main constraint is not feature count; it is preserving a responsive,
memory-bounded document pipeline while editing untrusted files safely. Read
`docs/architecture-decision.md` and the active implementation plan before
changing an engine or model boundary.

## Development setup

Install Node 20+, pnpm 9, Rust 1.82+, and the
[Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/).
Then:

```bash
pnpm install --frozen-lockfile
pnpm fetch-pdfium
export PATH="$HOME/.cargo/bin:$PATH"
pnpm tauri dev
```

PDFium is pinned by `scripts/fetch-pdfium.mjs`. Do not commit the native
library or local benchmark corpora.

## Architectural invariants

Changes must preserve these rules:

1. Every PDFium handle and every PDFium call stays on the dedicated engine
   thread. Do not make “small” PDFium calls from Tauri command threads.
2. PDF bytes, raw bitmaps, and large typed arrays do not enter frontend state.
   Rendered PNGs use the `pdfr://` transport; image previews use binary IPC.
3. Visible work keeps priority over thumbnails, text indexing, and prefetch.
   Long background jobs must yield in page-sized units.
4. Render URLs include an exact integer scale bucket and document generation.
   A superseded generation must be rejected or skipped, not rendered anyway.
5. Raster and text caches are charged in bytes and have hard budgets. Do not
   replace them with entry-count limits or unbounded browser caches.
6. Page/PDF/CSS rotation and y-axis conversion belongs in
   `src/lib/coordinates/` (frontend) or the engine page-space mapper (save).
7. All model mutations use `documentStore.apply(EditOp)` and have a valid
   inverse. Components do not own an alternate copy of document state.
8. Rust’s `PlanPage.rotation` is an extra rotation applied after source-page
   import. It is not an absolute angle.
9. Save remains temp-sibling → flush/sync → PDFium reopen verification →
   atomic replacement. A failure must leave the destination unchanged.
10. Frontend filesystem access stays behind narrow Rust commands/native
    dialogs; do not add a broad filesystem capability for convenience.

## Test-driven changes

For new behavior:

1. Add the smallest test that fails for the intended reason.
2. Run that focused test and capture the failure.
3. Implement the behavior.
4. Re-run the focused test, then the affected suite.
5. Run the full release gate before describing the change as complete.

Use generated PDF fixtures in Rust tests. Tests must not depend on copyrighted,
private, or network-fetched documents. Any test that loads PDFium must use the
bundled test lock so PDFium calls do not run concurrently.

Useful focused commands:

```bash
pnpm test -- src/features/document/documentStore.test.ts

export PATH="$HOME/.cargo/bin:$PATH"
cargo test --manifest-path src-tauri/Cargo.toml engine::save::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --bin bench
```

## Release gate

Run every command from the repository root:

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

For changes to rendering, coordinates, search, editing, dialogs, or saving,
also run the native smoke workflow described in the implementation plan.
Exercise a rotated page, a blank page, Save As, and reopen.

## Benchmarks

Benchmark changes with `pnpm bench:release`; see `bench/README.md`. Keep corpus
files and `bench/results/` local. Never copy numbers from a different machine
or substitute estimates for skipped fields. Include the report path, build
profile, corpus identity/size, and OS/architecture when discussing a result.

## Style and review

- Use Prettier for TypeScript/CSS/Markdown/JSON and `cargo fmt` for Rust.
- Keep Tauri commands typed and narrow; serialize DTOs as camelCase.
- Treat warnings as errors. Avoid `allow` attributes unless the warning is
  demonstrably incorrect and the reason is documented at the use site.
- Keep `unsafe` blocks at FFI boundaries, as small as possible, with a safety
  explanation.
- Clean up listeners, timers, observers, pending async registrations, and Blob
  URLs on teardown/document replacement.
- Controls need an accessible name, tooltip where meaning is icon-only,
  keyboard/focus behavior, disabled state, and no placeholder action.
- Preserve unrelated local work; do not regenerate lockfiles without a
  dependency change.

## Pull-request checklist

- [ ] New logic was demonstrated failing before implementation.
- [ ] Unit/integration tests cover success and failure paths.
- [ ] Rotated/cropped-page coordinate behavior was considered.
- [ ] Memory budgets and stale-task behavior remain bounded.
- [ ] Save failure cannot modify the original.
- [ ] New file/path access is documented in `docs/security.md`.
- [ ] User-facing behavior and limitations are reflected in `README.md`.
- [ ] The full release gate passes on the contributor’s platform.
- [ ] No native libraries, benchmark PDFs/results, secrets, or signing
      credentials are committed.

Project code is MIT-licensed. By contributing, you agree that your contribution
can be distributed under that license.
