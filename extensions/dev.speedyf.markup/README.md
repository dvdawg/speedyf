# Markup

Referee comments on someone else's paper. Read the draft a colleague sent,
select what you want to say something about, and write; SpeedyF numbers,
labels and groups every comment, and **Export PDF...** writes one PDF to
send back. It ships with SpeedyF, switched on; it never uses the internet,
and nothing in it carries your name.

It works on any PDF you open, in your library or not.

## Reviewing

- **Start**: the pen icon at the right end of the toolbar, **Cmd+Shift+M**
  (Ctrl+Shift+M), or **Markup: Start Reviewing** in the palette. The review
  drawer opens on the right (it takes the place of the Notepad's drawer, at
  the same width) and the page shows the comments. The same key stops.
- **Comment**: select text and press **C**, or click **Comment** beside the
  Quote button. The words get a soft wash in the comment's colour and a
  card opens in the drawer, at its place in reading order. **Enter** keeps
  it, **Shift+Enter** starts a new line, **Escape** lets it go.
- **Typo or wording**: select the words and press **T** (or **Fix**), type
  what should be there, Enter. The words are struck through and the fix is
  shown after them. Enter on an empty box suggests deleting the words.
- **Region**: press **R** (or the dashed box in the drawer's head) and drag
  a box round a figure, a diagram or an equation.

In a comment:

- `/` at the start lists the **QuickMarks** - gap ("Gap in the argument"),
  justify, hypothesis, cite, related, define, unclear, reorder, motivate,
  typo. One pick writes its words and sets the kind; keep typing after it.
- `@` lists the items in your library; Enter writes `[@citekey]`, which
  reads "[Jar97]", the same label the Notepad and the graph use. The items
  cited are listed under References in the export.
- `$x^2$` is math, typeset live under the box. `**bold**`, `*italic*`,
  `` `code` `` and `- ` lists work too.

## Kinds

Every comment has one of four kinds, shown as a small chip on its card and
as its colour: **Proof**, **Exposition** (the default), **Literature** and
**Typo**. A QuickMark sets it; click the chip to change it. There is no
severity: the Overall paragraph is where you say what matters most.

## Labels and numbers

Nothing here is stored; it is worked out as you go:

- **Numbers** follow reading order: page, then down the page, then across.
  They change as you add comments above others.
- **Labels** say where a comment is: "p. 5 . Sec. 2.1 . Lemma 2.3 . eq.
  (4)". The section comes from the PDF's outline, or the one SpeedyF
  generates from its headings; the statement from the theorems, lemmas and
  definitions SpeedyF finds in the paper, and "proof of Theorem 3.1" for a
  place inside the proof that follows one; the equation when the words sit
  in a numbered display.

## The drawer

- **Overall** at the top: what you think of the paper as a whole.
  **Start from an outline** fills in a skeleton with the counts per kind;
  you write the paragraph.
- The comments, **grouped by section** in reading order. Click a card's
  label line to go to it on the page (it is marked for a moment); click a
  comment's number on the page to find its card. **Edit** (or a double click)
  and **Delete** sit at the card's right; Delete can be undone from the
  status bar.
- **Filter** by kind with the chips above the list.
- **Papers you may have missed**, folded away at the end: items of your
  library that the citation graph ties to this paper (shared references,
  cited together) and that it does not cite. **Suggest** turns one into a
  literature comment citing it, at the paper's reference list. It needs the
  paper to be an item of your library, since the graph is the library's.

The review is saved as you go, per PDF (kept by the file's own identifier,
so it survives renaming and saving), in SpeedyF's data folder - never in
the PDF until you export.

## Sending it back

**Export PDF...** (also in the palette, **Markup: Export PDF...**) asks
where to write, beside the paper as "<name> - markup.pdf" by default, and
writes one PDF: the paper itself, each page widened on the right by a
margin strip. Nothing is added before or after it.

- **On the page**, each commented phrase keeps its soft highlight in the
  comment's colour (a strike-through and caret for a fix, a box for a
  region), as real PDF annotations holding the numbered comment, so any PDF
  reader lists them. A small number in the same colour sits just after the
  phrase (at the top-right corner of a region), matching its note.
- **In the margin**, each note is its number and its words - a fix reads as
  the struck words and what replaces them, a citation as its short label,
  math typeset. Notes sit level with what they are about; where two would
  collide they share the move, one up and one down, keeping their order,
  over the whole height of the page. Your Overall paragraph, if you wrote
  one, heads the first page's margin. A crowded page is set a little
  smaller.
- **Only as a last resort**, when a page has more to say than its margin
  can hold even then, its longest notes keep their first line and
  "(cont. p. N)", and their full text goes on one page added at the end of
  the document. A normal review adds no pages.

**Copy as text** puts the review on the clipboard as a plain letter for an
email: the Overall paragraph, the comments grouped by section with their
labels, the typos as a list, and the library items cited.

Export needs macOS for now: the margin is set by the system's web view.
