# Security model

SpeedyF is a local desktop application that parses attacker-controlled PDFs and
images with native libraries. Its v0.1 security posture is appropriate for a
developer foundation and ordinary local documents, not a hardened sandbox for
targeted hostile files or a tool for cryptographic/forensic workflows.

## Assets and trust boundaries

Protected assets include local document contents, filesystem integrity,
unsaved edits, and user privacy. The local user and packaged application code
are trusted. PDFs, passwords, embedded PDF structures, added images, filenames,
and drag-and-drop input are untrusted.

```text
untrusted PDF/image
        │
        ▼
native PDFium/image decoders (same process)
        │ owned DTOs / encoded PNG only
        ▼
Tauri command + pdfr:// boundary
        │
        ▼
SolidJS webview (no direct filesystem capability)
```

PDFium handles never cross the engine thread. The frontend receives page
metadata, text runs/rectangles, form metadata, and encoded image responses—not
raw native pointers, PDF bytes, or render buffers.

## Existing controls

### Webview and capabilities

The production CSP is:

```text
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' pdfr: http://pdfr.localhost blob: data:;
connect-src ipc: http://ipc.localhost;
font-src 'self';
object-src 'none';
base-uri 'none'
```

There are no remote script/network origins. The only installed Tauri plugin is
the native dialog plugin. The main-window capability grants core window/event
operations and dialogs but no general filesystem, shell, process, HTTP,
clipboard, updater, or opener capability.

Rendered pages use a custom scheme with strict integer parsing. Rotation is
limited to quarter turns, scale to `1..=8000`, tiles to non-zero dimensions no
larger than 4096×4096, and unknown paths are rejected. Generation-stamped URLs
prevent old work from being served after a render-state change.

Solid renders annotation/user text as text content rather than injected HTML.
No PDF JavaScript, embedded object, or remote link is executed by the frontend.

### Native engine

One dedicated thread owns PDFium and serializes all calls because PDFium is not
thread-safe. Render allocation rejects a single bitmap over 64 million pixels;
the frontend tiles much earlier (about 16.7 million). Raster/text caches have
explicit byte budgets. Password strings remain in Rust engine state only for
reopening during the session and are not persisted by SpeedyF.

PDFium is dynamically bundled from the pinned `chromium/7961` binary release.
The fetch script pins the release tag but currently does **not** verify a
checked-in cryptographic digest or upstream signature. A production release
should add per-platform SHA-256 verification and dependency provenance.

### Saving and data integrity

Save builds a new PDF at a randomly named temporary sibling of the destination,
flushes and syncs it, reopens it with PDFium, checks that it is readable and
has the expected page count, then atomically replaces the destination. Writer
or verification errors remove the temporary file and leave an existing
destination byte-for-byte unchanged. Tests inject verification failure to
enforce this property.

This protects against partial writes; it is not a full semantic verifier.
Verification does not prove that every annotation, form, metadata entry, or
advanced PDF feature survived. Atomic replacement also cannot protect against
disk/controller failures outside normal filesystem guarantees.

Unsaved-change guards run before opening another document or closing a dirty
window. There is no autosave or crash-recovery journal, so a process/OS crash
can still lose the current in-memory EditPlan.

### Privacy

SpeedyF has no telemetry, account, cloud sync, or runtime network feature. PDF
and image paths are local. Theme and low-memory preferences are the only data
stored in webview `localStorage`. The PDFium download and package-manager
operations use the network during development/build, not document viewing.

## Important limitations

### Native parsing is in-process

PDFium and the Rust image decoder run in the Tauri process. Memory corruption
or a native crash can terminate the entire application; a sufficiently serious
decoder vulnerability could have process-level impact. PDFium receives
Chrome-scale security attention, but that reduces rather than eliminates risk.
Open highly suspicious files in an OS sandbox/VM until the engine moves behind
a restricted helper process.

### Command paths are trusted-UI paths, not capability tokens

The webview has no filesystem plugin, but narrow commands currently accept
string paths chosen by the native dialogs/drag-drop flow. Rust validates file
existence/type at use sites but does not maintain an allowlist of
dialog-authorized path tokens. If application script execution were
compromised, an attacker who knew a local path could attempt to invoke the
PDF/image read or save commands directly.

The strong follow-up is a Rust-owned dialog/handle registry that issues
short-lived document/image/destination tokens. Until then, keep the CSP closed,
do not add remote content, audit every new HTML/script sink, and do not broaden
Tauri capabilities.

### Save is not cryptographic preservation

- Saving rebuilds the document from imported pages.
- Existing digital signatures should be assumed invalidated.
- Input password/encryption settings are not preserved on output.
- Bookmarks, attachments, document JavaScript, metadata, XFA, advanced forms,
  and unusual document-level structures are not guaranteed to survive.
- Form filling is best effort and limited to AcroForm text fields.

Do not save a document when those properties must remain authoritative without
first testing the output in the required downstream validator.

### No secure redaction

Rectangle/highlight/ink marks are visual edits. They do not delete covered text
operators, images, metadata, revisions, or other recoverable content. SpeedyF
does not implement secure redaction and must not be represented as doing so.

There is also no OCR, signature verification/creation, certificate validation,
or malware scanning.

## Release hardening checklist

Before distributing binaries:

- Verify PDFium with checked-in hashes for every target archive.
- Run dependency/license/vulnerability review for Rust and pnpm lockfiles.
- Build on each target OS; do not cross-sign opaque artifacts.
- Sign Windows installers and macOS code, including the bundled PDFium library.
- Notarize the final macOS app/DMG and test Gatekeeper on a clean machine.
- Generate an SBOM and retain build provenance.
- Run malformed/corpus fuzz regression tests under sanitizers where supported.
- Confirm CSP/capabilities did not broaden.
- Confirm release bundles contain no source maps, benchmark PDFs, paths,
  credentials, development entitlements, or signing secrets.
- Add helper-process sandboxing before claiming hostile-document isolation.
- Add Rust-owned scoped file tokens before allowing remote/web content.

## Reporting a vulnerability

Use the repository host’s private security-advisory channel when available.
Include the SpeedyF revision, operating system/architecture, PDFium tag, impact,
and minimal reproduction. Do not attach a confidential source document to a
public issue; reduce it to a synthetic fixture or arrange a private transfer.
