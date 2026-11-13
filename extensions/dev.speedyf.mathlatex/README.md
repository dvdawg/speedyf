# Math to LaTeX

Select math in a PDF and pick **Copy as LaTeX**, or hover a display equation
and click **Copy LaTeX**. A popover shows the LaTeX rendered, with buttons to
copy it raw, as Markdown (`$…$` / `$$…$$`), or as an `align` block when it
has several lines. A dim note says where the LaTeX came from and how sure it
is.

It ships **disabled**. Turning it on in **Home → Extensions** is the consent
to the network use described below; with **Prefer → fonts only (offline)**
nothing ever leaves the machine.

## Where the LaTeX comes from

Most exact first. Each step is used only when it applies, and the first one
that answers wins.

1. **The document's own tags.** A tagged PDF can carry each formula as a
   Formula element with its LaTeX in the alt text, or as MathML. When one
   covers the selection, that is the answer, exactly. (Read locally.)
2. **The arXiv source.** A paper with an arXiv id has its TeX on arxiv.org.
   The e-print for that exact version is fetched once, unpacked here (gzip,
   then tar; a single-file submission is just gzipped TeX; a PDF-only
   submission is noted and skipped), `\input` and `\include` are followed,
   and every piece of math in it — `$…$`, `\(…\)`, `\[…\]`, `$$…$$`,
   `equation`, `align`, `gather`, `multline`, `eqnarray`, starred or not — is
   matched against what you selected. Argument-free macros
   (`\newcommand{\R}{\mathbb{R}}`, `\def\eps{\varepsilon}`,
   `\DeclareMathOperator`) are expanded so the result renders; `\label`,
   `\nonumber` and `\tag` are dropped. For an equation's **Copy LaTeX**, its
   printed number picks between look-alikes.
3. **An open-access journal version.** For a DOI, Europe PMC is asked whether
   it holds the full text (this covers PLOS, eLife and the rest of PubMed
   Central). If it does, the article's JATS XML is fetched and its formulas
   read: the publisher's own TeX when it includes some, otherwise its MathML,
   converted.
4. **The fonts.** Reading the glyphs on the page. A glyph's Unicode text,
   when the PDF has one, says what it is; its font says how it is styled
   (CMSY's capitals are calligraphic, MSBM's blackboard, EUFM's fraktur) and,
   for TeX's extension font, how tall a delimiter is. A glyph with no
   Unicode is read by its code through the TeX encodings (CMMI, CMSY, CMEX,
   MSAM, MSBM). A Symbol font whose text came out as Mac Roman (Â for ∑) is
   read back through the Symbol encoding. The layout, from baselines and
   sizes, says how they fit together: smaller and raised is a superscript, a
   rule with symbols above and below is a fraction, a radical sign with a
   rule over its contents is a square root, rows inside tall parentheses are
   a matrix, brace tips with a label under them an `\underbrace`. This is a
   reconstruction: it is marked **high** confidence only when every glyph
   was identified and the layout was unambiguous.

Matching a selection against source compares the symbols in it, in order,
without structure: `\frac{a+b}{c}`, the glyphs on the page and the selected
text all reduce to `a + b c`. A selection that is part of a longer equation
gets just that part when it can be cut out cleanly.

Sources are remembered: in the extension's storage when they are small, in
memory for the session otherwise. Asking again for the same paper does not
fetch it again.

## What leaves your machine

Only when **Prefer** is "most exact" and the step's own setting is on:

- **arxiv.org** receives the paper's arXiv id and version (to fetch its
  source).
- **www.ebi.ac.uk** (Europe PMC) receives the paper's DOI, and then the
  PubMed Central id it answered with.

Nothing else: not the selection, not the document, not your library. These
two are the only hosts the manifest declares, so they are the only ones the
app will let it reach.

## Settings

- **Use the arXiv source** — step 2 on or off.
- **Use open-access journal versions** — step 3 on or off.
- **Prefer** — "most exact" uses every step; "fonts only (offline)" uses the
  document's tags and its glyphs and never the network.

## What it cannot do

- **Scans.** A scanned page has no fonts, and its OCR layer is a guess at the
  letters; math reconstructed from it would be invented. It says so ("This
  looks like a scan; math can't be read from it.") and shows nothing.
- **Unusual layouts** read from the fonts: commutative diagrams, `\overbrace`
  with labels, tables of formulas, and anything drawn as vector paths are not
  recognised. Confidence drops to medium or low when something could not be
  placed.
- A selection with no glyph access falls back to the selected text alone,
  through the Unicode table, with low confidence.
