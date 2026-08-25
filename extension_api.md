# SpeedyF Extension API — design plan

**Status:** design, not implemented. Nothing in this document exists in the
codebase yet.

**Decision it supersedes:** a plugin API was considered on 2026-08-21 and
deliberately deferred in favour of a versioned on-disk data contract between
SpeedyF and a separate corpus tool. That deferral left an explicit fork: keep
the reader clean (data contract wins) or make SpeedyF a platform others build
on (plugin API is right, at a multi-month price). This document takes the
second branch, and honours the condition attached to it — the design is tested
against several unrelated extension ideas rather than a single one.

---

## 1. The problem

An extension is code somebody else wrote, running inside SpeedyF.

Everything below follows from that one sentence. You did not write it, you
cannot review every future version of it, and it will still be installed after
you have rewritten half the engine. So the design is really three answers:

1. **Where does extension code run**, so a bad extension cannot break the app?
2. **How does it draw on screen**, without handing over control of the UI?
3. **What is it allowed to see and do?**

The Chrome extension model answers all three, and is the model followed here.

## 2. The six extensions this design is tested against

A platform designed against one extension becomes a bespoke integration with
extra steps. These six pull in deliberately different directions:

| #   | Extension                     | What it needs                                                     |
| --- | ----------------------------- | ----------------------------------------------------------------- |
| 1   | Corpus/graph bridge           | A custom interactive view; read citations; open documents         |
| 2   | BibTeX export                 | Read the citation under the cursor; write the clipboard           |
| 3   | OCR                           | Read page rasters in batch; **contribute** a text layer + outline |
| 4   | Translate/summarize selection | Read selected text; network access; show a result                 |
| 5   | Vim keybindings               | Bind keys to actions that already exist                           |
| 6   | Reading ruler / theme         | Presentation only; no data access at all                          |

\#3 is the one that shapes the design most, and section 9 works through why its
shape matters more than its difficulty.

## 3. Principles

1. **Isolation is structural, not disciplinary.** Extension code runs somewhere
   it physically cannot reach the app's internals. Isolation is not a rule
   contributors have to remember.
2. **Extensions describe UI; SpeedyF draws it.** Extensions get visual
   consistency and future restyles for free, and cannot break layout.
3. **Extensions bind to a facade, never to internals.** The engine stays free
   to change. Only a small, versioned, deliberately simplified surface is
   promised.
4. **Everything is declared up front.** Permissions and UI placement live in a
   manifest that is readable before any extension code runs.
5. **Extensions are never on the critical path** — until v3 deliberately puts
   one there, with a timeout policy.
6. **The user can always tell what is installed and what it can reach.**

## 4. Mechanism one — where code runs

SpeedyF ships a Content Security Policy (in `tauri.conf.json`) that includes
`script-src 'self'`, meaning **only JavaScript that shipped with the app may
run**. It exists so a hostile PDF cannot inject code into the window. It stays.

Extension logic therefore runs in a **Web Worker**: a separate JavaScript
environment with no DOM access, no access to app variables, and exactly one
channel out — asynchronous messages. A worker that crashes crashes alone; a
worker that spins forever does not freeze the UI.

This is the direct equivalent of Chrome's isolated worlds, and it is cheaper
here because SpeedyF's extensions have no reason to touch the document DOM at
all.

**Loading mechanism — settled by measurement, not assumption.** The obvious
route does not work. `new Worker(url)` requires the URL to be same-origin with
the creating document; that is a platform rule, not a CSP setting, and no
policy change relaxes it. A custom scheme is by definition a different origin,
so serving worker code from `speedyf-ext://` fails with an opaque error. This
was the first thing built, and it failed exactly there.

What works, and what SpeedyF does: the Rust side reads the entry file and hands
the source to the frontend, which wraps it in a blob URL. Blob URLs inherit the
creating document's origin, satisfying the rule. The CSP needs `worker-src
'self' blob:`; `script-src` is untouched.

Two consequences worth knowing before writing an extension:

- **An entry must be self-contained.** Relative `importScripts` cannot resolve
  from a blob, so extensions ship a bundle.
- **The worker inherits the app's CSP** for its own subresource loads, and
  `connect-src` is `ipc:` alone. An extension therefore cannot reach the
  network at all — denied by the platform rather than by our policy. That is a
  stronger guarantee than the permission model gives, and it holds until v3
  deliberately opens a hole.

The `speedyf-ext://` scheme still exists and is still needed: sandboxed pane
documents (v3) are iframes, and iframes are allowed to be cross-origin. Only
workers need the blob route.

## 5. Mechanism two — how extensions draw

Arbitrary DOM injection into a SolidJS tree is rejected: it inherits every
extension's layout bugs, style clashes and reactivity mistakes.

### Declarative UI (the default)

An extension returns a description of what it wants shown:

```json
{
  "rows": [
    {
      "id": "r1",
      "title": "Attention Is All You Need",
      "subtitle": "cited 3x - p. 4, 7, 11",
      "badge": "in library",
      "action": { "command": "engram.open", "args": { "doi": "10.5555/1" } }
    }
  ]
}
```

SpeedyF renders it with the same components the Figures and Notes panels use.
Extensions inherit dark mode, the inverted-page palette, and every future
restyle without their authors touching anything.

The vocabulary is deliberately small and grows only on demand:

- `rows` — title, optional subtitle, badge, thumbnail, action
- `sections` — a heading plus rows
- `empty` — placeholder text when there is nothing to show
- `error` — a failure message in the house style
- `fields` — label plus control (text, checkbox, select), for settings

### Custom panes (the escape hatch)

Extension #1's interactive graph cannot be expressed declaratively. Such an
extension may declare a pane rendered as a sandboxed `iframe` served from
`speedyf-ext://<id>/`. It has its own origin and cannot reach the host DOM.

It is clunkier, will not match SpeedyF's styling, and needs its own message
bridge — which is correct. It should be the rare path, and it does not exist
until v3.

## 6. Mechanism three — the manifest

Every extension ships a `speedyf.json` beside its code.

```json
{
  "id": "com.example.bibtex",
  "apiVersion": "1",
  "name": "BibTeX Export",
  "version": "0.2.0",
  "description": "Copy any resolved citation as a BibTeX entry.",
  "author": "example",
  "entry": "worker.js",
  "permissions": ["citations:read", "clipboard:write"],
  "contributes": {
    "commands": [{ "id": "copyAll", "title": "Copy All References as BibTeX" }],
    "citationActions": [{ "id": "copyOne", "title": "Copy as BibTeX" }]
  }
}
```

Two independent ideas live here.

**Permissions** are what it may _see_. Unrequested capabilities fail at the API
boundary. Shown to the user at install, and reviewable afterwards.

**Contributions** are where it may _appear_. An extension cannot place UI
freely; it picks from surfaces SpeedyF defines and controls.

`id` is a reverse-DNS string, unique per extension. It namespaces the
extension's commands (`com.example.bibtex/copyAll`), its storage, and its
custom URI scheme path.

## 7. Permission catalogue

| Permission                             | Grants                                                       | Version |
| -------------------------------------- | ------------------------------------------------------------ | ------- |
| `commands:run`                         | Run SpeedyF's own commands, View/Navigate/Panels/Search only | v2      |
| `document:read`                        | Text, outline, figures, page geometry of the open document   | v2      |
| `document:annotate`                    | Create and modify annotations                                | v2      |
| `document:edit`                        | Page operations: rotate, delete, reorder, insert             | v3      |
| `citations:read`                       | Extracted and resolved citations                             | v2      |
| `library:read`                         | Library entries and library-wide search                      | v2      |
| `clipboard:write`                      | Write text or images to the clipboard                        | v2      |
| `storage`                              | Private key/value store, namespaced to the extension         | v2      |
| `render:read`                          | Request page and region rasters                              | v2      |
| `document:contributeText`              | Contribute a text layer and outline for a document           | v2      |
| `filesystem:read` / `filesystem:write` | User-chosen paths only, via a picker                         | v3      |
| `network`                              | Outbound HTTP                                                | v3      |

**What `document:read` actually grants.** Page text, in a loop, is the whole
document's text. The permission means "everything this document says", not "a
page at a time", and consent UI must say so — a prompt that reads narrower than
the grant is misleading by omission.

**Raw PDF bytes are deliberately absent.** No `document.bytes`, no file path, no
rasters. Not because no extension could use them — a converter or a signature
verifier genuinely would — but because that one primitive makes every other
piece of scoping pointless: there is no reason to think carefully about what
`citations.list` returns if the whole file sits available beside it. If it is
ever added it belongs in v3, under its own loud permission, and
`document:bytes` together with `network` should require explicit per-extension
consent, because that pair means "silently upload every paper you open".

The counterweight is to make the narrow paths genuinely useful, so most
extensions never need the broad one. `selectionActions` exists for exactly this:
a translate or summarise extension should need only what the user selected, and
never `document:read`.

**What keeps this safe today is structural, not policy.** The worker inherits
the app's CSP, whose `connect-src` is `ipc:` alone, so an extension can read
everything and has nowhere to send it. That guarantee holds until v3 opens
`network` — which is the moment read-breadth starts to matter, and the reason
the row below is not just another permission.

`network` is not just another row. SpeedyF is entirely local today, and for a
tool that reads your research that is a feature rather than an accident. The
moment extensions can phone home, "local-only" becomes "local-only unless you
installed something." It requires explicit install-time consent, a persistent
indicator while an extension is using the network, and a per-extension
allowlist of hosts declared in the manifest.

## 8. Contribution point catalogue

| Surface                | Effect                                                | Version |
| ---------------------- | ----------------------------------------------------- | ------- |
| `commands`             | Entries in the command palette and keymap             | v1      |
| `panels` (declarative) | A sidebar tab built from the UI vocabulary            | v2      |
| `citationActions`      | A row action on a resolved citation                   | v2      |
| `selectionActions`     | An action on selected text                            | v2      |
| `settings`             | Declarative fields in the extension's settings page   | v2      |
| `panels` (custom)      | A sandboxed iframe pane                               | v3      |
| `documentTasks`        | Batch work over a document, contributing results back | v2      |

## 9. Consumers and contributors

Five of the six test extensions share one shape: the user acts, then the
extension runs, and if it is slow only the extension is slow. Call these
**consumers**. Failure is contained, so they are safe to ship early.

OCR (#3) is the exception, and how it is framed decides how hard it is.

**The shape to avoid** is a synchronous provider: the core calls the extension
mid-render and waits, because search, selection, smart copy and figure
detection all need the answer. That puts third-party code on the critical path
of core features — slow extension, slow SpeedyF — and needs a timeout policy, a
partial-results story and a precedence rule for competing extensions.

**The shape to build** is a batch contributor. The extension is handed a
document, takes as long as it takes, and contributes results back when it has
them. Nothing blocks. There is no render-path timeout because there is no
render-path call.

This leaves exactly one real question: **where do the results land?**

- _Into the extension's own panel only._ A pure consumer, zero new machinery,
  ships in v2 as-is. But the app's own search, text selection, smart copy and
  figure detection still see a blank page — the user gets two search boxes and
  only one works on scans. Not worth building.
- _Into the document's real text layer._ Core features start consuming
  third-party data, which is the thing to be careful about — but
  asynchronously, off the critical path, and with a precedent already in the
  codebase: the library extracts full text to a sidecar for library-wide
  search, tracked by `text_indexed` on `LibraryEntry`. Text produced elsewhere
  and cached beside the file is an established pattern here, not a new one.

Take the second. It reduces to a single asynchronous contribution point rather
than a provider subsystem, which is why `document:contributeText` sits in v2
below.

Policy decisions it still carries, all small:

- **Provenance.** Contributed text is marked as such, and the user can see that
  a page's text came from an extension rather than the file.
- **Uninstall.** Sidecars outlive the extension by default; removing them is an
  explicit user action. Losing a document's searchability because an extension
  was uninstalled would be surprising.
- **Precedence.** A document's embedded text always wins. Contributed text
  fills gaps; it never overrides.
- **Competition.** If two extensions contribute for the same document, first
  writer wins and the second is reported, rather than silently discarded.

`CitationResolver` in `src-tauri/src/library/mod.rs` — a one-method trait with a
single implementation — remains the template for any genuinely synchronous
provider slot, if one is ever needed. None is needed for OCR.

## 10. Message protocol

All host/extension traffic is asynchronous request/response with correlation
IDs. Never synchronous, from the very first version — see section 14.

Host to extension:

```json
{
  "type": "invoke",
  "id": 41,
  "command": "copyOne",
  "context": { "citation": { "doi": "10.5555/1", "label": "[3]" } }
}
```

Extension to host:

```json
{ "type": "call", "id": 7, "api": "citations.list", "args": { "docId": 2 } }
{ "type": "result", "id": 41, "value": null }
{ "type": "render", "panel": "refs", "ui": { "rows": [] } }
```

Rules:

- Every `call` is permission-checked against the manifest before it executes.
- Every engine request from an extension enters the existing priority queue at
  `Priority::Idle`, so extension work is starved behind the user's own
  scrolling. The scheduling policy for untrusted code is already written.
- A `call` that exceeds its deadline resolves as an error; it never blocks.
- An extension that throws is reported in the Extensions panel, not silently
  swallowed.

## 11. On-disk layout

Extensions are unpacked directories under the Tauri app data directory
(`app.path().app_data_dir()`, already used for the library index):

```
<app-data>/extensions/
  installed.json              registry: id, version, source, permissions, enabled
  com.example.bibtex/
    speedyf.json
    worker.js
  com.engram.bridge/
    speedyf.json
    worker.js
    graph.html
```

`installed.json` carries a schema version from day one. The library index
learned this lesson the hard way — an unrecognised shape used to discard the
whole index — and both persisted library files now carry versions.

---

# Version plan

## v1 — the skeleton

**Goal:** prove isolation, lifecycle and permissions end to end, with the
smallest possible API surface. If v1 is right, everything after it is
additive.

**Ships:**

- `speedyf-ext://` custom URI scheme, `worker-src` CSP addition
- Manifest parsing and validation, with a clear error for a bad manifest
- Install from a local folder; uninstall; enable/disable
- `installed.json` registry with a schema version
- Worker host: spawn, terminate, restart, crash containment
- Message protocol with correlation IDs and deadlines
- Permission gate at the API boundary (with only trivial permissions to gate)
- `contributes.commands` — extension commands appear in the command palette
  and are bindable, via the existing registry in `src/app/commands.ts`
- The Extensions panel replaces its placeholder: installed list, enable
  toggle, permissions, uninstall, per-extension error state

**Explicitly out of scope:** all UI contribution, all document data access,
clipboard, storage, network, custom panes, providers, install from a URL,
updates.

**Reference extension:** `examples/extensions/hello` — four commands that
respectively succeed, fail, hang, and attempt a call the permission gate
refuses. It exercises the whole v1 surface and no more.

Vim keybindings was the original candidate and does _not_ fit v1: binding keys
needs both a keybinding contribution point and the ability to invoke SpeedyF's
own commands, and v1 has neither. It moves to v2.

**Done when:** an extension living in a separate git repo can be installed from
a folder, adds a command to the palette, survives an app restart, is cleanly
uninstalled, and — when deliberately made to throw or loop forever — does not
degrade the reader.

Note that v1 grants no host APIs at all, so the permission gate refuses
everything. That is deliberate: it is far cheaper to open capabilities later
than to take them away from extensions already built against them.

**Why this is cheaper than it looks:** `src/app/commands.ts` opens with
_"Everything SpeedyF can be asked to do, as data. One registry, two readers."_
An extension API is the third reader.

## v2 — the useful one

**Goal:** extensions people actually want. Everything here is a _consumer_, so
failure stays contained.

**Ships:**

- **[shipped]** Declarative UI vocabulary (`rows`, `sections`, `empty`,
  `error`) and `contributes.panels` — a real sidebar tab. Extensions describe;
  SpeedyF draws with its own components, so a contributed panel matches the
  built-ins and inherits every future restyle. The description is untrusted
  input, so it is validated structurally rather than cast: unreadable rows are
  dropped individually so one bad row does not cost the panel, text is
  truncated rather than refused, and the row cap applies across the whole
  description so splitting into sections is not a way around it. Panels render
  when opened, never on a timer — an extension is not woken because a document
  scrolled. Reference extension: `extensions/references`, which now ships built in.
- `fields` in the vocabulary, for `contributes.settings`
- **[shipped]** `contributes.selectionActions` and
  `contributes.citationActions`. Deliberately need **no permission at all**:
  the extension is handed exactly what the user picked and nothing else. This
  is the counterweight that keeps `document:read` from being the only way to
  reach a paragraph — a translate, summarise or note-taking extension should
  need neither. Selection actions appear in the command palette when text is
  selected; citation actions appear on the hover popover. Reference extension:
  `examples/extensions/scratchpad`, which asks only for `clipboard:write` and
  `storage` and never reads the document.
- **[shipped]** `contributes.settings` — declared, not drawn: the extension
  says what it wants to ask (text, checkbox, select) and SpeedyF renders the
  control in the Extensions panel. Values persist under their own key prefix,
  so an extension's `storage.set` cannot overwrite what the user chose, and
  `settings.get` needs no permission — reading its own declared settings is not
  access to anything. Read fresh on each call rather than cached at activation,
  so a change takes effect without restarting the worker.
- **[shipped]** Read APIs: `document.info`, `document.pageText`,
  `document.outline`, `document.figures`, `citations.list`,
  `citations.resolve`, `library.search`. Deliberately _not_ the engine's DTOs
  forwarded — each shape is a narrower version, and the mapping between them is
  the seam that lets the engine change without breaking anything installed.
  Three rules chose them: omit what belongs to another permission (no
  filesystem paths, no engine ids), omit geometry until something needs it
  (page text is text, not positioned runs), and normalise oddities rather than
  pass them on (`arXiv` becomes `arxiv`). Reads are scoped to the _active_
  document, so an extension never names one and cannot reach a tab the user is
  not looking at. Reference extension: `examples/extensions/bibtex`.
- **[shipped]** `storage` — namespaced key/value, bounded (64 KB a value, 100
  keys an extension), cleared on uninstall. The namespace is applied by the
  host, not the extension, so one cannot read or overwrite another's.
- **[shipped]** `clipboard.write`, bounded at 100 KB. Note the platform
  constraint: a clipboard write is only permitted while the user action that
  started it is still in effect, and the round trip out to the worker spends
  some of that window. Extensions should write first and do slow work after;
  the host says so by name when it fails, because the raw platform message
  sends authors looking for a permission that does not exist.
- **[shipped]** Install from a git URL, optionally pinned with
  `#<branch|tag|commit>`. Shells out to `git clone` rather than adding an HTTP
  client, so SpeedyF still makes no outbound connections of its own — the cost
  is that git must be installed, which is reported by name when it is not. Only
  `https://` and `http://` are accepted: `ssh://` would clone with the user's
  own credentials, and `file://` would let a "URL" reach anywhere on disk. A
  URL or ref beginning with `-` is refused before git runs, since that is the
  one injection this interface has — git would read it as an option. Shallow by
  default, falling back to a full clone on remotes that cannot do it.
- Developer mode: load unpacked, hot reload on file change, a log console
- **[shipped]** `commands.run` + `contributes.keybindings` — the first real
  host API, and what vim keybindings needs. Two rules make it safe:
  `commands:run` reaches only the command groups that change _what you are
  looking at_ (View, Navigate, Panels, Search) and never the document, the file
  or settings; and keys bind to the extension's _own_ commands, so every effect
  on the app passes through the gate rather than a manifest entry. Built-in
  chords always win, and a contested chord goes to the first claimant with the
  loser told why. Reference extension: `examples/extensions/vim`.
- **[shipped, plumbing only]** `contributes.documentTasks` — batch work over a
  whole document, with `render:read` and `document:contributeText`. Rasters
  come from a Tauri command rather than a `pdfr://` URL, because the CSP's
  `connect-src` is `ipc:` alone and an extension needs bytes rather than an
  `<img>`; they are rendered at `Priority::Idle`, so a three-hundred-page job
  is starved behind the reader's own scrolling. Contributed text lands in a
  versioned sidecar under the app data directory, never in the PDF — writing
  into the file would change bytes the user did not ask to change. Provenance,
  precedence and competition are enforced (first writer wins, second is told
  who holds it); a sidecar deliberately outlives the extension that wrote it.
  Reference extension: `examples/extensions/pageaudit`.

  **Not shipped: core consumption.** SpeedyF's own search, selection, smart
  copy and figure detection do not read contributed text yet — that is engine
  work in Rust and the point at which core features start depending on
  third-party output. Deferred on purpose so the contract can settle against a
  real OCR extension first.

**Explicitly out of scope:** custom panes, synchronous providers, network,
filesystem, document editing.

**Reference extensions:** BibTeX export (#2); a references panel listing every
citation in the open document with library hit/miss status; and OCR (#3),
which is the reason `documentTasks` is in this version rather than the next.

**Done when:** all three reference extensions are built in separate repos by
following this document alone, with no changes to SpeedyF — and a scanned PDF
processed by the OCR extension is searchable and selectable through SpeedyF's
own search and text layer, not a parallel one.

## v3 — the platform

**Goal:** everything supported. After this, the API is a public commitment.

**Ships:**

_Capability_

- Custom panes: sandboxed iframe views with their own message bridge
- Synchronous provider slots (citation resolution, metadata) with timeouts and
  precedence — only if something actually needs them; OCR does not
- `network` with a manifest host allowlist, install-time consent, and a live
  in-app indicator
- `filesystem:read`/`write`, user-chosen paths only, through a picker
- `document:annotate` and `document:edit`
- `render:read` — page and region rasters
- Background tasks on idle, budgeted and cancellable

_Operations_

- Versioning: `apiVersion` ranges, deprecation warnings, a compatibility
  policy stated in writing
- Auto-update with signature or hash verification
- Discovery: a registry or curated index, however minimal
- Resource limits: memory ceiling, CPU budget, forced termination with a
  user-visible reason
- Health surface: per-extension errors, timings, and what each has accessed
- Full data cleanup on uninstall

**Done when:** all six reference extensions in section 2 are buildable without
changes to SpeedyF, and an extension can be published, installed by a stranger,
updated and removed without either party talking to the other.

---

## 12. Cost

v1 is on the order of weeks. v2 is larger than it looks now that
`documentTasks` sits in it, but it is where the platform becomes worth having.
v3 is where the multi-month estimate lives — sandboxed panes, network policy
and update infrastructure are separate hard problems wearing one version
number. Treat v3 as a phase, not a release.

## 13. Risks

- **The worker loading mechanism** (section 4) is unvalidated. Prototype first.
- **The declarative vocabulary will feel too small** the first time a real
  extension wants something. Resist widening it per request; a vocabulary that
  grows to match every extension becomes DOM injection with extra steps.
- **Contributed data changes the product.** Once core search and selection can
  read text an extension produced, SpeedyF's behaviour depends on third-party
  output. Off the critical path is not the same as harmless: bad OCR makes the
  app look broken. Provenance marking (section 9) is what keeps this
  debuggable.
- **v3 changes it further.** Network access means "local-first" becomes
  "local-first unless you installed something."

- **A published API is hard to withdraw.** Every surface here is a maintenance
  commitment for as long as extensions exist.

## 14. Hard to retrofit — get these right in v1

Each of these is cheap now and a breaking change for every installed extension
later.

0. **Manifest parsing refuses unknown fields.** A manifest saying
   `selectionActions` while the parser expected `selection_actions` parsed
   _successfully_, defaulted the field to empty, and produced an extension that
   installed cleanly and silently did nothing. A typo in a contribution point
   must be an install-time error. Added after it cost an afternoon.
1. **`apiVersion` in the manifest from day one.** Without it there is no clean
   way to evolve, and no way to refuse an incompatible extension.
2. **The permission system exists in v1**, even with almost nothing to gate.
   Adding permissions later means every existing extension had full access and
   now does not.
3. **Namespaced IDs from the first command.** Retrofitting namespacing renames
   every extension's commands and breaks every user's keybindings.
4. **The protocol is asynchronous with correlation IDs from the start.** A
   synchronous or fire-and-forget protocol cannot be made async later without
   rewriting every extension.
5. **Storage is namespaced per extension** before anything is stored.
6. **`installed.json` carries a schema version** before the first install.
