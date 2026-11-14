# Writing a SpeedyF extension

Everything an extension can do, and how. For _why_ it works this way, see
`extension_api.md` - this file is the reference.

SpeedyF ships with References, Notation, Math to LaTeX and Notepad already installed;
their source is in `extensions/`, and a built-in is an ordinary extension that
happens to live in the app bundle - see that folder's README for the three ways
it differs. Math to LaTeX uses the internet, so it ships **disabled**; switching
it on is the consent (see [Network](#network)).

Working examples live in `examples/extensions/`. Load any of them from
**Home -> Extensions -> Load folder...**.

---

## Contents

- [The shape of an extension](#the-shape-of-an-extension)
- [Manifest](#manifest)
- [Permissions](#permissions)
- [Contribution points](#contribution-points)
- [Messages](#messages)
- [APIs](#apis)
- [Describing UI](#describing-ui)
- [Limits](#limits)
- [Installing and iterating](#installing-and-iterating)
- [What extensions cannot do](#what-extensions-cannot-do)

---

## The shape of an extension

A directory with a `speedyf.json` and one JavaScript file:

```
my-extension/
  speedyf.json
  worker.js
```

Your code runs in a **Web Worker**. There is no `document`, no `window`, and no
access to SpeedyF's internals. The only channel is `postMessage`.

Two consequences worth knowing before you write anything:

- **Your entry file must be self-contained.** It is loaded through a blob URL,
  so relative `importScripts` cannot resolve. Ship a bundle if you need
  dependencies.
- **Your worker cannot reach the network.** Not a policy - the worker inherits
  a Content Security Policy that permits no outbound connections at all. An
  extension that needs a web service declares the hosts and asks SpeedyF to
  fetch for it (`net.fetch`, under the `network` permission); the request is
  made by the core, never by your worker.

The smallest extension that does something:

```js
self.onmessage = (event) => {
  const message = event.data;
  if (message.type === 'invoke') {
    self.postMessage({ type: 'log', level: 'info', message: 'hello' });
    self.postMessage({ type: 'done', requestId: message.requestId, ok: true });
  }
};
```

---

## Manifest

`speedyf.json`. Unknown fields are **rejected**, so a typo is an install error
rather than a silently ignored setting.

```json
{
  "id": "com.example.thing",
  "apiVersion": 1,
  "name": "Thing",
  "version": "1.0.0",
  "description": "What it does.",
  "author": "you",
  "entry": "worker.js",
  "icon": "icon.svg",
  "permissions": ["citations:read"],
  "network": ["api.example.org"],
  "contributes": {
    "commands": [
      { "id": "run", "title": "Thing: Run", "keywords": ["go"] },
      { "id": "events", "title": "Thing: Editor Events", "hidden": true },
      { "id": "clip", "title": "Thing: Clip Page", "when": "documentInProject" }
    ],
    "keybindings": [{ "command": "run", "key": "mod+shift+t" }],
    "panels": [{ "id": "main", "title": "Thing" }],
    "selectionActions": [{ "command": "run", "title": "Send to Thing" }],
    "citationActions": [{ "command": "run", "title": "Look up in Thing" }],
    "documentTasks": [{ "command": "run", "title": "Thing: Scan Document" }],
    "referenceProvider": { "command": "run" },
    "termProvider": { "command": "run" },
    "equationActions": [{ "command": "run", "title": "Copy LaTeX" }],
    "equationConverter": { "command": "run" },
    "readerPanes": [
      {
        "id": "pad",
        "title": "Pad",
        "when": "documentInProject",
        "key": "mod+shift+y",
        "editor": { "file": "pad.md", "command": "events" }
      }
    ],
    "projectViews": [
      { "id": "pad", "title": "Pad", "editor": { "file": "pad.md", "command": "events" } }
    ],
    "itemDetailLines": { "command": "run" },
    "settings": [
      {
        "key": "mode",
        "title": "Mode",
        "type": "select",
        "default": "fast",
        "options": ["fast", "thorough"],
        "description": "How hard to try."
      }
    ]
  }
}
```

| Field         | Required | Notes                                                                                                                                                                                                                |
| ------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`          | yes      | Lowercase reverse-DNS, must contain a dot. Namespaces your commands, storage and files.                                                                                                                              |
| `apiVersion`  | yes      | `1`. A build that does not recognise it refuses to install.                                                                                                                                                          |
| `name`        | yes      | Shown in the Extensions panel.                                                                                                                                                                                       |
| `version`     | yes      | Your version, any string.                                                                                                                                                                                            |
| `entry`       | yes      | Relative path to your worker, inside the extension.                                                                                                                                                                  |
| `icon`        | no       | Relative `.png`, `.svg`, `.jpg`. Drawn as a mask in the current text colour, so design it as a **single-colour silhouette** - only the shape survives. Without one, SpeedyF draws a generic mark with your initials. |
| `permissions` | no       | See below. Anything not requested is refused at the API boundary.                                                                                                                                                    |
| `network`     | no       | Hosts `net.fetch` may reach, with the `network` permission (and only with it). See [Network](#network).                                                                                                              |
| `contributes` | no       | Where you appear. See below.                                                                                                                                                                                         |

Command, panel, pane and view ids must be non-empty and contain no `/` -
that character separates them from your extension id. A command may say
`"hidden": true` (not listed in the palette; reached only through the
contribution that names it) and `"when": "documentInProject"` (listed only
while the reader's document is an item in a project). Actions and reader
panes take the same `when`; it is the only condition there is. Every `keybindings`, `selectionActions`,
`citationActions`, `documentTasks` and `equationActions` entry, and
`referenceProvider` and `termProvider`, must name a command you declared;
`referenceProvider` also needs the `library:contributeReferences` permission,
`termProvider` the `document:contributeTerms` permission, `itemDetailLines`
`projects:read`, and a pane's or view's `editor` `projects:files` (its `file`
a project file path, below). (The example above lists every
field - a real extension would not make one command all of these.)

---

## Permissions

Declared in the manifest, refused if not declared. Absent = refused.

| Permission                     | Grants                                                                                                                                                                                                                                    |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `document:read`                | The open document's text, outline and figures. Note this is the **whole document's text** - page text in a loop is everything it says.                                                                                                    |
| `citations:read`               | Citations found in the document, and resolving them against your library.                                                                                                                                                                 |
| `library:read`                 | Searching the library, and reading its items and projects: titles, authors, years, cite keys and which project holds what. Never a file path.                                                                                             |
| `commands:run`                 | Running SpeedyF's own commands - **View, Navigate, Panels and Search only**. Never File, Edit, Page, Tools or Settings, so an extension cannot delete a page or overwrite a file.                                                         |
| `clipboard:write`              | Writing text to the clipboard.                                                                                                                                                                                                            |
| `storage`                      | A private key/value store, namespaced to you.                                                                                                                                                                                             |
| `render:read`                  | Page rasters as PNG bytes.                                                                                                                                                                                                                |
| `document:contributeText`      | Writing a text layer for a document, stored in a sidecar.                                                                                                                                                                                 |
| `library:contributeReferences` | Being handed the stored text of library documents during idle indexing - never their paths - and returning their reference entries. Only with `referenceProvider`.                                                                        |
| `document:layout`              | The open document's typeset characters with their fonts, sizes and boxes, the thin rules between them, and its tagged-PDF Formula elements (`selection.glyphs`, `region.glyphs`, `document.mathTags`).                                    |
| `network`                      | `net.fetch` to the hosts in your manifest's `network` list, and nowhere else. Needs a non-empty list; a list needs it.                                                                                                                    |
| `document:contributeTerms`     | Answering `termProvider` with the terms the open document defines (`terms.contribute`), which SpeedyF previews when the reader hovers them. Only with `termProvider`.                                                                     |
| `projects:read`                | The library's projects and their items' ids, cite keys (with the keys each had before), titles, authors and years - never a path; the reader's item and the project it is read in; opening an item at a page and place.                   |
| `projects:files`               | Files of your own, per project, which SpeedyF keeps beside the library: written atomically, backed up, carried in project bundles, set aside when the project is deleted. Also what a `readerPanes` or `projectViews` editor is bound to. |
| `render:region`                | A PNG of what the user pointed at: a rectangle they drag (`region.capture`), or the equation they clicked (`equations.snapshot`). Never a page they did not pick.                                                                         |

`ui.render`, `ui.showMath`, `ui.itemLine`, `pane.open`, `projectView.open`
and `settings.get` need no permission: drawing into a surface you declared,
showing a formula in SpeedyF's popover, and reading settings the user set for
you, are not access to anything. Nor does `equations.convert`: you hand back
the token of the equation the user clicked.

### Network

SpeedyF itself never touches the internet without a setting the user turned
on. An extension may, under four rules:

- **Declared hosts only.** `network` lists plain lowercase hostnames
  (`export.arxiv.org`), or `*.example.org` for every subdomain of
  `example.org` (not `example.org` itself). No schemes, ports, paths, IP
  addresses or other wildcards; at most 20 entries. A request to anything
  else - including a redirect to anything else - is refused.
- **Enabling is the consent.** The extension's card in **Home -> Extensions**
  carries a tag, _Uses the internet: <hosts>_, whether it is on or off. An
  extension that declares `network` arrives **disabled** - a built-in ships
  that way, and an installed one is installed that way - and switching it on
  is the user agreeing to its traffic. There is no further prompt.
- **The core makes the request.** `net.fetch` is answered by SpeedyF, not by
  your worker, whose CSP still allows no connections. It is `https://` only, on
  the default port, with no credentials in the URL, a GET, 30 seconds at most,
  and redirects are followed only within your list.
- **Paced per host.** Requests share SpeedyF's per-host rate limiter with the
  library lookups: one request in flight per host, a minimum spacing (one
  second for a host the limiter has no row for; three for `export.arxiv.org`),
  a pause when a host answers 429 or 503, and one retry after a server fault.
  While any of your requests is in flight the status bar says
  _<your name> is using the internet..._.

**What `library:contributeReferences` grants.** The full text of every
document in the user's library, one at a time, in the background - more than
`document:read`, which only ever sees the document on screen. What it does not
grant is any way to tell which file a text came from (a document is an opaque
token) or to write anything but reference entries for the document it was
just handed. Ask for it only to be a reference provider.

Selection and citation actions need **no permission at all** - you are handed
exactly what the user picked and nothing else. Prefer them over `document:read`
whenever they suffice.

---

## Contribution points

| Point               | What it does                                                                            |
| ------------------- | --------------------------------------------------------------------------------------- |
| `commands`          | Entries in the command palette (Cmd+Shift+P).                                           |
| `keybindings`       | A key chord that invokes one of your commands.                                          |
| `panels`            | A tab in the sidebar, drawn from a description you send.                                |
| `selectionActions`  | Offered in the palette when text is selected; your command receives the selected text.  |
| `citationActions`   | A button on the citation hover popover; your command receives `doi:...` or `arxiv:...`. |
| `documentTasks`     | A batch job over a whole document, offered in the palette.                              |
| `settings`          | Fields rendered in the Extensions panel; `type` is `text`, `checkbox` or `select`.      |
| `referenceProvider` | `{ "command": "<id>" }`: reads library documents' reference lists at idle (below).      |
| `equationActions`   | A button on each display equation SpeedyF detects; your command receives the block.     |
| `termProvider`      | `{ "command": "<id>" }`: the terms a document defines, previewed on hover (below).      |
| `readerPanes`       | A drawer on the right of the reader holding a Markdown editor (below).                  |
| `projectViews`      | A project's pane in the Projects detail column, holding the same editor (below).        |
| `itemDetailLines`   | `{ "command": "<id>" }`: one quiet line in the Projects item detail column (below).     |
| `equationConverter` | `{ "command": "<id>" }`: an equation as LaTeX, for whoever asks the core (below).       |

### `equationActions`

SpeedyF looks for display equations on each page you read - lines set mostly
in math fonts (Computer Modern math, the AMS fonts, OpenType math fonts) or
holding big operators or fraction bars, roughly centred, with an optional
right-aligned number like `(3)`, merged with their neighbours into one block.
Detection runs at idle and is cached per page, so it never slows scrolling, and
it only runs at all while an enabled extension contributes an equation action.

Hovering a block shows a small button at its top-right corner, titled with your
action's `title`. A click invokes your command with
`value: { page, bbox: [x, y, w, h], number }` - the reader's zero-based page,
the block in page points from the displayed page's top-left corner, and the
printed number (`"3"`, `"2.4"`) or `null`. Read the block's characters with
`region.glyphs { page, bbox }`, and answer with `ui.showMath`.

### `readerPanes` and `projectViews`: the editor

Both hold the same thing: a Markdown editor SpeedyF draws, bound to one file
of your project files (`editor.file`, such as `notepad.md`). A reader pane is
a drawer on the right of the page - a quiet icon at the end of the toolbar,
and `key` as its toggle when SpeedyF does not use the chord - shown only while
its `when` holds. A project view (the name is historical: it is a pane,
not a view of the items) fills the Projects detail column while a project is
the scope and no item is selected; selecting an item shows its details
there, under a quiet line back to the note, and Escape or that line returns.
The items stay listed beside it, and rows dragged from the list onto the
note are inserted as `@key` (several as `@a; @b`) where they land. When
several extensions contribute one, the first enabled shows, or the one
`projectView.open` last named. Every view of one file, in
every window, edits the same text live: SpeedyF holds it once, saves it about
half a second after typing stops, reloads it when it changes on disk with
nothing unsaved (and keeps the outside version beside it when there is), and
closes it when its project is deleted.

The editor draws headings, emphasis, lists, block quotes, `$...$` and
`$$...$$` math (KaTeX), links (opened only after the usual confirmation),
images from your own project files (never from the web), and Pandoc
citations: `@key` and `[@key, p. 12]{box=x,y,w,h}`, the box in top-left page
points. The line with the cursor shows its Markdown. Typing `@` offers the
entries you marked `pick`.

The rest is yours, through `editor.command`, invoked with `value: { event,
projectId, file, cite? }`:

- `event: 'citations'` when an editor opens and when the library changes:
  answer with `editor.citations { projectId, entries }`, each entry `{ key,
label, title, detail, pick, aliases }`. A citation of `key` or of any of its
  `aliases` is drawn as `label` (with its locator); one no entry has is
  drawn as written, muted, and never rewritten.
- `event: 'activate'` when a citation is clicked, with `cite: { key,
locator, page, box }` (`page` zero-based from a "p." locator). Typically
  `navigate.openItem`.

### `itemDetailLines`

When the Projects detail column shows an item, your command is invoked with
`value: { item: { id, citeKey, keyHistory, projectIds }, projectId }`
(`projectId` the selected project, or `null` under the other scopes), and
again shortly after any project file changes. Answer with `ui.itemLine {
itemId, text, action? }`: `text` null removes your line; `action: { command,
value }` makes it clickable and must name one of your own commands.

### `equationConverter`

An equation's LaTeX for any extension that asks (`equations.convert`), from
whichever enabled extension declares a converter - the first one. Your
command is invoked exactly as an equation action is, `{ page, bbox, number,
equation }`, and your `ui.showMath` answer is taken as the result instead of
being shown. Math to LaTeX is the converter that ships.

### `referenceProvider`

SpeedyF links library papers by the references in them. Identifiers it finds
itself - DOIs, arXiv ids, and the DOI an APS journal reference ("Phys. Rev.
Lett. 133, 057102") stands for. A reference that prints none of those is
matched by the library title it contains, and for that its reference list has
to be cut into entries: that is the provider's job.

- **Only one provider is active**: the first enabled extension that declares
  one. In practice that is the built-in References. Disable it and no
  reference lists are read or used - title matching is off, identifier
  matching is not.
- During idle indexing SpeedyF invokes your command, one document at a time,
  with `value: { document, text }` - `document` an opaque token, `text` the
  document's stored full text (pages joined by newlines, whitespace
  collapsed; the last 2 MB of a longer one). Never a path, a name or an id
  that means anything outside this exchange.
- Answer with a `call` to `references.contribute { document, entries }`, then
  `done`. `entries` is `[{ label?, text, year? }]` in list order: `label` as
  printed (`"12"`, `"AB99"`), `text` the entry with its lines joined, `year`
  an integer. Finishing without contributing means "no reference list".
- SpeedyF computes everything it matches on from your `text` - identifiers,
  journal DOIs, a compact form for title matching - and keeps the `year` you
  give (it does not guess one).
- Results are stored per document and per provider version: a new `version`
  in your manifest reads the library again, and a changed file is read again.
  A document you fail on (error or timeout) is not offered again until the
  next session.
- A provider command does not appear in the palette.

### `termProvider`

Hovering a term the paper defines - an acronym, say - shows what it stands
for in SpeedyF's link-preview popover, with "Defined on p. 3"; clicking goes
there, and Back returns. Knowing what a paper defines is your part; finding
every use on every page is SpeedyF's.

- **Only one provider is active**: the first enabled extension that declares
  one. In practice that is the built-in Notation, which contributes every
  acronym it reads ("fluctuation-dissipation theorem (FDT)" or "FDT
  (fluctuation-dissipation theorem)"). Disable it and there are no term hovers.
- When a document comes on screen, SpeedyF invokes your command with `value:
{ document }`, an opaque token. Read the document with the usual APIs
  (`document:read`), then call `terms.contribute { document, terms }` - before
  or after you report `done`, as long as the reader has not switched documents
  in between; an answer for a document no longer on screen is dropped
  (`{ stored: 0 }`), and SpeedyF asks again when it comes back.
- Each term is `{ text, body, page, caseSensitive?, kind?, title?, box? }`:
  `text` as it appears in the text (`"FDT"`, `"mRNA"`, `"COVID-19"`; two to 64
  characters, never a single letter), `body` what the popover shows (plain
  text, or with `$...$` TeX), `page` where it is defined (zero-based, as the
  reader sees pages), `box` the `ApiBox` of the definition. `title` defaults
  to `text`, `caseSensitive` to `true`. At most 2000; a malformed term is
  dropped on its own.
- SpeedyF matches whole tokens only: `FDT` matches `FDTs`, `FDT's`,
  `FDT-based` and `non-FDT`, never `SFDT`; a case-sensitive `IT` never matches
  `it` or `ITS`. Nothing matches inside mathematics, sub- and superscripts, the
  bibliography, a running head or a table of contents, nor inside `box` - the
  definition itself - or another "long form (TERM)" for it. Where a link
  already covers the text, the link's preview wins.
- The same `text` contributed twice with different bodies: each use shows the
  nearest preceding definition, or every one when none precedes it.
- The provider command does not appear in the palette.

**Key chords** use `mod` (Cmd on macOS, Ctrl elsewhere), `shift`, `alt`, then the
key: `j`, `mod+k`, `shift+g`, `alt+ArrowDown`. Chords SpeedyF already uses are
refused and reported in the Extensions panel - built-ins always win. If two
extensions want the same chord, the first installed keeps it.

A keybinding on a command that is _also_ a selection action receives the current
selection, exactly as the palette does.

---

## Messages

Asynchronous request/response with correlation ids, both directions.

### SpeedyF -> you

```js
{ type: 'activate', id, apiVersion, permissions }   // once, before anything else
{ type: 'invoke', requestId, command, value?, reason? }  // value: string, or { document, text }
{ type: 'reply', requestId, ok, value?, error? }    // answer to your `call`
```

`value` carries the selected text or citation for an action, `{ page, bbox,
number, equation }` for an equation action (`equation` an opaque token for
`equations.convert` and `equations.snapshot`), an editor event for an
editor's command, `{ item, projectId }` for an `itemDetailLines` command, `{ document, text }` for a
`referenceProvider` command, and `{ document }` for a `termProvider`
command. `reason` is
`'panel'` when your panel was opened rather than a command being run.

### You -> SpeedyF

```js
{ type: 'call', requestId, api, args }   // ask SpeedyF to do something
{ type: 'done', requestId, ok, error? }  // finish an invoke
{ type: 'log', level, message }          // 'info' | 'warn' | 'error'
```

**Always answer an `invoke` with a `done` carrying the same `requestId`.** If you
do not within **5 seconds** (**20 seconds** for a `referenceProvider`
document, **60 seconds** for an extension holding `network`), SpeedyF stops
waiting and reports a timeout against
your extension. It does not kill your worker - a slow command is not a broken
extension - but nothing in the UI ever blocks on you.

Anything that is not one of these three shapes is dropped and logged.

The plumbing every example uses:

```js
let nextRequestId = 1;
const pending = new Map();

function call(api, args) {
  const requestId = nextRequestId++;
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    self.postMessage({ type: 'call', requestId, api, args });
  });
}

self.onmessage = (event) => {
  const m = event.data;
  if (m.type === 'invoke') {
    handle(m.command, m.value)
      .then(() => self.postMessage({ type: 'done', requestId: m.requestId, ok: true }))
      .catch((e) =>
        self.postMessage({
          type: 'done',
          requestId: m.requestId,
          ok: false,
          error: String(e.message ?? e),
        })
      );
  } else if (m.type === 'reply') {
    const waiting = pending.get(m.requestId);
    if (!waiting) return;
    pending.delete(m.requestId);
    m.ok ? waiting.resolve(m.value) : waiting.reject(new Error(m.error));
  }
};
```

---

## APIs

All reads are scoped to the **active document**. You never name a document, and
cannot reach a tab the user is not looking at. Pages are zero-based.

| API                        | Args                                     | Returns                                                                                                                        | Needs                          |
| -------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------ |
| `document.info`            | -                                        | `{ name, pageCount, currentPage }`                                                                                             | `document:read`                |
| `document.pageText`        | `{ page }`                               | `string`                                                                                                                       | `document:read`                |
| `document.outline`         | -                                        | `[{ title, page, children }]`                                                                                                  | `document:read`                |
| `document.figures`         | -                                        | `[{ label, caption, page }]`                                                                                                   | `document:read`                |
| `citations.list`           | `{ page }`                               | `[{ kind, value, page }]`                                                                                                      | `citations:read`               |
| `citations.resolve`        | `{ kind, value }`                        | `{ inLibrary, title, fileName, pageCount, citeKey }`                                                                           | `citations:read`               |
| `library.search`           | `{ query }`                              | `[{ name, title, matches }]`                                                                                                   | `library:read`                 |
| `library.papers`           | -                                        | `{ papers: [{ id, title, authors, year, venue, citeKey, projectIds, isPaper }] }`                                              | `library:read`                 |
| `library.projects`         | -                                        | `[{ id, name, paperCount }]`                                                                                                   | `library:read`                 |
| `library.paper`            | `{ id }`                                 | the record: `meta` with a `source` per field, `doi`, `arxiv`, `citeKey`, `projectIds`, `isPaper`, `files: [{ name, missing }]` | `library:read`                 |
| `commands.run`             | `{ id }`                                 | `{ ran }`                                                                                                                      | `commands:run`                 |
| `render.page`              | `{ page, scaleMilli? }`                  | `number[]` - PNG bytes                                                                                                         | `render:read`                  |
| `document.contributeText`  | `{ page, text }`                         | `{ stored }`                                                                                                                   | `document:contributeText`      |
| `document.contributedText` | -                                        | `{ extensionId, pages } \| null`                                                                                               | `document:contributeText`      |
| `references.contribute`    | `{ document, entries }`                  | `{ stored }` - only for the document you are being invoked with                                                                | `library:contributeReferences` |
| `terms.contribute`         | `{ document, terms }`                    | `{ stored }` - only while that document is still on screen                                                                     | `document:contributeTerms`     |
| `clipboard.write`          | `{ text }`                               | `{ written }`                                                                                                                  | `clipboard:write`              |
| `storage.get`              | `{ key }`                                | `string \| null`                                                                                                               | `storage`                      |
| `storage.set`              | `{ key, value }`                         | `{ stored }`                                                                                                                   | `storage`                      |
| `storage.remove`           | `{ key }`                                | `{ removed }`                                                                                                                  | `storage`                      |
| `settings.get`             | `{ key }`                                | `string`                                                                                                                       | -                              |
| `ui.render`                | `{ panel, ui }`                          | `{ rendered }`                                                                                                                 | -                              |
| `selection.glyphs`         | -                                        | `{ page, glyphs: Glyph[] } \| null` - the current text selection                                                               | `document:layout`              |
| `region.glyphs`            | `{ page, bbox }`                         | `{ page, glyphs: Glyph[] }`                                                                                                    | `document:layout`              |
| `document.mathTags`        | `{ page }`                               | `[{ bbox, alt, actualText, mathml }]` - tagged-PDF Formula elements                                                            | `document:layout`              |
| `document.identifiers`     | -                                        | `{ doi, arxiv, arxivVersion }`, each `null` when unknown                                                                       | `document:read`                |
| `net.fetch`                | `{ url, as?, maxBytes? }`                | `{ status, contentType, body }` - `body` a string for `as: 'text'` (the default), `number[]` for `as: 'bytes'`                 | `network` (host listed)        |
| `ui.showMath`              | `{ latex, display, source, confidence }` | `{ shown: true }`                                                                                                              | -                              |
| `projects.list`            | -                                        | `[{ id, name, itemCount }]`                                                                                                    | `projects:read`                |
| `projects.items`           | `{ projectId? }`                         | `[{ id, citeKey, keyHistory, title, authors, firstAuthor, alphaLabel, year, projectIds }]` - a project's items, or every item  | `projects:read`                |
| `reader.context`           | -                                        | `{ item: { id, citeKey, title } \| null, projectIds, projectId, page, selection: { page, box, text } \| null }`                | `projects:read`                |
| `reader.chooseProject`     | -                                        | a project id or `null`: asks once which of the paper's projects, kept for the session                                          | `projects:read`                |
| `navigate.openItem`        | `{ itemId, projectId?, page?, box? }`    | `{ opened }` - opens it (at a page and box), with Back                                                                         | `projects:read`                |
| `projectFiles.read`        | `{ projectId, path, as? }`               | a string (`as: 'text'`, the default; the live text of an open editor), `number[]` for `as: 'bytes'`, or `null`                 | `projects:files`               |
| `projectFiles.write`       | `{ projectId, path, text \| bytes }`     | `{ written }` - atomically; into an open editor as an edit                                                                     | `projects:files`               |
| `projectFiles.list`        | `{ projectId }`                          | `[{ path, size }]`                                                                                                             | `projects:files`               |
| `projectFiles.remove`      | `{ projectId, path }`                    | `{ removed }` - not a file open in an editor                                                                                   | `projects:files`               |
| `editor.citations`         | `{ projectId, entries }`                 | `{ stored }`                                                                                                                   | `projects:files`               |
| `editor.insert`            | `{ projectId, text, at?, file? }`        | `{ offset }` - at the end after a blank line, or `at: 'cursor'` in an editor showing it                                        | `projects:files`               |
| `editor.reveal`            | `{ projectId, offset, file? }`           | `{ revealed }` - scrolls an editor there, now or when one opens                                                                | `projects:files`               |
| `region.capture`           | `{ prompt? }`                            | `{ page, box, png }` or `null` when the user pressed Escape                                                                    | `render:region`                |
| `equations.snapshot`       | `{ equation }`                           | `{ page, box, png }` of the equation you were invoked with                                                                     | `render:region`                |
| `equations.convert`        | `{ equation }`                           | `{ latex, display }` from the enabled `equationConverter`, or `null`                                                           | -                              |
| `ui.itemLine`              | `{ itemId, text, action? }`              | `{ shown }`                                                                                                                    | -                              |
| `pane.open`                | `{ pane, projectId? }`                   | `{ opened }` - your reader pane, when its `when` holds                                                                         | -                              |
| `projectView.open`         | `{ view, projectId?, offset? }`          | `{ opened }` - your pane of a project (the selected one by default), in the detail column, its item let go                     | -                              |

`kind` is `'doi'` or `'arxiv'`. `document.links` reports **every** outbound
link, including the many references cited by URL alone - a JMLR paper, a book on
Google Books, a Semantic Scholar entry. Its `citation` is filled in only for the
two schemes SpeedyF can resolve against your library, which is a statement about
what can be matched rather than about what counts as a reference. `citations.resolve`
answers `citeKey: null` for a work that is in the library but has no paper record
yet, and for one that is not in the library at all.

The `library.*` calls work with no document open; everything else rejects with
"no document is open". `library.papers` is every item in the library, in the
order the Projects screen lists them; an item's `projectIds` are the ids
`library.projects` reports. The scan takes some PDFs in a library folder for
something other than an item - a manual, a form, a slide deck - and leaves
those out of `library.papers`; a record reached by id may still be one, and
`isPaper` says so. Each record's metadata carries where it came from (`user`,
`crossref`, `inspire`, `datacite`, `arxiv`, `semantic-scholar`, `dblp`,
`openalex`, `open-library`, `xmp`, `identifier`, `citations`, `pdf-info`,
`first-page`, `filename`), so a
title the user typed can be told from one a heuristic guessed. Nothing here
can write: there is no `library:write` permission.

**Glyphs.** `selection.glyphs` and `region.glyphs` return the characters
whose box centre lies in the selection (or `bbox`), in the order the page
draws them, each as:

```ts
{
  text: string; // Unicode from the ToUnicode map; '' when the character has none
  code: number; // the raw code in the font when there is no Unicode; else the Unicode scalar
  font: string; // base font name, subset prefix stripped: 'CMMI10', 'CambriaMath'
  size: number; // font size in points, matrix-scaled
  x: number;
  y: number;
  w: number;
  h: number; // loose box, page points, top-left origin
  baseline: number; // baseline y, page points from the top
  bold: boolean;
  italic: boolean;
}
```

followed by the thin horizontal rules that touch the area - fraction bars,
radical overbars - as glyphs with `font: '__rule'`, `text: ''`, `code: 0`.
Coordinates here are top-left, y down, on the page as displayed; every other
API's `ApiBox` stays y-up. PDFium exposes no raw character codes for text it
can map to Unicode, which is why `code` falls back to the scalar there.

`document.mathTags` reads the page's structure tree: each `Formula` element
(and each MathML `math` root outside one) with its `/Alt` and `/ActualText`
and, when the tagging carries MathML structure, that structure serialised as
MathML. MathML attached as an associated file is not visible to SpeedyF, and
an untagged page answers `[]`.

`document.identifiers` answers from the library record for the open file
(which keeps the user's corrections), then from the library scan, then from a
reading of the document's first page; `arxivVersion` is the file's own version
(from the arXiv stamp, the id, or the file name).

**Showing a formula.** `ui.showMath` opens SpeedyF's math popover, anchored at
the selection or at the equation the user clicked: the LaTeX typeset by KaTeX
(in display mode when `display`), copy buttons for **LaTeX** (raw),
**Markdown** (`$...$`, or `$$...$$` when `display`) and, when the LaTeX breaks
lines with `\\`, **align** (`\begin{align} ... \end{align}`, taking the rows of
an `aligned` or `split` block across), and a dim note naming the `source`
(`tags` reads "PDF tags", `arxiv` "arXiv source", `jats` "journal version",
`fonts` "fonts") and the `confidence` (`high`, `medium`, `low`). Copying happens
on the button click, inside the user's gesture, so you do not need the
clipboard for this. The popover opens as a loading line the moment an equation
action (or a selection action of an extension with `document:layout`) starts;
if the command fails, its error is shown in the same popover. Escape or a
click elsewhere closes it.

`render.page` defaults to `scaleMilli: 2000` (roughly 2x), clamped to
200-4000. Rasters render at idle priority, so a long job never competes with
the reader's own scrolling.

**Clipboard timing.** The platform only permits a clipboard write while the user
action that started it is still in effect, and the trip out to your worker
spends part of that window. Write first, do slow work after.

---

## Describing UI

You do not render. You send a description and SpeedyF draws it with its own
components - which is why a contributed panel matches the app and follows every
restyle, including dark mode.

```js
await call('ui.render', {
  panel: 'main',
  ui: {
    sections: [
      {
        title: 'In your library',
        rows: [
          {
            id: 'a',
            title: 'Attention Is All You Need',
            subtitle: 'cited 3x - p. 4',
            badge: 'in library',
            action: { command: 'open', value: '1706.03762' },
          },
        ],
      },
    ],
  },
});
```

- `rows` at the top level is shorthand for one untitled section.
- `empty` is shown instead of the sections when there is nothing to list.
- `error` is shown above everything, in the house error style.
- A row's `action.command` must be one of _your_ commands; it is invoked with
  `value` as the invoke's `value`.
- A row with no `action` is inert - not focusable, not clickable.
- A row's `title` and `subtitle` may carry TeX between `$...$` (or `\(...\)`),
  typeset with KaTeX the way paper titles are elsewhere in SpeedyF:
  `{ title: '$\\lambda$', subtitle: 'the control parameter' }`. Everything
  else is shown as plain, escaped text; write a literal dollar sign as `\$`.

Your panel is rendered when the user opens it and when they press Refresh, not
on a timer. Handle `invoke` with `reason: 'panel'` by drawing.

Malformed rows are dropped individually, over-long text is truncated, and
anything unreadable is ignored rather than costing you the whole panel.

---

## Limits

| Thing                                     | Limit                                                                                     |
| ----------------------------------------- | ----------------------------------------------------------------------------------------- |
| Time to answer an `invoke`                | 5 s before a timeout is reported                                                          |
| Time for one `referenceProvider` document | 20 s                                                                                      |
| Text handed to a reference provider       | 2 MB (the end of a longer text)                                                           |
| `references.contribute` entries           | 1 000; each `text` 1 500 characters, then truncated; malformed entries dropped one by one |
| Rows in one panel description             | 500 total, across all sections                                                            |
| Sections                                  | 50                                                                                        |
| Any single string in a description        | 2 000 characters, then truncated                                                          |
| `clipboard.write`                         | 100 000 characters                                                                        |
| `storage` value                           | 64 000 characters                                                                         |
| `storage` keys per extension              | 100                                                                                       |
| `document.contributeText` per page        | 200 000 characters                                                                        |
| `network` hosts in a manifest             | 20                                                                                        |
| `net.fetch` response                      | 5 MB by default (`maxBytes` to change), 20 MB at most; larger is refused, not truncated   |
| `net.fetch` duration                      | 30 s, connect to last byte                                                                |
| `net.fetch` pacing                        | per host, shared with library lookups; 1 s between requests to a host without its own row |
| Time to answer an `invoke` with `network` | 60 s                                                                                      |
| `ui.showMath` LaTeX                       | 20 000 characters                                                                         |
| A project file                            | 8 MB; a path is plain names (letters, digits, `.`, `_`, `-`), at most four parts          |
| `editor.insert` text                      | 200 000 characters                                                                        |
| `editor.citations` entries                | 20 000                                                                                    |
| An invoke waiting on the user             | the clock stops while SpeedyF waits for a rectangle or a project pick                     |

Storage and settings are namespaced to your extension and removed when it is
uninstalled. Contributed text sidecars deliberately **survive** uninstall -
losing a scanned document's text months later would be surprising. The user can
discard them with **Forget Text Contributed By Extensions**.

---

## Installing and iterating

**Home -> Extensions**:

- **Load folder...** - point at your extension directory.
- **Install** - paste a git URL in the field beside it, optionally pinned:
  `https://.../repo#v1.2.0`.
  `https://` and `http://` only. Requires `git` on the machine.
- **Reload** - restarts your worker with the current files.
- **Remove** - deletes files, storage and settings.

Reinstalling stops the running worker, so your new code takes effect. Errors,
timeouts, refused permissions and your own `log` messages all appear in the
extension's row - there is no console.

---

## What extensions cannot do

Not oversights; each is deliberate, and listed so you find out now rather than
three files in.

- **Reach the network from the worker.** Structurally impossible under the
  current CSP. With `network`, SpeedyF fetches the hosts you list on your
  behalf, and nothing else.
- **Draw arbitrary UI.** Only the vocabulary above, and the editor SpeedyF
  draws for a pane or view. No canvas, no custom layout, no iframe.
- **Read or write files.** No filesystem access, and no path to the open
  document - `document.info` reports a name, never a path. The one
  exception is your own project files (`projects:files`), named by a
  relative path SpeedyF resolves inside your folder.
- **Get the PDF bytes.** There is no `document.bytes`.
- **Edit the document.** No annotations, no page operations, no saving.
- **Run SpeedyF's File, Edit, Page, Tools or Settings commands.**
- **Reach another document.** Only the active one - except a reference
  provider, which is handed library documents' text, anonymously, and can
  only answer with their reference entries.
- **Have contributed text used by SpeedyF itself.** You can store an OCR text
  layer, but the app's own search and text selection do not read it yet.
