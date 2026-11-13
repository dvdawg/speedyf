# SpeedyF Projects - design plan

**Status:** implemented (phases 1 to 4) on 2026-11-12, with two additions the
design did not foresee: file-derived metadata is re-derived on every rescan
(sec. 2), and the reader classifies an open PDF before offering to file it (sec. 6),
because SpeedyF is a viewer first and not every PDF is a paper. The
bibliography parser of sec. 8 was not ported to Rust: it runs in the References
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
   different the moment the file is re-saved with annotations - which is why
   it comes after the identifier and before the path.
3. **Path.** The canonical path recorded on the file.

No match creates a new paper. Two files that resolve to the same paper are
both kept on it. A file that disappears leaves the paper in place with the
file marked missing: the paper stays in its projects and its `.bib`, the UI
dims it and says so, and the next file that matches by identifier or
fingerprint reattaches.

**While the app runs** (`library/watch.rs`) the folders are watched, so a
rename reattaches at once rather than at the next launch: the watcher's
removed path marks its file missing now (`missingSince` set) and the new
path is scanned and reconciled, which finds the paper by its fingerprint
and moves the row to the new path. A file moved to a Trash (`.Trash`,
`.Trashes`, any hidden folder below a root) counts as removed and is never
reattached there. A cloud placeholder (macOS `SF_DATALESS`: iCloud Drive,
Dropbox or OneDrive evicted it) is never opened, since any read downloads
it: the scan takes its name and stamp only, the text pass skips it, and it
is not missing; once downloaded it has no fingerprint yet, so the next scan
reads it properly.

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

**Merge suggestions** (`merge::duplicates`, offered through Merge in the
detail column), strongest reason first: a shared DOI or arXiv id; an arXiv
id the lookup cache says became a DOI another record holds; an identifier
citing papers print for one record that the other holds ("cited as the same
DOI by 3 papers", sec. 4, "From citing papers"); the same title
(years at most one apart); a near-identical title with the same year; and
**nearly the same reference list** (`library::versions`): both items have at
least 12 stored references, the Jaccard index of the two lists is at least
0.7 (a reference matches by the library item it resolves to, a printed DOI
or arXiv id, its title, its volume and first page, or for an untitled entry
its first author and year; an inverted index over those keys, never all
pairs), and one weak signal agrees - titles alike (similarity >= 0.5), the
same first-author family, or known years at most 2 apart. Reason: "nearly
the same 48 references". Surveys of one field (about 30 % shared, Jaccard
~0.2) and a follow-up by the same authors (about half shared, ~0.33) stay
apart. This signal is only ever a suggestion; an automatic merge still
needs one of the two rules above.

All of an item's files stay visible: the detail column lists them, and any
can be opened or made the main one (`files[0]`: the one Open, previews and
the scan's fields use), which a rescan keeps.

**The main file** (`library/mainfile.rs`). The user's choice (`primaryPath`,
set by Make main) is never overridden. Without one, the main file is chosen
again when a file attaches, on a merge, after a scan marks files missing,
and in the load sweep, by narrowing the files rule by rule (a rule applies
only when some file passes it): on disk; the published version (the file's
own first-page or XMP DOI is the item's journal DOI, not a 10.48550 one, or
its classifier reason names a publisher); the highest arXiv version (the
index entry's `arxiv_version`, read off the stamp, `arXiv:1706.03762v5
[cs.CL]`, when it names the file's own id, else off a file name that gave
the id, `2606.13614v2.pdf`; failing both, any `v3` in the name,
`paper_v3.pdf`; a file whose version is unknown is not ruled out. Index v13
rescans once, metadata only, to fill it); authors
on the first page (an anonymous review copy loses); not a browser duplicate
(`name (1).pdf`, `Copy of`, `name copy.pdf`); the newest mtime; a tie keeps
the order. The evidence is kept on each file (`traits`), refreshed by every
reconcile, so a merge or a load can rank files without the index.

**Byte-identical copies.** Files of one item with the same fingerprint, both
on disk, are one PDF in the UI: the DTO marks each copy with `duplicateOf`
(the path of the copy shown: the main file when it is in the group, else
one that is not a browser duplicate, else the oldest), the detail column
shows one row "name, 2 copies" with every path in its tooltip, Open and
Make main act on that copy, and the list's "N PDFs" counts distinct
contents. The records keep every file; nothing is deleted.

**Items gone for a week.** A file's `missingSince` is stamped when a scan
first finds it missing and cleared when it is back. An item all of whose
files have been missing for seven days is removed, by the scan's
`mark_missing` and by the load sweep (saved, and `library:changed`, once),
unless it is in a project, has user edits or a pinned key, carries merged
records or aliases, or has parked candidates. A file under `/Volumes/X`
while X is not mounted does not count as gone.

### Authors as people

`library::persons` clusters the name variants the library prints into
persons. Two names are candidates when the family names match (case,
diacritics, hyphens and leading particles folded: "van der Waals" is
"Waals", "Garcia-Lopez" is "Garcia Lopez") and the given names fit position
by position (an initial fits any name it begins; two spelled-out names must
be equal). A candidate pair on two items is merged only with evidence: the
same spelled-out first given name; a shared coauthor (compatible names); or
a citation between the two items with at least 3 shared references (the
citation pass records these as `topicLinks`). Clusters are complete-linkage:
no two names in one person are incompatible ("J. Smith" never joins "John"
and "James") and no item holds a person twice; edges are taken in a fixed
order, so clusters are the same on every machine. Persons are refreshed
before every save and kept in `library.json`'s `persons` section (its own
`version`; another is rebuilt), ids carried over by majority of members.

The best form of a person is the fullest given name seen, then the most
seen. In the detail column and the reader's Library panel an author with
other items is a quiet link; it lists the person's items in the middle
column under "Items by John Smith", with "also as J. Smith" below, until
Escape or the close button, like the graph's focus (`library_person`).
Consensus counts two forms of one person as agreeing; a brand-new item's
cite key takes the first author's family as the person is most often
printed. Existing keys never change because a person did.

### Metadata with provenance

Each field carries where it came from:

```
title, authors, year, venue, volume, issue, pages, publisher, abstract, kind
source: one of the ranks below
```

| Rank (strongest first) | Source                                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------------------------- |
| 1                      | `user`                                                                                                         |
| 2-9                    | lookups: `crossref`, `inspire`, `datacite`, `arxiv`, `semantic-scholar`, `dblp`, `openalex`, `open-library`    |
| 10                     | `xmp`                                                                                                          |
| 11                     | `identifier` (the year an arXiv id or DOI carries)                                                             |
| 12                     | `citations`: what at least two independent citing items in the library agree on, identifiers included (sec. 4) |
| 13                     | `pdf-info`                                                                                                     |
| 14                     | `first-page`                                                                                                   |
| 15                     | `filename`                                                                                                     |

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
  library.json          # papers, projects, memberships - the user's work
  backups/              # library-YYYYMMDD-HHMMSS.json.gz, gzip'd snapshots
<app data dir>/
  library-index.json    # the scan cache, as today - disposable
  library-root.json     # the folders, as today
  lookup-cache.json     # raw lookup responses by identifier - disposable
  text/                 # full-text sidecars, as today
```

The split is between what can be regenerated and what cannot. The index and
the sidecars are rebuilt from the folders; `library.json` is the only file
that holds something the user did, and the only one that moves when they
choose a different data folder. Default: the app data dir. **Settings -> Library
data folder** moves it, copying the file and then reading from the new place.

**Live folder watching** (`library/watch.rs`, the `notify` crate: FSEvents
on macOS, inotify on Linux, ReadDirectoryChangesW on Windows). Every root is
watched recursively from launch, re-armed when roots change and when an
unmounted volume comes back (checked every minute; a root under an
unmounted `/Volumes/X` is not watched). Events only name paths: a path is
looked at once it has been quiet for 1.5 s, and a file is taken once its
size and mtime agree across two looks 0.5 s apart, so a download still
arriving is not read. Non-PDFs, hidden files and folders, our own data
folders and partial downloads (`.crdownload`, `.part`, Safari's
`.download` bundles) are ignored; the full scan skips the same. A gone
path marks its files missing; a settled PDF is scanned at idle (no OCR,
nothing copied) and reconciled (`LibraryWatchNext`, one file per job). A
burst is one batch, saved and announced (`library:changed`) at most once
a second while it drains and once at the end; text, cites and OCR follow
through the usual idle chain. A download shows up about 2-3 s after it
finishes. **Safety net:** every 10 minutes, unless a scan is running, a
stat-only pass compares the folders with the index and queues only what is
new, changed or gone - for coalesced FSEvents, network volumes, sleep,
and roots the watcher could not arm (logged once, never an error the user
sees).

Written atomically: serialize to a temp file in the same directory, then
rename over the old one. The existing index is written with a plain
`fs::write`, which is acceptable for a cache and not for this file.

**Backups** (`library/backup.rs`). Full snapshots, gzip-compressed (a
thousand items is tens of kilobytes), not diffs, so each restores on its
own. A save writes one at most once an hour, and only when the content
differs from the last backup (an FNV-1a hash, primed from the newest file
at the first save). Rotation keeps every backup of the last 24 hours, the
newest of each day for 14 days and of each week for 8 weeks, and deletes
the rest. A failed backup never fails the save. `library_backups` lists
them (when, size, items, projects; unreadable ones are left out) and
`library_restore_backup { file }` takes only a backup's file name, refuses
one that does not parse or was written by a newer build, backs up the
current state first, then replaces the records in memory and on disk.
Settings shows "Last backup ..." and a Restore... list with an in-place
confirm.

`library.json`, version 1:

```json
{
  "version": 1,
  "papers": [
    {
      "id": "7c1e0b5a-...",
      "files": [
        {
          "path": "/Users/me/Papers/noneq/jarzynski1997.pdf",
          "size": 310221,
          "mtimeMs": 1699000000000,
          "fingerprint": "f1a2...",
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
  "persons": { "version": 1, "topicLinks": [], "people": [] },
  "projects": [
    {
      "id": "3b9d...",
      "name": "Noneq stat mech",
      "order": 0,
      "paperIds": ["7c1e0b5a-..."],
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
  publisher families (Elsevier `j.journal.2020.`, Springer Nature `s...-020-`,
  IEEE `.2019.`, Wiley) carries its year; and an Elsevier PII such as
  `0022-0000(88)90028-1`, which old scans print instead of a DOI, carries
  its year in the parentheses and converts to the DOI `10.1016/<PII>`, which
  then unlocks a Crossref lookup. These land at the `identifier` rank: above
  the Info dictionary, below XMP and every lookup.
- **The verdict.** The classifier of sec. 6 runs on every file during the scan,
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
  images, too few letters, too many 1-2 character tokens, too many
  vowel-less words) and its image objects for one covering >= 60 % of the
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
  Cost: ~0.4 s a page on Apple silicon, and the first Vision call of a
  session loads its model and can take tens of seconds - so it runs on its
  own thread (`engine::ocr_thread`, utility QoS), never on the PDFium
  worker: the worker renders the page (PDFium stays there), hands the PNG
  over a bounded channel, one page in flight per document, and goes on
  with its queue. The OCR thread writes the reading to the cache at once
  (so a quit loses nothing already read) and sends it back as an idle
  `OcrDone` job, which resumes whatever waited: the scan scans that file
  again with the page cached; the text pass picks its document up where it
  stopped - pages already extracted, the six-page budget as spent. Only
  the idle library jobs ask, never a scan the user is waiting for (an
  inbox copy reads the cache only and is scanned again at idle,
  `ocr_deferred`). Elsewhere it compiles to nothing: Windows and Linux keep
  the text layer. Search in an open document uses the cache too, never
  running OCR itself: a page whose layer needs OCR and has a cached
  reading is indexed from it, its hits highlighted from Vision's line boxes
  (a hit's share of its line), since its offsets are not PDFium
  characters.

The first-line title heuristic and the filename patterns stay as the floor.

### From citing papers

Offline, from the reference lists the provider already read
(`library::consensus`, `library::witness`). Every reference entry of a
library item that resolves to another item (by identifier or title) is an
independent witness of the cited item's authors (family names and
initials), year, title, venue, volume and first page; the engine reads the
entry in the common styles (APS, IEEE, Elsevier, Springer, Nature, APA,
Vancouver, ACM) and leaves a field empty rather than guess. A field is taken
when **at least 2 independent citing items agree and they are a strict
majority** of the citing items that say anything about it. An "et al." list
supports the full list it opens. Citing items that are versions of one
another (a version pair above, or one title) count once, and a version of
the item never vouches for it. Venues match across abbreviation ("Phys. Rev.
Lett." is "Physical Review Letters", not "Phys. Rev. A" vs "Applied").

The value lands at rank `citations`: above the first page, the file name and
the Info dictionary, below the identifier, XMP, lookups and the user, via the
same `offer`, so user edits are never touched. A value the record already
holds that agrees stays under its own source.

**Identifiers.** An item with no DOI (or arXiv id) takes the one its citing
papers print when at least 2 independent citing items name it by title and
print that identifier in the same entry, no citing item prints another, and
each entry's first-author family and year (when both sides have them, years
at most 1 apart) fit the item. An arXiv DOI counts as the arXiv id. It is
held with source `citations`: a scan of the file that finds an identifier,
XMP, a lookup and the user each replace it; a scan that finds none leaves it;
it goes when the consensus does. Once held it is an identifier like any
other (resolution, merging); cite keys are untouched, frozen or not. An
identifier another item already holds is never taken: the two are offered as
duplicates, "cited as the same DOI by 3 papers". The DOI field shows "from 3
citing papers". The record keeps which items
vouched (`citationFacts.vouched`); the detail column's provenance tag reads
"from 3 citing papers". When the consensus goes, a value from it is dropped
and the primary file reconciled again. The pass runs at idle a few seconds
after the provider stores a list (coalesced) and when the provider changes.

### Online, opt-in

Off until **Settings -> Look up paper metadata online** is turned on. This is
the first network call SpeedyF makes, so it meets the bar `extension_api.md`
set for extensions wanting the same: a plain statement of what leaves the
machine and an indicator in the status bar while a lookup runs.

What leaves the machine, once the setting is on: DOIs, arXiv ids, ISBNs, and
for a paper that has none of them its first-page title. Never file contents.
Only records the scan (or the user) takes for papers are looked up at all.

| Have     | Ask, in order                                                                                                                                                                                                    |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| DOI      | Crossref; DataCite if Crossref has no record; OpenAlex only when venue, volume or pages are still missing; Semantic Scholar (abstract, and the references for sec. 8); INSPIRE when the paper looks like physics |
| arXiv id | arXiv (which may yield the DOI, then the row above); INSPIRE when physics; Semantic Scholar; OpenAlex only when the venue is still missing                                                                       |
| ISBN     | Open Library                                                                                                                                                                                                     |
| none     | Title search on OpenAlex, Crossref, arXiv, Semantic Scholar, and DBLP when the paper looks like computer science; the answers are merged                                                                         |

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
User-Agent in exchange for better service; **Settings -> Contact for lookups**
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
+- projects ------+- papers -----------------------------+- paper ---------------+
| All papers  412 | All papers          [Home | Projects] | +---------------+     |
| Not filed    37 | [search...]                          | | first page    |     |
| --------------- | v Noneq stat mech  23                | |               |     |
| Noneq stat mech |    Nonequilibrium equality...  1997  | +---------------+     |
| Active matter   |    Entropy production...       1999  | Jarzynski - 1997      |
| Thesis          | > Active matter  41                  | Physical Review Lett. |
|                 | v Thesis  117                        | doi 10.1103/...       |
| + New project   |    ...                               | key jarzynski1997...  |
|                 |                                      | [x] Noneq  [ ] Active |
|                 |                                      | [Open] [Copy key]     |
+-----------------+--------------------------------------+-----------------------+
```

**Left column.** _All papers_, _Not filed_ and _Other PDFs_ with counts, a
rule, then the projects in their stored order, then _New project_. _Other
PDFs_ holds what the scan does not take for a paper; a paper misjudged there
is one click from being one, and anything may still be dragged into a project. Projects are renamed in
place and reordered by drag. Deleting a project removes memberships and
nothing else; files and papers are untouched. Dropping paper rows onto a
project adds them.

**Middle column.** Header: the selection's name and count; for a project also
its synced `.bib` path or "no .bib", **Export .bib...**, and the incomplete-entry
count. Then the search field, then the list.

- Under _All papers_ the list is grouped by project, each group collapsible,
  with _Not filed_ last. A paper in three projects appears three times; that
  is the truth of the data and the point of the overview.
- Under a project the list is flat.
- A row is the title, then authors, year and venue dimmed. A missing file dims
  the whole row. The cite key appears at the right on hover.
- Search matches title, authors, year and key as you type. Below the matches a
  single row, _Search the full text of every paper for "..."_, runs the existing
  trigram search, because "which paper said this" is a different question and
  already has an answer.
- Selecting a row fills the right column; Enter or double-click opens the
  paper in a tab, through `openInNewTabOrFocus`, exactly as the Library panel
  does today.

**New.** A row under _All items_, with a muted count, present only while
there are untriaged new items: items (`is_paper`) whose record was created
(`addedAt`) in the last 7 days and after the library first loaded with this
build (`review.since`, so items already there never count). Selecting it
lists them flat, newest first, whatever the sort. On the focused row a digit
1-9 files the item into the Nth project of the left column (the digits show,
muted, at the end of the project rows only in this scope), D or Backspace
marks it done, Enter opens; a quiet _Done_ per row and _All done_ in the
header do the same with the mouse, and a drag onto a project files and
triages. When the last one is triaged the row goes and the view falls back
to _All items_. Palette: _Show New Items_.

**Health.** A row below _Other PDFs_, with a muted count, present only while
there are findings (`library_health`, `library/health.rs`). The middle
column lists them under group headers, each row the item, one muted reason
and text buttons on the right; a row selects the item, double-click opens it.
Groups, each only when non-empty: _Likely duplicates or versions_ (the
duplicate groups, with version pairs; _Merge..._ opens the merge
confirmation), _Year may be wrong_ (the year hint; _Use 2021_ writes the
floor as a user edit), _Missing files_ (every file gone: for how long, and
when it leaves the library; a drive not connected says so), _Weak metadata_
(no title beyond the file name, no authors, or incomplete for BibTeX in a
project that syncs a `.bib`; _Look up_ only when lookups are on), _Title
mismatch_ (a confident first-page title sharing too little with a recorded
title the user did not type: not one inside the other and similarity below
0.3; _Use page title_). Every finding has _Show_ and _Ignore_; Ignore is
kept per item and kind and the finding never returns. Offline, no file read:
the duplicate groups are computed once per `library:changed` (a generation
counter) and shared with the duplicate line. Palette: _Show Library Health_.

`library.json` carries both decisions in one versioned section:
`"review": { "version": 1, "since", "reviewed": [ids], "ignored": [{ "paperId", "kind" }] }`;
reviewed ids past the 7-day window are dropped as it is rewritten.

**Sort.** A quiet _Sort: Default_ text button in the list's header line
cycles Default (the engine's order) | Year (newest first, undated last) |
Title | Cited here, kept across sessions (`speedyf-list-sort-v1`). _Cited
here_ counts the items of the scope that cite each one (in a project, its
members; under _All items_, every item), from the graph's edges in one pass
(`library_cited_here`), most first, ties by year then title; only then does
the meta line say "cited by 5 here". Under _All items_ it sorts within each
group.

**Export bundle (.zip)...** (the project's menu, beside Export .bib..., and the
palette's _Export Project Bundle..._) writes one zip through the save
dialog: `<project name>.bib`, rendered as Export .bib renders it and its
keys pinned the same way, and `pdfs/<citekey>.pdf`, a byte copy of each
item's main file that is on disk (stored, not recompressed; a key made safe
as a file name that would collide gets `-2`). Nothing else goes in. Items
whose main file is missing are skipped and named only in the screen's
inline result ("Exported 23 PDFs and the .bib; 2 missing: key1, key2").
The zip is streamed to a temp file in the destination folder and renamed
into place, so a failed export leaves nothing; library files are only read,
and the PDFs are copied outside the store lock (`library/bundle.rs`,
`library_export_bundle`).

**Opening in a new window.** Cmd+Enter (Ctrl+Enter) in quick open, Cmd- or
Alt-click on a file's Open in the detail column, and Cmd+Enter or Cmd-click
on the palette's _Open <file>_ rows open the file in a window of its own
(`open_in_new_window`: the new window takes the path as its opening tab,
the way a restored window takes its session). A window that already has
the file, or one still coming up for it, is raised instead.

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
the screen says _All items_, the detail says "Looks like a thesis: ...", and
the reader's panel is called _Library_. Internal names keep "paper".

**Keyboard.** Up and down move within a column, left and right move between
columns, Enter opens. The copy-cite-key chord is chosen at implementation
against `reservedChords()` so it collides with nothing SpeedyF already binds.

**Empty states.** No library folders: the screen says so and offers _Add
folder..._. No projects: the left column holds only the two fixed rows and _New
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
an abstract with sections. A paper gets "This looks like a paper (arXiv:... on
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
_Copy BibTeX_, _Add to Project..._, _Look Up Metadata_, _Switch Launch Screen_.
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
| not enough to say   | `@misc{key, title, note = {...}}` preceded by `% incomplete: no author, no year` |

Authors are written `Family, Given and Family, Given`. Titles are wrapped
`{{...}}` to protect case. The characters `& % $ # _ { } ~ ^ \` are escaped;
everything else is written as UTF-8, which every modern TeX reads.

**Options** (Settings, Advanced, BibTeX; closed by default). Kept in
`library.json` as `bibtexOptions` (absent: the defaults, which write
exactly the output above), so the clipboard, exports and synced files use
them on every machine; changing one saves, which rewrites every synced
`.bib` whose bytes change. Include abstract (off); include URL (off: the
doi.org link, else `https://arxiv.org/abs/<id>`); include file path (off:
the main file on disk); title case protection (on: `{{...}}`, off: `{...}`);
abbreviate journal names (off: a built-in table of common physics, math
and CS journals, matched ignoring case, spacing and a leading "The"; only
the `journal` field, unknown names left alone).

**Export .bib...** opens a save dialog and writes the project's entries once.
**Sync to a .bib...** records a path on the project; from then on every change
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
fallbacks). The engine keeps per entry what matching needs - the compact text,
the year, the explicit DOIs and arXiv ids and the DOI an APS journal reference
spells out - in a refs sidecar stamped with the provider and its version, and
matches entries to library titles. Disable References and no refs sidecar is
read: DOI, arXiv and APS-journal edges remain (the engine finds those in the
whole text itself), title-only edges go.

**Related items.** Below _Cited by_ in the Library panel, and after the
projects in the detail column, a _Related_ list: at most five other items tied
to this one by what both cite (bibliographic coupling) and by the items citing
both (co-citation), each row with why ("shares 6 references, cited together
by 3 items") and opening that item like a cites row. Works are the keys
`suggest` already uses (a resolved item, else DOI, arXiv id or exact reference
text), folded so one work read twice counts once. Score: shared works over
`sqrt(|works a| * |works b|)` plus co-citations over
`sqrt(|citers a| * |citers b|)` (Salton), so a review with a huge
bibliography or a much-cited classic does not win on size. An item needs at
least 2 shared works or 2 co-citations; an item one cites or is cited by
must qualify the same way and then counts at half its score, since Cites and
Cited by already show it. Never listed: the item, anything sharing one of its
identifiers (aliases, unmerged twins) or a file fingerprint. Ties break by
id. Computed per request from the same snapshots as citation links, offline;
about 0.1 s on 3000 items with 80 citations each (release). No list, no
heading.

**Cited with these.** For a project, `library_cocited_works`
(`library/cocitation.rs`) names the works the library tends to cite
alongside the members but that are not in the project: library items
outside it, and missing works keyed as the missing-works list keys them.
For a candidate X, its co-citers are the citing works whose bibliography
holds X and at least one member; records that are versions of one another
(a shared DOI or arXiv id, aliases included, or the same title under the
same first author) count as one citing work. Score: the sum over
co-citers of the members each cites, divided by the square root of how
many citing works in the library cite X, so a textbook everyone cites does
not dominate. Shown with at least 2 co-citers and at least 2 members (or
the user's higher "with >= N of these"); members, versions of members and
the missing works the "cited by these" list shows at its threshold are
left out, so the two lists never repeat. Ties go by members, co-citers,
key; at most 20. One pass builds an inverted index from cited work to
citing works; only members' co-citers are visited.

The user sees three things, all quiet. The project header says "3 works
are often cited with this project" when there are any; clicking it opens
the missing-works list on its second mode. That list has a two-way switch,
_Cited by these_ | _Cited with these_; a cited-with row is the title or the
printed reference with "cited with 4 of these", and a library item has
**Add** (the membership command). In the graph, cited-with works are ghost
nodes in the missing works' style, placed near the members they are cited
with and joined to them by faint dashed lines; they take no part in the
layout, so nothing moves. Clicking one picks it into the detail column,
where a library item can be added too. Everything rereads on
`library:changed`.

**View.** The graph's controls carry one _View_ button (beside Reset
layout) that opens a small popover: Edges (All | Selected: only the
hovered or selected item's), Labels (Many: as before | Few: the most cited
only | Hover: none until hovered or picked), Unlinked items (Hide: counted
in the chip, as before | Show: drawn in rows under the layout, which does
not rerun), Missing works with its "cited by >= N" stepper, and, in a
project, Cited with with its "with >= N of these" stepper. Both kinds of
ghost may be on at once. The defaults draw the graph as before; the
choices are kept across sessions in their own localStorage key. Escape or
a press outside closes it.

**Dating.** A paper is at least as new as what it cites. Its `yearFloor` is
the latest year its reference list gives, ignoring years in the future and a
single newest year more than 3 above the next, and supported by the
references at or after it: one with an identifier or two without (else the
next older year is tried). An item with no year shows the floor where the
year goes, as "2019+", sorts as that year, and the detail column says "from
its references"; it is never a BibTeX year. An item whose recorded year is
more than 1 below the floor gets a quiet hint beside its year, "cites work
from 2021; year may be wrong". The graph's time pull uses the floor for an
item without a year.

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
library_papers { filter }            -> { papers, total }
library_paper { id }                 -> paper
library_paper_for_path { path }      -> paper | null
library_update_paper { id, patch }   -> paper
library_projects                     -> [project]
library_create_project { name }      -> project
library_rename_project { id, name }
library_delete_project { id }
library_reorder_projects { ids }
library_set_membership { paperId, projectId, member }
library_preview { paperId }          -> resolved (doc id + preview spec)
library_file_into_inbox { path }     -> newPath
library_bibtex { paperIds }          -> string
library_export_bib { projectId, path }
library_set_project_bib { projectId, path | null }
library_lookup { paperId, mode }     -> applied | [candidate]        (phase 2)
library_related_items { paperId }    -> [{ row, score, sharedRefs, coCited, reason }]  (phase 4)
library_data_dir / library_move_data { dir }
library_health                       -> [{ kind, label, findings }]
library_health_ignore { paperId, kind }
library_new_items { query }          -> [row], newest first
library_mark_reviewed { paperIds }
library_cited_here { filter }        -> { id: count }
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
  mod.rs        scan gains Info dict + fingerprint; reconcile files -> papers
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

## Phase 1 - projects

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

## Phase 2 - lookup

**Ships:** the opt-in setting and status indicator; Crossref and arXiv by
identifier, per paper and per project; OpenAlex by title on request with a
candidate picker; the response cache; `reqwest` and the XML dependency.

**Done when:** a project of identifier-bearing papers looks up in one action
and every field shows its source; turning the setting off makes no request.

## Phase 3 - `.bib`

**Ships:** the serializer and its tests; Copy BibTeX; Export .bib...; synced
paths with debounced atomic rewrites; key freezing on first write; the
incomplete count in the project header.

**Done when:** a synced `.bib` compiles under `bibtex` with the project's
papers, and a key that has been written never changes without the user
unpinning it.

## Phase 4 - graph

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

## 12. Hard to retrofit - get these right in phase 1

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
