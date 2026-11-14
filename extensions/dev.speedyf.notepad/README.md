# Notepad

One Markdown notepad per project. It ships with SpeedyF, switched on; it
never uses the internet.

## Where it is

- **Beside the paper.** When the open document is an item in a project, a
  notepad icon sits at the right end of the toolbar and **Cmd+Shift+N**
  (Ctrl+Shift+N) opens a drawer on the right of the page; the page re-fits
  beside it. Drag its left edge to resize. It stays open as you move between
  papers. For a PDF that is in no project there is no icon and no drawer.
- **On the Projects screen.** Select a project and pick **Notepad** beside
  List and Graph.

Both are the same note, live, in every window.

## Writing

The line you are on shows its Markdown; the rest is rendered: headings,
_emphasis_, lists, quotes, `$x^2$` and `$$ ... $$` math, links and images.
**Source** in the header edits the whole note as plain text (Escape returns).

- **@** opens a picker of the project's items; Enter inserts `@citekey`,
  which reads "Attias 2022". Click it to open the paper.
- `[@citekey, p. 12]{box=72,310,240,28}` reads "Attias 2022, p. 12" and opens
  the paper at that place on page 12.
- A cite key that changes later keeps working: the item remembers the keys it
  had. A key no item has stays as you wrote it, muted.

## From the reader

- **Quote to Notepad** (palette, with text selected) adds
  `> "the text" [@key, p. 12]{box=...}`.
- **Send to Notepad** on an equation's hover button adds its LaTeX when
  Math to LaTeX is on and can read it, and a picture of it otherwise, with a
  link to it.
- **Send Region to Notepad** (palette): drag a rectangle around a figure;
  its picture goes into the note with a link to it.

They write to the notepad of the project you opened the paper from; for a
paper in several projects opened from elsewhere, SpeedyF asks once.

## On the Projects screen

An item a notepad cites shows "Mentioned in notepad (3)" in its details;
click it to go to the first mention.

## Where the files are

Beside your `library.json`:
`project-files/dev.speedyf.notepad/<project id>/notepad.md`, pictures in
`assets/` next to it. They are backed up with the library, travel in a
project bundle, and are kept (under `deleted/`) when a project is deleted.
Disabling Notepad hides it; the files stay.
