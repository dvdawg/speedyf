# Math to LaTeX: the host contract

What the core provides to this extension. Written before either side was
built so they can be built in parallel; the core agent implements exactly
this, the extension consumes exactly this.

## Manifest additions

```json
{
  "permissions": ["document:read", "document:layout", "clipboard:write", "network"],
  "network": ["arxiv.org", "www.ebi.ac.uk"],
  "contributes": {
    "selectionActions": [{ "command": "copyLatex", "title": "Copy as LaTeX" }],
    "equationActions": [{ "command": "copyEquation", "title": "Copy LaTeX" }]
  }
}
```

- `document:layout` (new): glyph geometry and math tags of the open document.
- `network` (new): outbound HTTP to the hosts in `network` only. The card
  shows "Uses the internet: <hosts>". A built-in declaring it ships disabled.
- `equationActions` (new contribution point): the core detects display
  equations on the page and shows a small "Copy LaTeX" button on hover; a
  click invokes the command.

## Invocations

- Selection action: `{ type: 'invoke', command: 'copyLatex', value: <selected text> }`
  (unchanged). Call `selection.glyphs` for the geometry.
- Equation action: `{ type: 'invoke', command: 'copyEquation', value: { page, bbox: [x, y, w, h], number: string | null } }`
  where bbox is in page points, top-left origin, display orientation.
  Call `region.glyphs { page, bbox }` for the glyphs.

## APIs (all via `call`)

| API                    | Args                                                                                                                           | Returns                                                                                                             | Needs                         |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| `selection.glyphs`     | none                                                                                                                           | `{ page, glyphs, groups, lines } \| null` (current text selection)                                                  | document:layout               |
| `region.glyphs`        | `{ page, bbox }`                                                                                                               | `{ page, glyphs, groups, lines }` (the equation at `bbox`, grown to all of it)                                      | document:layout               |
| `document.mathTags`    | `{ page }`                                                                                                                     | `[{ bbox, alt: string \| null, actualText: string \| null, mathml: string \| null }]` (tagged-PDF Formula elements) | document:layout               |
| `document.identifiers` | none                                                                                                                           | `{ doi: string \| null, arxiv: string \| null, arxivVersion: number \| null }`                                      | document:read                 |
| `net.fetch`            | `{ url, as: 'text' \| 'bytes', maxBytes?: number }`                                                                            | `{ status, contentType, body: string \| number[] }`                                                                 | network (host must be listed) |
| `ui.showMath`          | `{ latex: string, display: boolean, source: 'tags' \| 'arxiv' \| 'jats' \| 'fonts', confidence: 'high' \| 'medium' \| 'low' }` | `{ shown: true }`                                                                                                   | none                          |

`Glyph`:

```ts
{
  text: string; // Unicode from the PDF's ToUnicode map ('' if none)
  code: number; // raw character code in the font (for TeX font decoding)
  font: string; // base font name, subset prefix stripped: 'CMMI10', 'CMSY10', 'CMEX10', 'MSBM10', 'CambriaMath', 'STIXTwoMath-Regular', ...
  size: number; // font size in points (matrix-scaled)
  x: number;
  y: number;
  w: number;
  h: number; // loose box, page points, top-left origin
  baseline: number; // baseline y, page points
  bold: boolean;
  italic: boolean;
  ink: [x, y, w, h] | null; // the glyph's own ink box, when PDFium can say
}
```

The loose box is the font's ascent and descent: the same for every glyph of
a font, so it says nothing about how tall a delimiter or big operator is.
`ink` does. PDFium exposes no raw codes for a font that has a Unicode
mapping: `code` is the raw code only when `text` is empty, and otherwise the
Unicode scalar. A character outside the Basic Multilingual Plane (a
mathematical italic Y, U+1D44C, from an OpenType math font) is one glyph, its `text` the full character.

`region.glyphs` treats `bbox` as an equation's block: any glyph whose ink
touches it is taken, then the glyphs on the same baselines next to those
(a left-hand side) and those stacked right on a bar taken (a numerator, a
denominator). The host renders the page and reads its layout with classical
image analysis: `ink` comes from the connected components each glyph
covers; a bar the PDF did not draw as a path (found as a long thin run of
ink) is added as a `__rule` glyph; `groups` (`[x, y, w, h][]`) are the 2-D
stacks a small vertical dilation fuses, a fraction with its numerator and
denominator, an operator with its limits; `lines` (`[x, y, w, h][]`, top
down) are the strips of ink joined through those stacks. Both are empty when
the page could not be rendered.

Rules lines/rects that form fraction bars and radicals are returned as
glyphs with `font: '__rule'`, `text: ''`, `code: 0` (thin horizontal boxes).

`ui.showMath` shows a popover anchored at the selection or equation with the
LaTeX rendered by KaTeX and three copy buttons: "LaTeX" (raw), "Markdown"
(`$...$` or `$$...$$`), and, when the LaTeX has `\\` line breaks, "align"
(`\begin{align} ... \end{align}`). Copying happens on the button click
(user gesture), so the extension does not write the clipboard itself in this
flow. The source and confidence show as a small dim note.

Network calls respect the per-host rate limiter. `maxBytes` defaults to
5 MB, hard cap 20 MB. A host not in the manifest list is refused.
