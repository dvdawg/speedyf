# References

Every work the open paper cites, where it is cited, and which ones are already
in your library. Click any row to jump to the first page it appears on.

Load it from **Home → Extensions**, open a paper, then pick **References** in
the sidebar.

## Settings

- **Order** — by first appearance, most cited, or title.
- **Separate what you already have** — split into works in your library and
  works not in it.

## What it demonstrates

A worked example of most of the v2 API:

| Feature                                | Used for                                              |
| -------------------------------------- | ----------------------------------------------------- |
| `panels` + `ui.render`                 | The sidebar list, drawn from a description            |
| `citations.list` / `citations.resolve` | Finding cited works and matching them to your library |
| `navigate.toPage`                      | Jumping to where a work is cited                      |
| `settings`                             | Order and grouping, read fresh on each render         |
| `storage`                              | Caching the scan so reopening is instant              |

Three things worth copying:

- **It renders progress while scanning.** A long paper takes a second or two,
  and a panel that stays blank until it finishes looks broken.
- **It answers every invoke promptly**, doing slow work between renders rather
  than inside one, so it never trips the five-second timeout.
- **It caches per document**, so a rescan is something you ask for rather than
  something that happens every time you glance at the panel.

## A note on empty results

References are found from links and identifiers in the document. A paper whose
bibliography carries neither will show nothing.
