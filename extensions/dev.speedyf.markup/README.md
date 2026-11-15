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
  number in the page's margin to find its card. **Edit** (or a double click)
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
writes one PDF:

- **The paper**, each page widened on the right by a margin strip. Commented
  words are marked with real PDF annotations (a highlight, a strike-through
  and caret for a fix, a box for a region), each holding its numbered
  comment, so any PDF reader lists them.
- **Margin notes** beside each page, level with what they are about,
  numbered, math typeset. When a page has more to say than room, the
  longest notes are shortened to their first lines with "... see 7 on
  p. 14", pointing into the letter.
- **The letter** at the end, set like a careful referee report: "Comments
  on <title>" and the date, the Overall paragraph, the comments by section
  in paper order (label line, the quoted words, kind, comment), then a
  table of **Typos and wording** (page, original, suggested), then
  **References** for the library items cited. The same QuickMark on the
  same words again is one entry, "Also p. 7, p. 9."

Numbers are links both ways: a margin note's number opens its entry in the
letter, an entry's number goes to its place on the page.

**Copy as text** puts the same letter on the clipboard as plain text, for an
email.

Export needs macOS for now: the sheets are set by the system's web view.
