# Writing a SpeedyF extension

Everything an extension can do, and how. For _why_ it works this way, see
`extension_api.md` — this file is the reference.

SpeedyF ships with References already installed; its source is in
`extensions/`, and a built-in is an ordinary extension that happens to live in
the app bundle — see that folder's README for the three ways it differs.

Working examples live in `examples/extensions/`. Load any of them from
**Home → Extensions → Load folder…**.

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
- **You cannot reach the network.** Not a policy — the worker inherits a
  Content Security Policy that permits no outbound connections at all.

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
  "contributes": {
    "commands": [{ "id": "run", "title": "Thing: Run", "keywords": ["go"] }],
    "keybindings": [{ "command": "run", "key": "mod+shift+t" }],
    "panels": [{ "id": "main", "title": "Thing" }],
    "selectionActions": [{ "command": "run", "title": "Send to Thing" }],
    "citationActions": [{ "command": "run", "title": "Look up in Thing" }],
    "documentTasks": [{ "command": "run", "title": "Thing: Scan Document" }],
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
| `icon`        | no       | Relative `.png`, `.svg`, `.jpg`. Drawn as a mask in the current text colour, so design it as a **single-colour silhouette** — only the shape survives. Without one, SpeedyF draws a generic mark with your initials. |
| `permissions` | no       | See below. Anything not requested is refused at the API boundary.                                                                                                                                                    |
| `contributes` | no       | Where you appear. See below.                                                                                                                                                                                         |

Command and panel ids must be non-empty and contain no `/` — that character
separates them from your extension id. Every `keybindings`, `selectionActions`,
`citationActions` and `documentTasks` entry must name a command you declared.

---

## Permissions

Declared in the manifest, refused if not declared. Absent = refused.

| Permission                | Grants                                                                                                                                                                            |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `document:read`           | The open document's text, outline and figures. Note this is the **whole document's text** — page text in a loop is everything it says.                                            |
| `citations:read`          | Citations found in the document, and resolving them against your library.                                                                                                         |
| `library:read`            | Searching the library.                                                                                                                                                            |
| `commands:run`            | Running SpeedyF's own commands — **View, Navigate, Panels and Search only**. Never File, Edit, Page, Tools or Settings, so an extension cannot delete a page or overwrite a file. |
| `clipboard:write`         | Writing text to the clipboard.                                                                                                                                                    |
| `storage`                 | A private key/value store, namespaced to you.                                                                                                                                     |
| `render:read`             | Page rasters as PNG bytes.                                                                                                                                                        |
| `document:contributeText` | Writing a text layer for a document, stored in a sidecar.                                                                                                                         |

`ui.render` and `settings.get` need no permission: drawing into a panel you
declared, and reading settings the user set for you, are not access to anything.

Selection and citation actions need **no permission at all** — you are handed
exactly what the user picked and nothing else. Prefer them over `document:read`
whenever they suffice.

---

## Contribution points

| Point              | What it does                                                                           |
| ------------------ | -------------------------------------------------------------------------------------- |
| `commands`         | Entries in the command palette (⌘⇧P).                                                  |
| `keybindings`      | A key chord that invokes one of your commands.                                         |
| `panels`           | A tab in the sidebar, drawn from a description you send.                               |
| `selectionActions` | Offered in the palette when text is selected; your command receives the selected text. |
| `citationActions`  | A button on the citation hover popover; your command receives `doi:…` or `arxiv:…`.    |
| `documentTasks`    | A batch job over a whole document, offered in the palette.                             |
| `settings`         | Fields rendered in the Extensions panel; `type` is `text`, `checkbox` or `select`.     |

**Key chords** use `mod` (⌘ on macOS, Ctrl elsewhere), `shift`, `alt`, then the
key: `j`, `mod+k`, `shift+g`, `alt+ArrowDown`. Chords SpeedyF already uses are
refused and reported in the Extensions panel — built-ins always win. If two
extensions want the same chord, the first installed keeps it.

A keybinding on a command that is _also_ a selection action receives the current
selection, exactly as the palette does.

---

## Messages

Asynchronous request/response with correlation ids, both directions.

### SpeedyF → you

```js
{ type: 'activate', id, apiVersion, permissions }   // once, before anything else
{ type: 'invoke', requestId, command, value?, reason? }
{ type: 'reply', requestId, ok, value?, error? }    // answer to your `call`
```

`value` carries the selected text or citation for an action. `reason` is
`'panel'` when your panel was opened rather than a command being run.

### You → SpeedyF

```js
{ type: 'call', requestId, api, args }   // ask SpeedyF to do something
{ type: 'done', requestId, ok, error? }  // finish an invoke
{ type: 'log', level, message }          // 'info' | 'warn' | 'error'
```

**Always answer an `invoke` with a `done` carrying the same `requestId`.** If you
do not within **5 seconds**, SpeedyF stops waiting and reports a timeout against
your extension. It does not kill your worker — a slow command is not a broken
extension — but nothing in the UI ever blocks on you.

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

| API                        | Args                    | Returns                                     | Needs                     |
| -------------------------- | ----------------------- | ------------------------------------------- | ------------------------- |
| `document.info`            | —                       | `{ name, pageCount, currentPage }`          | `document:read`           |
| `document.pageText`        | `{ page }`              | `string`                                    | `document:read`           |
| `document.outline`         | —                       | `[{ title, page, children }]`               | `document:read`           |
| `document.figures`         | —                       | `[{ label, caption, page }]`                | `document:read`           |
| `citations.list`           | `{ page }`              | `[{ kind, value, page }]`                   | `citations:read`          |
| `citations.resolve`        | `{ kind, value }`       | `{ inLibrary, title, fileName, pageCount }` | `citations:read`          |
| `library.search`           | `{ query }`             | `[{ name, title, matches }]`                | `library:read`            |
| `commands.run`             | `{ id }`                | `{ ran }`                                   | `commands:run`            |
| `render.page`              | `{ page, scaleMilli? }` | `number[]` — PNG bytes                      | `render:read`             |
| `document.contributeText`  | `{ page, text }`        | `{ stored }`                                | `document:contributeText` |
| `document.contributedText` | —                       | `{ extensionId, pages } \| null`            | `document:contributeText` |
| `clipboard.write`          | `{ text }`              | `{ written }`                               | `clipboard:write`         |
| `storage.get`              | `{ key }`               | `string \| null`                            | `storage`                 |
| `storage.set`              | `{ key, value }`        | `{ stored }`                                | `storage`                 |
| `storage.remove`           | `{ key }`               | `{ removed }`                               | `storage`                 |
| `settings.get`             | `{ key }`               | `string`                                    | —                         |
| `ui.render`                | `{ panel, ui }`         | `{ rendered }`                              | —                         |

`kind` is `'doi'` or `'arxiv'`. `document.links` reports **every** outbound
link, including the many references cited by URL alone — a JMLR paper, a book on
Google Books, a Semantic Scholar entry. Its `citation` is filled in only for the
two schemes SpeedyF can resolve against your library, which is a statement about
what can be matched rather than about what counts as a reference. `library.search` works with no document open;
everything else rejects with "no document is open".

`render.page` defaults to `scaleMilli: 2000` (roughly 2×), clamped to
200–4000. Rasters render at idle priority, so a long job never competes with
the reader's own scrolling.

**Clipboard timing.** The platform only permits a clipboard write while the user
action that started it is still in effect, and the trip out to your worker
spends part of that window. Write first, do slow work after.

---

## Describing UI

You do not render. You send a description and SpeedyF draws it with its own
components — which is why a contributed panel matches the app and follows every
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
            subtitle: 'cited 3× · p. 4',
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
- A row with no `action` is inert — not focusable, not clickable.

Your panel is rendered when the user opens it and when they press Refresh, not
on a timer. Handle `invoke` with `reason: 'panel'` by drawing.

Malformed rows are dropped individually, over-long text is truncated, and
anything unreadable is ignored rather than costing you the whole panel.

---

## Limits

| Thing                              | Limit                            |
| ---------------------------------- | -------------------------------- |
| Time to answer an `invoke`         | 5 s before a timeout is reported |
| Rows in one panel description      | 500 total, across all sections   |
| Sections                           | 50                               |
| Any single string in a description | 2 000 characters, then truncated |
| `clipboard.write`                  | 100 000 characters               |
| `storage` value                    | 64 000 characters                |
| `storage` keys per extension       | 100                              |
| `document.contributeText` per page | 200 000 characters               |

Storage and settings are namespaced to your extension and removed when it is
uninstalled. Contributed text sidecars deliberately **survive** uninstall —
losing a scanned document's text months later would be surprising. The user can
discard them with **Forget Text Contributed By Extensions**.

---

## Installing and iterating

**Home → Extensions**:

- **Load folder…** — point at your extension directory.
- **Install** — paste a git URL in the field beside it, optionally pinned:
  `https://…/repo#v1.2.0`.
  `https://` and `http://` only. Requires `git` on the machine.
- **Reload** — restarts your worker with the current files.
- **Remove** — deletes files, storage and settings.

Reinstalling stops the running worker, so your new code takes effect. Errors,
timeouts, refused permissions and your own `log` messages all appear in the
extension's row — there is no console.

---

## What extensions cannot do

Not oversights; each is deliberate, and listed so you find out now rather than
three files in.

- **Reach the network.** Structurally impossible under the current CSP.
- **Draw arbitrary UI.** Only the vocabulary above. No canvas, no custom layout.
- **Read or write files.** No filesystem access, and no path to the open
  document — `document.info` reports a name, never a path.
- **Get the PDF bytes.** There is no `document.bytes`.
- **Edit the document.** No annotations, no page operations, no saving.
- **Run SpeedyF's File, Edit, Page, Tools or Settings commands.**
- **Reach another document.** Only the active one.
- **Have contributed text used by SpeedyF itself.** You can store an OCR text
  layer, but the app's own search and text selection do not read it yet.
