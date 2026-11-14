# Notepad

One Markdown notepad per project. It ships with SpeedyF, switched on; it
never uses the internet.

## Where it is

- **Beside the paper.** When the open document is an item in a project, a
  notepad icon sits at the right end of the toolbar and **Cmd+Shift+N**
  (Ctrl+Shift+N) opens a drawer on the right of the page; the page re-fits
  beside it. Drag its left edge to resize. It stays open as you move between
  papers. For a PDF that is in no project there is no icon and no drawer.
- **On the Projects screen.** Select a project: with no item selected, the
  right-hand column is its notepad. Selecting an item shows its details
  there, under a line back to the note (or press Escape). Drag items from
  the list onto the note to cite them. Drag the column's left edge to make
  it wider.

Both are the same note, live, in every window.

## Writing

The line you are on shows its Markdown; the rest is rendered: headings,
_emphasis_, lists, quotes, `$x^2$` and `$$ ... $$` math, links and images.
**Source** at the top (or Cmd+A twice) edits the whole note as plain text
(Escape returns).

- **@** opens a picker of the project's items; Enter inserts `@citekey`,
  which reads "[AS22]", the same alpha.bst-style label the graph shows. Click it to open the paper. Items dragged in
  from the Projects list land as `@a; @b`.
- `[@citekey, p. 12]{box=72,310,240,28}` reads "[AS22, p. 12]" and opens
  the paper at that place on page 12.
- A cite key that changes later keeps working: the item remembers the keys it
  had. A key no item has stays as you wrote it, muted.

## From the reader

- **Quote to Notepad** (palette, with text selected) adds
  `> "the text" [@key, p. 12]{box=...}`.
- **Send to Notepad** on an equation's hover button adds it at once, with a
  link to it: as LaTeX when Math to LaTeX is on and can read it from the
  page (its tags or its fonts), else as a picture. Then, quietly, Math to
  LaTeX looks for the author's own LaTeX online (the arXiv source or an
  open-access journal version); if that differs, the equation is replaced
  in place, a picture by its LaTeX (the picture's file is removed). Cmd+Z
  undoes it. An equation you have already edited is left alone.
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
