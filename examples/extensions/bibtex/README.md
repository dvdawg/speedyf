# BibTeX Export — SpeedyF v2 read-API reference

Reads the open document: every DOI and arXiv citation in it, which of them are
already in your library, and a summary of its outline and figures.

Load it from **Home → Extensions → Load folder…**, open a paper, then run any of
its commands from the command palette. Results appear in this extension's log in
the Extensions panel.

| Command                    | Uses                                                        |
| -------------------------- | ----------------------------------------------------------- |
| BibTeX: List References    | `document.info`, `citations.list`                           |
| BibTeX: Library Coverage   | the above plus `citations.resolve`                          |
| BibTeX: Summarise Document | `document.outline`, `document.figures`, `document.pageText` |

## What the API deliberately does not give you

- **No file paths.** `document.info` reports a name, never a path. Reading the
  filesystem is a different permission and must not be spellable as this one.
- **No engine ids.** You address the _active_ document; you cannot name another.
- **No geometry.** `document.pageText` returns text, not positioned runs.
  Coordinates are the hardest thing to keep stable across an engine change, and
  nothing needed them yet.

Ask for an API you did not declare a permission for and the call is rejected
with a message naming the permission — the same way a wrong command id is.
