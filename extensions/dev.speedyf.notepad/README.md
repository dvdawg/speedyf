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

The line you are on shows its Markdown (a quote, all of its lines); the
rest is rendered: headings, _emphasis_, lists, quotes (consecutive `>` lines
are one quote), `$x^2$` and `$$ ... $$` math, links and images.
**Source** at the top (or Cmd+A twice) edits the whole note as plain text
(Escape returns). Once the note has two headings, **Outline** beside it
lists them (three levels deep); the arrows and Enter, or a click, go to one
with the caret at its start, and Escape closes the list.

- **@** opens a picker of the project's items; Enter inserts `@citekey`,
  which reads "[AS22]", the same alpha.bst-style label the graph shows. Click it to open the paper. Items dragged in
  from the Projects list land as `@a; @b`.
- `[@citekey, p. 12]{box=72,310,240,28}` reads "[AS22, p. 12]" and opens
  the paper at that place on page 12.
- Paste a DOI or an arXiv id (bare, `doi:...`, `arXiv:2401.01234v2`, or a
  doi.org or arxiv.org link) and it becomes `@citekey` when an item in your
  library has it; otherwise it pastes as it is. In a longer paste only the
  identifiers an item has are replaced. Cmd+Z takes the paste back.
- A cite key that changes later keeps working: the item remembers the keys it
  had. A key no item has stays as you wrote it, muted.

## From the reader

- **Quote to Notepad**: select text on the page and click the small
  **Quote** button that appears past the end of the selection, or press
  **Cmd+Shift+E** (Ctrl+Shift+E), or use the palette. It adds the passage as
  one block quote that keeps its paragraphs: lines wrapped on the page are
  rejoined (a word hyphenated across a line end is put back together), a
  bare `>` line separates paragraphs, and the link to the page is on the
  last line:

  ```
  > First paragraph ...
  >
  > Second paragraph ... [@key, p. 12]{box=...}
  ```

  Markdown characters in the text are escaped, so it reads as written. A
  selection running onto the next page links to where it begins. Long
  passages are quoted whole.

- **Send to Notepad** on an equation's hover button shows where it will go
  at once - a soft placeholder with its caption ("Equation (3), [ZD24a,
  p. 4]") - then fills it in: as LaTeX when Math to LaTeX is on and can read
  it from the page (its tags or its fonts), else as a picture. Then, quietly,
  Math to LaTeX looks for the author's own LaTeX online (the arXiv source or
  an open-access journal version), with a dim "checking arXiv source" line
  under the equation meanwhile; if that differs, the equation is replaced in
  place, a picture by its LaTeX (the picture's file is removed). Cmd+Z
  undoes it. An equation you edit meanwhile is left alone, and the line
  goes.
- **Send Region to Notepad** (palette): drag a rectangle around a figure; a
  placeholder of its shape appears, then its picture with a link to it.

If the note is not showing, the drawer opens on it as you send; on the
Projects screen the note just scrolls to it. The placeholders are never
written to the file, and go by themselves if something fails.

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
