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
- **Cite a statement.** After picking an item from `@` (or typing
  `@citekey,`), the picker lists that paper's theorems, lemmas, definitions
  and numbered equations ("Reading statements..." while it reads a paper
  that is not open). Type to narrow ("prop 2.2", a word of the name); Enter
  writes `[@citekey, Prop. 2.2]{box=... page=3}`, which reads "[RZ26, Prop.
  2.2]" and opens the paper at the statement. Escape keeps plain `@citekey`.
- Citations read like the links in a PDF: the label in the link colour,
  underlined when you point at it, and a preview of what it points to (the
  place on the page, or the paper's title).
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

  When Math to LaTeX is on, a passage with math in it keeps its math: read
  from the page's fonts, each formula in the line becomes `$...$`
  (superscripts, subscripts, norms, set symbols and Greek as LaTeX) and a
  displayed equation in the selection its own `$$ ... $$` line in the
  quote, its number as `\tag{3}`. The quote draws them typeset. Then,
  quietly, Math to LaTeX looks for the passage in the paper's arXiv source,
  with a dim "checking arXiv source" line under the quote meanwhile; when it
  finds it for certain, the quote is replaced in place by the author's own
  text and LaTeX (citations as the page printed them). Cmd+Z undoes it; a
  quote you edit meanwhile is left alone. With Math to LaTeX off, or a
  passage with no math, the quote is the plain text, as above.

- **Statements.** A quote that takes in a theorem, lemma, proposition,
  definition (any of the paper's labelled statements) - or starts on its
  label's line, or runs from inside it to its end - becomes a statement
  block, set as the paper sets it: the label bold, its name upright, the
  body italic (upright for a definition, remark or example), with the page
  link small under it. It is stored as a quote opening with the bold label,
  which any Markdown reader shows the same way:

  ```
  > **Proposition 2.2** (Moments of two block overlaps). Let $P$ be ...
  >
  > [@key, p. 2]{box=...}
  ```

  The label, number and name come from the reader even when the selection
  missed the label. Text selected after (or before) the statement is quoted
  on its own beside it. Pointing at a statement's label on the page shows a
  small **Send to Notepad** button that sends the whole statement, down to
  its end before "Proof".

- **Citations in a quote** ("[Sim15]", "[Han16, Lar23]", "[12]", "[3-5]")
  are read through the quoted paper's reference list: one an item in your
  library is becomes `[@citekey]` (several `[@a; @b]`); one that is not
  becomes a footnote marker naming the note's entry for that work, with
  the label the paper printed, `[^simon2015spectral]{label=Sim15}`. It
  reads "[Sim15]", dim, and pointing at it shows the reference with its
  DOI or arXiv link. A bracket that mixes both, "[Han16, Lar23, AAHLZ24]",
  stays one bracket in that order:
  `[@han2016; @aahlz2024][^larsen2023blocks]{label=Lar23 at=1}`.
- **One References section** at the end of the note holds every entry
  once, as Pandoc footnotes under `## References`. Two quotes citing the
  same work share its entry, however each paper prints it (the same DOI or
  arXiv id, else the same title, first author and year); different works
  never merge. It shows as one quiet line, "References (12)"; open it to
  see the entries, dim the unused ones and **Remove unused**, or, when
  some of the works are now in your library, link them as citations.
  Entries an earlier version wrote under each quote move there on the next
  quote, or with **Move to References**.
- **Cross-references in a quote** ("Theorem 2.1", "Eq. (3)", "Section 2",
  "Fig. 1") that the reader resolved become links into the paper,
  `[Theorem 2.1](@key#p3){box=...}`: they read as the words, preview the
  target on hover and open the paper there. Others stay text.

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

## Export

**Export** at the top of the note, or the palette (on the Projects screen
with a project open, or in the reader for a paper in a project):

- **Export Notepad as Markdown...** saves the note where you choose (the
  project's name by default). Its pictures are copied into a
  `<name>-assets/` folder beside the file, and the links point there.
  Citations stay Pandoc's `@key`; a page link `[@key, p. 12]{box=...}`
  becomes `[@key, p. 12]` (and a statement's `[@key, Prop. 2.2]`), a
  cross-reference its words, and a quoted reference stays a footnote
  (the References heading goes; Pandoc gathers footnotes itself).
- **Copy Notepad as LaTeX** puts the note on the clipboard as LaTeX:
  headings as `\section`, `\subsection` and `\subsubsection`, emphasis as
  `\emph` and `\textbf`, lists as `itemize` and `enumerate`, quotes as
  `quote`, math as written (`$$ ... $$` as `\[ ... \]`), `@a; @b` as
  `\cite{a,b}` and `[@key, p. 12]` as `\cite[p.~12]{key}` (with the key the
  item has now; `[@key, Prop. 2.2]` as `\cite[Prop.~2.2]{key}`), a statement
  as a `quote` with its label in `\textbf` and its body `\itshape`, a
  cross-reference as its words, a quoted reference as `[Sim15]\footnote{...}`
  (after the `\cite` of the items in its bracket),
  and each picture as a `% figure: <path>` comment. LaTeX's
  special characters in the prose are escaped; nothing in math is.

## On the Projects screen

An item a notepad cites shows "Mentioned in notepad (3)" in its details;
click it to go to the first mention.

## Where the files are

Beside your `library.json`:
`project-files/dev.speedyf.notepad/<project id>/notepad.md`, pictures in
`assets/` next to it. They are backed up with the library, travel in a
project bundle, and are kept (under `deleted/`) when a project is deleted.
Disabling Notepad hides it; the files stay.
