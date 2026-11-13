# SpeedyF Projects — design plan

**Status:** implemented (phases 1 to 4) on 2026-11-12, with two additions the
design did not foresee: file-derived metadata is re-derived on every rescan
(§2), and the reader classifies an open PDF before offering to file it (§6),
because SpeedyF is a viewer first and not every PDF is a paper. The
bibliography parser of §8 was not ported to Rust: it runs in the References
extension as the library's reference provider.

**What it is:** the part of a reference manager SpeedyF does not have. Papers
grouped into projects, real metadata for each paper, a cite key, and a `.bib`
per project that you point LaTeX at. It is a custom alternative to a reference
manager, not a bridge to one: nothing here reads or writes another tool's
database.

**What already exists and is reused:** the library (folders of PDFs, scanned
for DOI, arXiv id and a first-line title; full-text sidecars and a trigram
index; `library-index.json` in app data), citation resolution against it, the
capped pool of "library documents" the engine opens for hover previews, and
the References extension's reading of a paper's bibliography.

---

## 1. Decisions already made

Each was settled on 2026-11-11 and is recorded here so it is not re-argued
three files in.

| Question             | Decision                                                                                                                                                                                                                                             |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Core or extension    | **Core.** Extensions get read access to the result. The extension API has no library write, no network, no filesystem and declarative UI only; building those first for one consumer is the bespoke-integration trap `extension_api.md` warns about. |
| Unit of grouping     | **Projects.** Flat, a paper can be in many, each has its own `.bib`. Not nested collections. Tags and saved searches come later, if at all.                                                                                                          |
| Where it appears     | **A second launch screen**, behind a Home / Projects toggle at the top right of the launch screen. Not a panel in the Home rail.                                                                                                                     |
| Layout               | **Three columns**: projects, papers, the selected paper. With _All papers_ selected the middle list is grouped by project with collapsible headers; with a project selected it is flat. No cards.                                                    |
| Files                | **File-first.** The PDFs in the library folders stay the source of truth; projects and metadata are layered over them. A paper added from outside the folders is **copied** into a chosen inbox folder, never moved.                                 |
| Where the data lives | **Plain JSON** in a user-chosen folder, defaulting to app data, so Dropbox, iCloud or git can sync it. No SQLite.                                                                                                                                    |
| Metadata sources     | Local first: PDF Info dictionary and XMP, the first-page heuristic, the filename. Then **opt-in online lookup** by DOI and arXiv id. Title lookup only on request, per paper.                                                                        |
| `.bib` flavour       | **Classic BibTeX.** `journal`, `year`, `volume`, `pages`; arXiv-only works as `@misc` with `eprint`.                                                                                                                                                 |
| `.bib` ownership     | **SpeedyF owns the file.** Export on demand from the project, plus an optional synced path per project that SpeedyF rewrites on every change. Hand edits to a synced file are lost by design.                                                        |
| Cites and cited-by   | In the reader's Paper panel, once the graph exists. The References extension stays as it is.                                                                                                                                                         |

---

## 2. The model

Four kinds of record. Everything else is derived.

**Paper.** One work. Has an id that never changes, one or more files, zero or
more identifiers (DOI, arXiv), metadata with per-field provenance, and a cite
key.

**File.** A PDF on disk, as the library scan found it: canonical path, size,
mtime, and a content fingerprint. A paper usually has one; the arXiv preprint
and the journal version of the same work are two files on one paper.

**Project.** A name, an ordered position, the ids of its papers, and an
optional path to a synced `.bib`.

**Membership.** Paper in project. Many-to-many, held on the project as a list
of paper ids.

Every scanned PDF gets a paper record. That is what makes _All papers_ the
whole library and keeps cite keys stable from the moment a file is seen,
rather than from the moment the user first touches it.

### Identity: how a file finds its paper

The library index is keyed by path and discards an entry when the file moves.
A project must survive that, so a paper is matched to a scanned file in this
order and the first hit wins:

1. **Identifier.** The scan's DOI or arXiv id equals the paper's.
2. **Content fingerprint.** Size plus a hash of the first and last 64 KiB.
   Cheap enough to compute in the scan, stable across renames and moves,
   different the moment the file is re-saved with annotations — which is why
   it comes after the identifier and before the path.
3. **Path.** The canonical path recorded on the file.

No match creates a new paper. Two files that resolve to the same paper are
both kept on it. A file that disappears leaves the paper in place with the
file marked missing: the paper stays in its projects and its `.bib`, the UI
dims it and says so, and the next file that matches by identifier or
fingerprint reattaches.

**Automatic merging.** Two item records that are plainly one work become
one, through the ordinary merge (snapshots, so Split restores them):

- a shared DOI or arXiv id, aliases included (an arXiv DOI
  `10.48550/arxiv.X` counts as arXiv id X); or
- the same normalized title of four words or more, the same first-author
  family name (case and accents folded), known years at most one apart, and
  no conflicting identifiers (two different DOIs, or two different arXiv
  ids, are two works).

Never a pair the user dismissed (directly or through a third record), a
non-item, or anything in a read-only store. It runs whenever an identifier
or a lookup lands on a record (a lookup, an edited DOI or arXiv id, a scan's
reconcile) and once over the whole library on load. The keeper is the
record with more projects, then a pinned key, then user edits, then more
files, then the earlier `addedAt`, then the smaller id; the reason is kept
on it and shown beside Split. Splitting an automatic merge records the
records as not duplicates, so they are not merged again.

All of an item's files stay visible: the detail column lists them, and any
can be opened or made the main one (`files[0]`: the one Open, previews and
the scan's fields use), which a rescan keeps.

### Metadata with provenance

Each field carries where it came from:

```
title, authors, year, venue, volume, issue, pages, publisher, abstract, kind
source ∈ user | crossref | arxiv | openalex | pdf-info | first-page | filename
```

Precedence is fixed and the rule is one line: **a value never loses to a
weaker source.** A user edit is never overwritten. A lookup overwrites the
local heuristics. A rescan never overwrites a lookup. Without the provenance
there is no way to tell a title the user typed from one the heuristic guessed,
and the first rescan would clobber it.

The complement: **what the file itself said is re-derived on every rescan.**
A title the Info dictionary gave last time and no longer gives (the filter got
stricter, the file was replaced) is dropped and the first page's title takes
its place. Only lookups and user edits outlive the evidence for them. Judged
from the paper's primary file, so a second file without a Title cannot keep
retracting what the first supplies.

### Cite keys

Generated as `{family}{year}{word}`: the first author's family name, the year,
the first title word that is not a stop word, all lower-case ASCII, so
`jarzynski1997nonequilibrium`. Missing parts degrade in place: no author gives
`anon`, no year gives `nd`. Collisions take `a`, `b`, `c` in order of
creation. The pattern is fixed in the first version; a setting can come later.

**A key is frozen the first time it is written to any `.bib`.** Until then it
follows the metadata, so a lookup that discovers the real author fixes the
key. After that it is pinned, because a key that changes under a manuscript
breaks every `\cite` in it. The user can unpin and regenerate deliberately.
Keys are unique across the whole library, so the same paper has the same key
in every project's `.bib`.

---

## 3. On disk

```
<library data dir>/
  library.json          # papers, projects, memberships — the user's work
<app data dir>/
  library-index.json    # the scan cache, as today — disposable
  library-root.json     # the folders, as today
  lookup-cache.json     # raw lookup responses by identifier — disposable
  text/                 # full-text sidecars, as today
```

The split is between what can be regenerated and what cannot. The index and
the sidecars are rebuilt from the folders; `library.json` is the only file
that holds something the user did, and the only one that moves when they
choose a different data folder. Default: the app data dir. **Settings → Library
data folder** moves it, copying the file and then reading from the new place.

Written atomically: serialize to a temp file in the same directory, then
rename over the old one. The existing index is written with a plain
`fs::write`, which is acceptable for a cache and not for this file.

`library.json`, version 1:

```json
{
  "version": 1,
  "papers": [
    {
      "id": "7c1e0b5a-…",
      "files": [
        {
          "path": "/Users/me/Papers/noneq/jarzynski1997.pdf",
          "size": 310221,
          "mtimeMs": 1699000000000,
          "fingerprint": "f1a2…",
          "missing": false
        }
      ],
      "ids": { "doi": "10.1103/physrevlett.78.2690", "arxiv": null },
      "meta": {
        "title": {
          "value": "Nonequilibrium Equality for Free Energy Differences",
          "source": "crossref"
        },
        "authors": { "value": [{ "family": "Jarzynski", "given": "C." }], "source": "crossref" },
        "year": { "value": 1997, "source": "crossref" },
        "venue": { "value": "Physical Review Letters", "source": "crossref" },
        "volume": { "value": "78", "source": "crossref" },
        "pages": { "value": "2690-2693", "source": "crossref" },
        "kind": { "value": "article", "source": "crossref" }
      },
      "citeKey": "jarzynski1997nonequilibrium",
      "citeKeyPinned": true,
      "addedAt": "2026-11-11T20:00:00Z",
      "updatedAt": "2026-11-11T20:05:00Z"
    }
  ],
  "projects": [
    {
      "id": "3b9d…",
      "name": "Noneq stat mech",
      "order": 0,
      "paperIds": ["7c1e0b5a-…"],
      "bibPath": "/Users/me/papers/noneq/refs.bib",
      "createdAt": "2026-11-11T20:00:00Z"
    }
  ]
}
```

Ids are UUID v4 (new `uuid` dependency). The file carries a version from day
one; an index written without one was silently discarded once already, and
`library/mod.rs` still carries the scar.

Multiple windows share one file. Every write goes through the engine's one
library state, and after each write the backend emits a `library:changed`
event so every window's store refreshes. No window may assume it is the only
writer.

---

## 4. Metadata: where it comes from

### Local, during the scan

The scan already reads the first page's text. It gains two things at the same
point, on the engine thread, at no extra I/O:

- **The PDF Info dictionary** through `PdfDocument::metadata()`: Title,
  Author, Subject, Keywords, CreationDate. Publisher PDFs fill Title and
  Author reliably; arXiv PDFs often carry a Title. These rank above the
  first-page heuristic and below any lookup. A Title that is a filename, empty,
  or "Untitled" is ignored, which is common enough to need the rule.
- **The content fingerprint** for identity.

- **The XMP packet**, when the file has one: the publisher's typesetter writes
  `dc:title`, `dc:creator`, `prism:doi` or `pdfx:doi`, the journal name,
  volume, issue and pages, and the publication date. It ranks above the Info
  dictionary. In the first real library (118 PDFs from a Downloads folder) 43
  files had a packet, 32 with a title and 2 with a DOI: a title source more
  than an identifier source.
- **An ISBN** on the first page, checksum-validated, for books.
- **What an identifier itself says.** An arXiv id encodes its submission
  year and month, and the first-page stamp its category; a DOI from several
  publisher families (Elsevier `j.journal.2020.`, Springer Nature `s…-020-`,
  IEEE `.2019.`, Wiley) carries its year; and an Elsevier PII such as
  `0022-0000(88)90028-1`, which old scans print instead of a DOI, carries
  its year in the parentheses and converts to the DOI `10.1016/<PII>`, which
  then unlocks a Crossref lookup. These land at the `identifier` rank: above
  the Info dictionary, below XMP and every lookup.
- **The verdict.** The classifier of §6 runs on every file during the scan,
  from the first page and Info dictionary, and from pages two to four as
  well when the first says little, because a thesis or a textbook puts the
  decisive page after the cover. Theses ("a dissertation submitted", the
  degree phrasing) and books (an ISBN, a press with an edition, a copyright
  page) are items with their kind set, so the BibTeX comes out as
  `@phdthesis` or `@book` without a lookup. The record carries its verdict,
  kind and reason. Records that are not papers sit under _Other PDFs_ on
  the Projects screen, are left out of _All papers_ and the counts, are never
  looked up by a batch, and show no cite key. The user can overrule either
  way, and a marked record is never dropped as a placeholder.

- **OCR for scanned papers** (macOS only; `engine::ocr`, `library::ocrtext`).
  Old publisher scans have a text layer that is missing, or capture-tool
  garbage ("~ ,;l1 Il"), or good except for a title that is still a picture.
  The scan probes each page it reads, at no extra I/O: its text against a
  garbage heuristic (`ocr::needs_ocr`: too few characters on a page with
  images, too few letters, too many 1–2 character tokens, too many
  vowel-less words) and its image objects for one covering ≥ 60 % of the
  page (a scan). A page that needs it, and a scanned first page whose layout
  gave no confident title, are rendered at 2x and read by Apple's Vision
  (`VNRecognizeTextRequest`, accurate, language correction on). The OCR text
  replaces the page's text for classification, identifiers and the ISBN, and
  its lines go through the same first-page reader for the title and authors.
  The text pass does the same for every page whose text layer needs it, at
  most six pages per idle job (a long scan resumes on the next), and marks
  those pages `ocr` in the text sidecar; a document whose first three pages
  are clean and whose first is not a scan is never rendered. Every reading
  is cached per page beside the sidecar (`text/<hash>.ocr.json`), keyed by
  the content fingerprint, so no page is read twice across rescans.
  Cost: ~0.4 s a page on Apple silicon, on the engine thread — so only from
  the idle-priority library jobs, never from a scan the user is waiting for
  (an inbox copy reads the cache only and is scanned again at idle,
  `ocr_deferred`). The first Vision call of a session loads its model and
  can take tens of seconds. Elsewhere it compiles to nothing: Windows and
  Linux keep the text layer. Follow-up: feed the OCR text to
  `document:contributeText` so search in an open scan finds it too (an OCR
  page's offsets are not PDFium character indices, so its hits need their
  own highlighting).

The first-line title heuristic and the filename patterns stay as the floor.

### Online, opt-in

Off until **Settings → Look up paper metadata online** is turned on. This is
the first network call SpeedyF makes, so it meets the bar `extension_api.md`
set for extensions wanting the same: a plain statement of what leaves the
machine and an indicator in the status bar while a lookup runs.

What leaves the machine, once the setting is on: DOIs, arXiv ids, ISBNs, and
for a paper that has none of them its first-page title. Never file contents.
Only records the scan (or the user) takes for papers are looked up at all.

| Have     | Ask, in order                                                                                                                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| DOI      | Crossref; DataCite if Crossref has no record; OpenAlex only when venue, volume or pages are still missing; Semantic Scholar (abstract, and the references for §8); INSPIRE when the paper looks like physics |
| arXiv id | arXiv (which may yield the DOI, then the row above); INSPIRE when physics; Semantic Scholar; OpenAlex only when the venue is still missing                                                                   |
| ISBN     | Open Library                                                                                                                                                                                                 |
| none     | Title search on OpenAlex, Crossref, arXiv, Semantic Scholar, and DBLP when the paper looks like computer science; the answers are merged                                                                     |

A source is skipped when it cannot add anything the record lacks, which is
what keeps a batch from asking eight services about every paper.

A title search applies its best match on its own when the normalized titles
are equal, or nearly equal with the same year. Anything weaker is **parked**
on the paper as candidates: the detail column lists them and the user picks
one or dismisses them, and a batch does not search that paper again while
the question is open. This was the user's call (2026-11-12): exact or
near-with-year applies, titles are sent without a per-run confirmation.

Batches run for a project from its header and for the whole library from
_All papers_, one paper at a time, and skip papers looked up in the last
thirty days unless they are still incomplete. The status bar shows how far
along a batch is.

**Rate limits.** Every request goes through one per-host queue: one request
in flight per service, each service's own minimum spacing (arXiv's terms ask
for three seconds; Semantic Scholar one second with a key and three without;
Crossref, OpenAlex and DataCite a tenth of a second in their polite pools;
INSPIRE half a second; DBLP and Open Library one second), a `Retry-After` on
a 429 honoured to the second, exponential backoff from two seconds to two
minutes when there is none, and one retry after a timeout or a 5xx. Answers
and misses are cached in `lookup-cache.json` (misses for a week), so a re-run
costs nothing and a rescan never re-fetches. An optional Semantic Scholar key
(**Settings**) buys the better rate limit and nothing else.

Crossref asks politely-identified clients to include a contact address in the
User-Agent in exchange for better service; **Settings → Contact for lookups**
is optional and empty by default. Requests go through `reqwest` with
`rustls-tls` from the Rust side; the webview's CSP is untouched. The arXiv API
answers in Atom XML, parsed with `quick-xml`. Google Scholar is not a source:
it has no API and blocks automated clients.

---

## 5. The Projects launch screen

`App.tsx` shows `HomeScreen` when no tab is active. It will show one of two
launch screens, chosen by `settings.launchScreen`, and each carries the toggle
at the top right of its header. The tab strip's Home button and the
_Go to Home Screen_ command return to whichever one is set.

```
 ┌─ projects ─────┬─ papers ──────────────────────────┬─ paper ───────────────┐
 │ All papers  412│ All papers        [Home | Projects]│ ┌───────────────┐     │
 │ Not filed    37│ [search…]                          │ │ first page    │     │
 │ ───────────────│ ▾ Noneq stat mech  23              │ │               │     │
 │ Noneq stat mech│    Nonequilibrium equality…  1997  │ └───────────────┘     │
 │ Active matter  │    Entropy production…       1999  │ Jarzynski · 1997      │
 │ Thesis         │ ▸ Active matter  41                │ Physical Review Lett. │
 │                │ ▾ Thesis  117                      │ doi 10.1103/…         │
 │ + New project  │    …                               │ key jarzynski1997…    │
 │                │                                    │ ☑ Noneq  ☐ Active     │
 │                │                                    │ [Open] [Copy key]     │
 └────────────────┴────────────────────────────────────┴───────────────────────┘
```

**Left column.** _All papers_, _Not filed_ and _Other PDFs_ with counts, a
rule, then the projects in their stored order, then _New project_. _Other
PDFs_ holds what the scan does not take for a paper; a paper misjudged there
is one click from being one, and anything may still be dragged into a project. Projects are renamed in
place and reordered by drag. Deleting a project removes memberships and
nothing else; files and papers are untouched. Dropping paper rows onto a
project adds them.

**Middle column.** Header: the selection's name and count; for a project also
its synced `.bib` path or "no .bib", **Export .bib…**, and the incomplete-entry
count. Then the search field, then the list.

- Under _All papers_ the list is grouped by project, each group collapsible,
  with _Not filed_ last. A paper in three projects appears three times; that
  is the truth of the data and the point of the overview.
- Under a project the list is flat.
- A row is the title, then authors, year and venue dimmed. A missing file dims
  the whole row. The cite key appears at the right on hover.
- Search matches title, authors, year and key as you type. Below the matches a
  single row, _Search the full text of every paper for "…"_, runs the existing
  trigram search, because "which paper said this" is a different question and
  already has an answer.
- Selecting a row fills the right column; Enter or double-click opens the
  paper in a tab, through `openInNewTabOrFocus`, exactly as the Library panel
  does today.

**Right column.** The first page, rendered through the engine's library
document pool by a new by-path variant of citation resolve, so it costs what a
hover preview costs. Beneath it the metadata, each field editable in place
(an edit sets its source to `user`), the identifiers, the cite key with
**Copy**, the file path, and a checkbox per project. Buttons: **Open**,
**Copy BibTeX**, **Look up** (when lookup is on), **Find by title** (when the
paper has no identifier).

**Copy \cite{}.** With one or more rows selected (click, shift-click,
cmd/ctrl-click: the same selection a drag carries), the list header offers
**Copy \cite{}** beside _Merge N items_. It writes `\cite{key1,key2,...}`
with the keys in the list's visible order, each once, from the rows already
in memory, so the write lands inside the click; then `library_pin_keys`
pins the keys, as copying BibTeX does (a key that changes under a
manuscript breaks the `\cite`). The Library command _Copy \cite for
Selection_ (Cmd/Ctrl+Option/Alt+C, matched on the physical key) does the
same for the list's selection, and with no list on screen copies
`\cite{key}` for the open document's item.

**Drag and drop** is done with pointer events, not HTML5 drag and drop. The
app keeps Tauri's native file drop on so a PDF from Finder arrives with its
path, and Tauri's runtime answers every drag itself, so the webview never
sees an HTML5 drag, internal ones included (verified against tauri-runtime-wry
2.11: its handler returns `true` unconditionally, and this version has no
hook to change that). A Finder drop that lands on a project row files the PDF
into the inbox and adds it to that project; elsewhere on the screen it files
without a project; anywhere else in the app it opens, as before.

**The noun is "item".** Papers, theses and books all live in the library, so
the screen says _All items_, the detail says "Looks like a thesis: …", and
the reader's panel is called _Library_. Internal names keep "paper".

**Keyboard.** Up and down move within a column, left and right move between
columns, Enter opens. The copy-cite-key chord is chosen at implementation
against `reservedChords()` so it collides with nothing SpeedyF already binds.

**Empty states.** No library folders: the screen says so and offers _Add
folder…_. No projects: the left column holds only the two fixed rows and _New
project_. Nothing selected: the right column is blank.

**Narrow windows.** Below roughly 900px the right column collapses behind a
toggle; the Home rail is already 240px, so this was going to come up.

The existing Library panel on Home keeps its folders and full-text search; it
is not duplicated and not removed.

---

## 6. In the reader

A sixth sidebar mode, `paper`, beside Pages, Outline, Formal, Figures and
Notes. `DEFAULT_SIDEBAR_WIDTH` is sized for five buttons and grows to fit six.

The **Paper panel** shows, for the open document: title, authors, year, cite
key with Copy, a checkbox per project, **Copy BibTeX**, **Look up**. When the
open file is not in any library folder it first asks the engine whether the
document looks like a paper: an arXiv stamp, a DOI or a publisher's name on
the first page, a journal typesetter in the Info dictionary, or the shape of
an abstract with sections. A paper gets "This looks like a paper (arXiv:… on
the first page)" and a prominent **Add to library**; a manual, a form or a
scan gets one quiet line and the same button in its secondary style. Adding
copies the file into the inbox folder and then shows the panel proper.
Later it also lists which of your papers this one cites and which cite it
(section 8).

**Project suggestions.** Under the project checkboxes, in this panel and in
the Projects screen's detail column, at most one quiet line: "Add to
Thesis? (cites 2 papers in it)" with **Add** and **Not now**. It comes from
`library_suggest_projects`, which, for each project the item is not in,
counts the members the item cites, the members citing it, and the works
both the item and some member cite (by library record, identifier, or the
missing-works reference-text key). Score: `2 * (cites + cited_by) +
min(shared, 10)`, divided by the square root of the project's size so a
big project does not win on size alone; a project needs a citation either
way or at least 3 shared references; at most three, strongest first. Not
now is per item and project for the session, and the next suggestion takes
its place. Re-read on item change and on `library:changed`. Nothing is ever
filed without the click.

New commands, in a new `Library` group in `commands.ts`: _Copy Cite Key_,
_Copy BibTeX_, _Add to Project…_, _Look Up Metadata_, _Switch Launch Screen_.
The copy commands are also offered when a paper is selected on the Projects
screen.

### Filing

A paper reaches a project from outside the library folders in one of two
ways: adding the open document from the Paper panel, or adding a PDF the user
dropped on the Projects screen. Both go through the same path: **copy** into
`settings.libraryInbox`, a library folder chosen once (the only root if there
is one, otherwise asked for), then scan the copy, create the paper, add the
membership. Never a move, never a delete. Copying a file into a watched folder
is the one filesystem write this feature makes outside its own data files.

---

## 7. BibTeX

Written in Rust so the one-off export, the synced file and the clipboard copy
share one serializer.

| Paper has           | Entry                                                                            |
| ------------------- | -------------------------------------------------------------------------------- |
| a journal and a DOI | `@article{key, author, title, journal, year, volume, number, pages, doi}`        |
| a conference venue  | `@inproceedings{key, author, title, booktitle, year, pages, doi}`                |
| only an arXiv id    | `@misc{key, author, title, year, eprint, archivePrefix = {arXiv}, primaryClass}` |
| not enough to say   | `@misc{key, title, note = {…}}` preceded by `% incomplete: no author, no year`   |

Authors are written `Family, Given and Family, Given`. Titles are wrapped
`{{…}}` to protect case. The characters `& % $ # _ { } ~ ^ \` are escaped;
everything else is written as UTF-8, which every modern TeX reads.

**Export .bib…** opens a save dialog and writes the project's entries once.
**Sync to a .bib…** records a path on the project; from then on every change
to the project's membership or to any member's metadata rewrites that file,
atomically and debounced, and the project header shows the path. Keys are
frozen on first write (section 2). The header's incomplete count tells the
user what to look up before they compile.

---

## 8. The citation graph

Comes last, in two steps of very different size.

**Cheap edges.** The full-text sidecar for every library paper already exists.
Running `find_explicit_citations` over it during text indexing yields every
DOI and arXiv id the paper cites, stored on its index entry. Resolving those
against paper records gives _cites_ and _cited by_ for identifiers, which in
physics and ML covers most of the bibliography. Shown in the Paper panel as two
lists, each row opening that paper.

**The graph view.** The middle column has a _List | Graph_ toggle for a
project and for _All items_. The graph is the set's items as nodes and the
citations between them as arrows, laid out by a small deterministic force
simulation in plain SVG: click selects into the detail column, double-click
opens, hover lights up a node's neighbours, nodes drag, the wheel zooms.
Citations that leave the set are counted, not drawn, and items whose text
is not indexed yet are named, since their citations are not known rather
than absent. One engine call returns nodes and edges, with the engine's one
identifier table built once per call.

**Good edges.** The References extension reads a bibliography properly: it
follows each `[12]`'s internal link destination to cut the reference page into
entries with titles, authors and years. For the library-wide graph the
decision went the other way from the port this section first proposed:
reference-list parsing stays in the extension, and the extension gained a way
to see library documents. It declares `referenceProvider` and
`library:contributeReferences` (EXTENSIONS.md); during idle indexing the
window hands it each library document's stored text under an opaque token,
never a path, and it answers with the entries (`label`, `text`, `year`) cut by
a text parser (heading, numbered/bracketed/superscript chains, author-year
fallbacks). The engine keeps per entry what matching needs — the compact text,
the year, the explicit DOIs and arXiv ids and the DOI an APS journal reference
spells out — in a refs sidecar stamped with the provider and its version, and
matches entries to library titles. Disable References and no refs sidecar is
read: DOI, arXiv and APS-journal edges remain (the engine finds those in the
whole text itself), title-only edges go.

---

## 9. Surfaces

### Engine and commands

New `Work` variants: `LibraryPreview { path }` (opens a library document and
returns the same shape as citation resolve) and, in phase 4, cited-identifier
extraction inside `LibraryTextNext`. The scan gains the Info-dictionary read
and the fingerprint.

New Tauri commands, all going through the one library state and all emitting
`library:changed` after a write:

```
library_papers { filter }            → { papers, total }
library_paper { id }                 → paper
library_paper_for_path { path }      → paper | null
library_update_paper { id, patch }   → paper
library_projects                     → [project]
library_create_project { name }      → project
library_rename_project { id, name }
library_delete_project { id }
library_reorder_projects { ids }
library_set_membership { paperId, projectId, member }
library_preview { paperId }          → resolved (doc id + preview spec)
library_file_into_inbox { path }     → newPath
library_bibtex { paperIds }          → string
library_export_bib { projectId, path }
library_set_project_bib { projectId, path | null }
library_lookup { paperId, mode }     → applied | [candidate]        (phase 2)
library_data_dir / library_move_data { dir }
```

### Frontend

```
src/features/projects/
  ProjectsScreen.tsx     the launch screen, three columns
  ProjectsColumn.tsx     left
  PaperList.tsx          middle, grouped or flat
  PaperDetail.tsx        right
  PaperPanel.tsx         the reader's sidebar panel
  projectsStore.ts       papers, projects, selection; refreshes on library:changed
src/app/App.tsx          launch screen choice
src/features/home/HomeScreen.tsx   the toggle
src/stores/uiStore.ts    'paper' sidebar mode; six-button default width
src/stores/settings.ts   launchScreen, libraryInbox, onlineLookup, lookupContact, libraryDataDir
src/app/commands.ts      Library group
src/app/shortcuts.ts     chords
src/types/engine.ts, src/lib/transport/engine.ts   DTOs and calls
```

### Rust

```
src-tauri/src/library/
  mod.rs        scan gains Info dict + fingerprint; reconcile files → papers
  store.rs      library.json: load, save atomically, version
  papers.rs     paper records, identity, provenance rules
  projects.rs   projects and memberships
  citekey.rs    generation, collisions, freezing
  bibtex.rs     serializer
  filing.rs     copy into inbox
  lookup.rs     Crossref, arXiv, OpenAlex, cache          (phase 2)
src-tauri/src/commands/library.rs   the commands above
```

### Extension API

Under the existing `library:read`: `library.papers`, `library.projects`,
`library.paper { id }`, returning the public fields (never paths).
`citations.resolve` gains `citeKey`. No `library:write` in this plan; the
BibTeX example extension becomes a real one the day something needs it.

---

## Phase 1 — projects

**Goal:** the Projects screen works, a paper can be put in a project, and
nothing on disk is lost or overwritten.

**Ships:**

- `library.json`, atomic writes, versioned; data folder setting
- paper records for every scanned file; identity by id, fingerprint, path;
  missing-file handling
- Info-dictionary and fingerprint in the scan
- metadata with provenance; in-place editing
- cite keys, collisions, freezing rule (nothing freezes them yet)
- projects: create, rename, reorder, delete, membership, drag
- the Projects launch screen, the toggle, keyboard, empty states
- `library_preview` and the first-page column
- the Paper sidebar panel with projects and Copy Cite Key
- filing by copy into the inbox folder
- `library:changed` across windows
- extension API read calls

**Done when:** a library of a few thousand files scans into papers without
duplicates, a renamed file keeps its project, two windows see the same change,
and the release gate passes.

**Explicitly out:** network, `.bib`, tags, graph.

## Phase 2 — lookup

**Ships:** the opt-in setting and status indicator; Crossref and arXiv by
identifier, per paper and per project; OpenAlex by title on request with a
candidate picker; the response cache; `reqwest` and the XML dependency.

**Done when:** a project of identifier-bearing papers looks up in one action
and every field shows its source; turning the setting off makes no request.

## Phase 3 — `.bib`

**Ships:** the serializer and its tests; Copy BibTeX; Export .bib…; synced
paths with debounced atomic rewrites; key freezing on first write; the
incomplete count in the project header.

**Done when:** a synced `.bib` compiles under `bibtex` with the project's
papers, and a key that has been written never changes without the user
unpinning it.

## Phase 4 — graph

**Ships:** cited identifiers from sidecars; cites and cited-by in the Paper
panel. Then, as its own decision, the Rust port of the bibliography parser.

---

## 10. Cost

Phase 1 is the large one: most of the Rust data layer and all of the screen.
Phases 2 and 3 are each a fraction of it and are independent of each other,
though 3 is only useful after 2 has filled in authors and years. Phase 4's
first step is small; its second is comparable to phase 1 and should be
re-estimated when it is reached.

## 11. Risks

- **Duplicate papers.** Two files of one work with no identifier and
  different bytes become two papers. A manual _Merge_ is the fix and is cheap
  once records exist; it is not in phase 1.
- **The heuristic title.** Until lookup runs, many papers show a first line
  that is a running header. The Info dictionary improves this; it does not fix
  it. The screen must look acceptable with bad titles.
- **Sync conflicts.** Two machines writing `library.json` through a sync
  service produce a conflicted copy. Atomic writes keep each copy valid; a
  merge is not attempted. Say so in the data folder setting.
- **Network.** Once lookup exists the "entirely local" statement in the README
  needs a qualifier, the same one `extension_api.md` foresaw.
- **Scan cost.** Fingerprinting reads 128 KiB per file. Across a few thousand
  files that is one pass at idle priority; it is not a per-render cost.

## 12. Hard to retrofit — get these right in phase 1

- **Paper ids are random and permanent.** Never derived from a path or an
  identifier, both of which change.
- **Per-field provenance from the first write.** Without it the precedence
  rule cannot exist and a rescan overwrites user edits.
- **Cite key freezing** as a rule in the data layer, not a UI convention.
- **The data file's version and its location setting.** Moving a file people
  have already synced is the migration nobody wants.
- **Atomic writes.** The index can be regenerated; this file cannot.
- **`library:changed` from day one.** Multiple windows exist today.
- **Filing is copy, never move.** The one filesystem write beyond the data
  files, and the one a user cannot undo from inside SpeedyF.
