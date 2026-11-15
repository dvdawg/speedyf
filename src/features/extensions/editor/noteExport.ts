/** A note made ready to leave SpeedyF: as a Markdown file of its own, or
 * as LaTeX for a paper. Read without a DOM, so it is tested on its own.
 *
 * Both read the note the way the editor draws it (`markdown.ts`): code and
 * display math are left exactly as written, and only prose is changed. */
import {
  codeSource,
  footnotesIn,
  inlineSpans,
  lineShape,
  mathSource,
  noterefLabel,
  noterefText,
  splitBlocks,
  statementHead,
  type Citation,
  type Span,
} from './markdown';

/* ---- Markdown ---- */

/** An image the note's own folder holds, as the editor accepts one: a
 * plain relative path. Anything else (a web address) is left alone. */
export function isNoteAsset(src: string): boolean {
  return /^[A-Za-z0-9_-][A-Za-z0-9._/-]*$/.test(src) && !src.includes('..');
}

/** Where a picture goes in the export's folder: its path less a leading
 * `assets/`. The same rule as the core's `export_asset_name`. */
export function exportAssetName(src: string): string {
  return src.startsWith('assets/') ? src.slice('assets/'.length) : src;
}

/** The folder of pictures beside an exported `<name>.md`. */
export function exportFolderName(fileName: string): string {
  const base = fileName.split(/[\\/]/).pop() ?? fileName;
  const stem = base.replace(/\.(md|markdown|txt)$/i, '');
  return `${stem || 'notes'}-assets`;
}

/** Apply `change` to the parts of a line outside inline code. */
function outsideCode(line: string, change: (part: string) => string): string {
  return line
    .split(/(`[^`]*`)/)
    .map((part, i) => (i % 2 === 1 ? part : change(part)))
    .join('');
}

/** The note as an exported Markdown file: pictures from its folder linked
 * into `folder` beside it (spaces and the like percent-encoded), the
 * `{box=...}` a page link or a statement citation carries for SpeedyF
 * dropped, so `[@key, p. 12]` and `[@key, Prop. 2.2]` are plain Pandoc,
 * and a cross-reference into a paper (`[Theorem 2.1](@key#p3){box=...}`)
 * its words; a reference marker's `{label=... at=...}` goes too. Reference entries (`[^Sim15]: ...`) are Pandoc footnotes
 * already. `assets` lists the pictures to copy, as the note names them. */
export function exportMarkdown(text: string, folder: string): { text: string; assets: string[] } {
  const assets: string[] = [];
  const blocks = splitBlocks(text).map((block) => {
    if (block.kind === 'code' || block.kind === 'math') return block.raw;
    return block.raw
      .split('\n')
      .map((line) =>
        outsideCode(line, (part) =>
          part
            .replace(/(\[@[^\]\n]*\])\{box=[^}\n]*\}/g, '$1')
            .replace(
              /(\[\^[A-Za-z0-9_.:+-]{1,80}\])\{(?:\s*(?:label=(?:"[^"\n]*"|[^\s}"]+)|at=\d{1,3}))+\s*\}/g,
              '$1'
            )
            .replace(/\[([^\]\n]+)\]\(@[^)\s]+\)(?:\{box=[^}\n]*\})?/g, '$1')
            .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (whole, alt: string, src: string) => {
              if (!isNoteAsset(src)) return whole;
              if (!assets.includes(src)) assets.push(src);
              return `![${alt}](${encodeURI(`${folder}/${exportAssetName(src)}`)})`;
            })
        )
      )
      .join('\n');
  });
  return { text: blocks.join('\n'), assets };
}

/* ---- LaTeX ---- */

const SPECIAL: Record<string, string> = {
  '\\': '\\textbackslash{}',
  '{': '\\{',
  '}': '\\}',
  $: '\\$',
  '&': '\\&',
  '#': '\\#',
  _: '\\_',
  '%': '\\%',
  '^': '\\textasciicircum{}',
  '~': '\\textasciitilde{}',
};

/** Prose with LaTeX's special characters escaped. */
export function escapeLatex(text: string): string {
  return text.replace(/[\\{}$&#_%^~]/g, (c) => SPECIAL[c]!);
}

/** A locator as LaTeX writes one: `p. 12` -> `p.~12`, `pp. 3-5` ->
 * `pp.~3--5`, `Prop. 2.2` -> `Prop.~2.2`. */
function latexLocator(locator: string): string {
  const m = /^(pp?\.|pages?|[A-Za-z]{2,8}\.?)\s+(.*)$/.exec(locator.trim());
  const range = (s: string) => escapeLatex(s).replace(/(\d)\s*[-\u2013]\s*(\d)/g, '$1--$2');
  return m ? `${m[1]}~${range(m[2]!)}` : range(locator.trim());
}

export interface LatexOptions {
  /** The key to write for a cited key (a key the item had before, say,
   * read as the one it has now). The key as written by default. */
  key?: (key: string) => string;
  /** The note's reference entries (`[^id]: ...`), by id. */
  notes?: Map<string, string>;
}

/** Inline spans as LaTeX: emphasis, code, math kept as written, links, and
 * citations, neighbours with nothing but `;` or `,` between them grouped
 * into one `\cite{a,b}` (a citation with a locator stands alone). */
function inlineLatex(spans: Span[], opts: LatexOptions): string {
  const keyOf = (cite: Citation) => (opts.key ? opts.key(cite.key) : cite.key);
  let line = '';
  /** A `]` closing a bracketed group of citations, dropped with its `[`. */
  let dropClose = false;
  let i = 0;
  while (i < spans.length) {
    const span = spans[i]!;
    if (span.kind === 'cite') {
      if (span.cite.locator) {
        line += `\\cite[${latexLocator(span.cite.locator)}]{${keyOf(span.cite)}}`;
        i++;
        continue;
      }
      const keys = [keyOf(span.cite)];
      let j = i + 1;
      while (j + 1 < spans.length) {
        const between = spans[j]!;
        const next = spans[j + 1]!;
        if (
          between.kind !== 'text' ||
          !/^\s*[;,]?\s*$/.test(between.text) ||
          next.kind !== 'cite' ||
          next.cite.locator
        ) {
          break;
        }
        keys.push(keyOf(next.cite));
        j += 2;
      }
      // `[@a; @b]` reads as one bracketed group: its brackets go too.
      const after = spans[j];
      if (line.endsWith('[') && after?.kind === 'text' && after.text.startsWith(']')) {
        line = line.slice(0, -1);
        dropClose = true;
      }
      line += `\\cite{${[...new Set(keys)].join(',')}}`;
      i = j;
      continue;
    }
    switch (span.kind) {
      case 'text':
        line += escapeLatex(dropClose ? span.text.slice(1) : span.text);
        dropClose = false;
        break;
      case 'em':
        line += `\\emph{${inlineLatex(span.children, opts)}}`;
        break;
      case 'strong':
        line += `\\textbf{${inlineLatex(span.children, opts)}}`;
        break;
      case 'code':
        line += `\\texttt{${escapeLatex(span.text)}}`;
        break;
      case 'math':
        line += `$${span.tex}$`;
        break;
      case 'link': {
        const href = span.href.replace(/[%#\\{}]/g, (c) => `\\${c}`);
        line += `\\href{${href}}{${inlineLatex(span.children, opts)}}`;
        break;
      }
      case 'xref':
        line += escapeLatex(span.text);
        break;
      case 'noteref': {
        const entry = opts.notes?.get(span.id);
        line += escapeLatex(noterefLabel(span.id, entry, span.label));
        if (entry) line += `\\footnote{${escapeLatex(noterefText(entry))}}`;
        break;
      }
      case 'group': {
        // The items' citations as one \cite (the bibliography style orders
        // them), one with a locator on its own, then each reference no
        // item is as its printed label with its entry as a footnote.
        const plain = span.items.flatMap((it) =>
          it.kind === 'cite' && !it.cite.locator ? [keyOf(it.cite)] : []
        );
        if (plain.length > 0) line += `\\cite{${[...new Set(plain)].join(',')}}`;
        for (const it of span.items) {
          if (it.kind === 'cite' && it.cite.locator) {
            line += `\\cite[${latexLocator(it.cite.locator)}]{${keyOf(it.cite)}}`;
          } else if (it.kind === 'noteref') {
            const entry = opts.notes?.get(it.id);
            line += escapeLatex(noterefLabel(it.id, entry, it.label));
            if (entry) line += `\\footnote{${escapeLatex(noterefText(entry))}}`;
          }
        }
        break;
      }
      case 'image': {
        // A comment ends its line: the picture is noted on a line of its own.
        const alt = span.alt ? ` (${span.alt.replace(/\s+/g, ' ')})` : '';
        line += `\n% figure: ${span.src.replace(/\s+/g, ' ')}${alt}\n`;
        break;
      }
    }
    i++;
  }
  return line;
}

const SECTIONS = ['section', 'subsection', 'subsubsection', 'paragraph'];

/** A one-line `$$ ... $$` (as a quote can hold), or null. */
function displayIn(body: string): string | null {
  const m = /^\s*\$\$([\s\S]*?)\$\$\s*$/.exec(body);
  return m && m[1]!.trim() ? m[1]!.trim() : null;
}

/** The note as LaTeX, for pasting into a paper. */
export function noteToLatex(text: string, options: LatexOptions = {}): string {
  const opts: LatexOptions = { notes: footnotesIn(text), ...options };
  const blocks = splitBlocks(text);
  const levels = blocks
    .filter((b) => b.kind === 'line')
    .map((b) => lineShape(b.raw))
    .flatMap((shape) => (shape.kind === 'heading' ? [shape.level] : []));
  const top = levels.length > 0 ? Math.min(...levels) : 1;
  const inline = (s: string) =>
    inlineLatex(inlineSpans(s), opts)
      .replace(/[ \t]*\n[ \t]*/g, '\n')
      .trim();

  const out: string[] = [];
  /** Lists open now, innermost last. */
  const lists: { indent: number; env: 'itemize' | 'enumerate' }[] = [];
  const closeLists = (toIndent = -1) => {
    while (lists.length > 0 && lists[lists.length - 1]!.indent > toIndent) {
      out.push(`${'  '.repeat(lists.length - 1)}\\end{${lists.pop()!.env}}`);
    }
  };
  const paragraphBreak = () => {
    if (out.length > 0 && out[out.length - 1] !== '') out.push('');
  };

  for (const block of blocks) {
    if (block.kind !== 'line') closeLists();
    if (block.kind === 'math') {
      paragraphBreak();
      out.push('\\[', mathSource(block.raw), '\\]');
      continue;
    }
    if (block.kind === 'code') {
      paragraphBreak();
      out.push('\\begin{verbatim}', codeSource(block.raw), '\\end{verbatim}');
      paragraphBreak();
      continue;
    }
    if (block.kind === 'quote') {
      paragraphBreak();
      out.push('\\begin{quote}');
      let gap = false;
      // A statement: its label bold, its name upright, its body in the
      // kind's own shape (italic for a theorem), as amsthm sets it.
      const lines = block.raw.split('\n');
      const firstShape = lineShape(lines[0] ?? '');
      const head = firstShape.kind === 'quote' ? statementHead(firstShape.body) : null;
      const shaped = (latex: string) =>
        head?.italic && latex && !/^\\cite/.test(latex) ? `{\\itshape ${latex}}` : latex;
      for (const [n, line] of lines.entries()) {
        const shape = lineShape(line);
        let body = shape.kind === 'quote' ? shape.body : line;
        if (n === 0 && head) {
          const label = head.number ? `${head.word} ${head.number}` : head.word;
          const title = head.title ? ` (${inline(head.title)})` : '';
          const rest = inline(head.rest);
          out.push(`\\textbf{${escapeLatex(label)}}${title}. ${shaped(rest)}`.trimEnd());
          continue;
        }
        if (body.trim() === '') {
          gap = true;
          continue;
        }
        if (gap) out.push('');
        gap = false;
        const display = displayIn(body);
        body = display !== null ? `\\[ ${display} \\]` : shaped(inline(body));
        out.push(body);
      }
      out.push('\\end{quote}');
      paragraphBreak();
      continue;
    }
    const shape = lineShape(block.raw);
    if (shape.kind === 'bullet' || shape.kind === 'ordered') {
      const env = shape.kind === 'bullet' ? 'itemize' : 'enumerate';
      closeLists(shape.indent);
      const open = lists[lists.length - 1];
      if (open && open.indent === shape.indent && open.env !== env) closeLists(shape.indent - 1);
      const current = lists[lists.length - 1];
      if (!current || current.indent < shape.indent) {
        if (lists.length === 0) paragraphBreak();
        out.push(`${'  '.repeat(lists.length)}\\begin{${env}}`);
        lists.push({ indent: shape.indent, env });
      }
      out.push(`${'  '.repeat(lists.length)}\\item ${inline(shape.body)}`);
      continue;
    }
    closeLists();
    switch (shape.kind) {
      case 'blank':
        paragraphBreak();
        break;
      case 'heading': {
        const name = SECTIONS[Math.min(shape.level - top, SECTIONS.length - 1)]!;
        paragraphBreak();
        out.push(`\\${name}{${inline(shape.body)}}`);
        paragraphBreak();
        break;
      }
      case 'rule':
        paragraphBreak();
        out.push('\\noindent\\rule{\\linewidth}{0.4pt}');
        paragraphBreak();
        break;
      case 'quote':
        // Unreachable: quote lines are quote blocks.
        out.push(inline(shape.body));
        break;
      case 'footnote':
        // A reference entry: written as the footnote of its mention.
        break;
      case 'para':
        out.push(inline(shape.body));
        break;
    }
  }
  closeLists();
  return `${out
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/^\n+|\n+$/g, '')}\n`;
}
