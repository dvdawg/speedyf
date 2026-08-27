# References

Every work the open paper cites — its title, its authors, the pages it is
actually cited on, and whether it is already in your library. Opening a row
lists every place the work is cited, each with the line it appears in, nested
under the reference they belong to. Picking one goes to that line rather than
to the top of the page it is on, and marks it for a couple of seconds so you
can see where to look without hunting for the `[12]` yourself.

Load it from **Home → Extensions**, open a paper, then pick **References** in
the sidebar.

## Settings

- **Order** — by first citation, most cited, or title.
- **Separate what you already have** — split into works in your library and
  works not in it.

## How it reads a bibliography

A typeset paper already knows which entry each `[12]` refers to, because the
`[12]` is a link and it points at a place on the references page. Following
those destinations answers both halves of the problem at once:

- **Forwards**, every destination that lands on an entry is a page the work is
  cited on. That is a different thing from the page it is _listed_ on, which is
  the same page for every entry and the only answer available without them.
- **Downwards**, sorted by position, the destinations cut the reference list
  into entries — and the words inside one entry are its authors, its title and
  its year.

So a row reads

> **Effect of batch learning in multilayer neural networks**
> Kenji Fukumizu · 1998 · cited on pages 3, 5, 9

rather than

> **api.semanticscholar.org/CorpusID:605683**
> link · page 14

None of this guesses at the prose. Where an entry begins is the document's own
answer, not a judgement about where a citation seems to start.

## What it demonstrates

A worked example of most of the v2 API:

| Feature                                | Used for                                            |
| -------------------------------------- | --------------------------------------------------- |
| `panels` + `ui.render`                 | The sidebar list, drawn from a description          |
| `document.jumps`                       | Which entry each citation refers to, and from where |
| `document.pageRuns`                    | Reading titles and authors out of the entries       |
| `document.links` / `citations.resolve` | Identifiers, and matching them to your library      |
| `navigate.toPage`                      | Walking the places a work is cited                  |
| `settings`                             | Order and grouping, read fresh on each render       |
| `storage`                              | Caching the scan so reopening is instant            |

Three things worth copying:

- **It renders progress while scanning.** A long paper takes a second or two,
  and a panel that stays blank until it finishes looks broken.
- **It answers every invoke promptly**, doing slow work between renders rather
  than inside one, so it never trips the five-second timeout.
- **It caches per document**, so a rescan is something you ask for rather than
  something that happens every time you glance at the panel.

## What it cannot do

- **A paper with no internal links** — an older scan, or most things exported
  from a word processor — has no destinations to follow. The panel falls back
  to listing the addresses printed in the bibliography, and says "listed on"
  rather than "cited on", because in that case nothing in the file says where a
  work is cited. A paper whose bibliography carries no links or identifiers
  either will show nothing at all.
- **A reference nothing cites** is invisible. An entry is found by the
  citations pointing at it, so one that is listed but never cited — a
  `\nocite`, usually — has nothing pointing at it and is read as part of the
  entry above.
- **A word broken across a line** cannot always be put back correctly.
  "sensi-tivity" is closed up to "sensitivity", which is right; a lowercase
  compound that breaks at its own hyphen closes up too, so "two-layer" becomes
  "twolayer". A capitalised continuation is kept hyphenated, which is what
  saves "Vapnik-Chervonenkis".
