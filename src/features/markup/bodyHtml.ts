/** A comment body (or the Overall summary) as HTML: paragraphs, "- " lists,
 * **bold**, *italic*, `code`, `$math$` typeset by KaTeX, and `[@key]`
 * citations as their labels. One renderer for the drawer's cards and the
 * exported pages, so what the reviewer sees is what gets sent.
 *
 * Pure: KaTeX is handed in (`math`), so the node tests reach it. Every bit
 * of the reviewer's own text is escaped; only KaTeX output (from
 * `trust: false`, which emits no links or scripts) is passed through. */
import { splitMath } from '../../components/mathSplit';
import { citationsAsText } from './model';

export interface BodyOptions {
  /** TeX to HTML, or null while KaTeX has not loaded (the source shows). */
  math?: ((tex: string, display: boolean) => string) | null;
  /** Cite key -> its label without brackets ("Jar97"), null when unknown. */
  cite?: (key: string) => string | null;
}

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

/** Escaped prose with **bold**, *italic* and `code`. */
function marks(text: string): string {
  let html = escapeHtml(text);
  html = html.replace(/`([^`\n]+)`/g, '<code>$1</code>');
  html = html.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/(^|[^*\w])\*([^*\s][^*\n]*?)\*(?!\w)/g, '$1<em>$2</em>');
  html = html.replace(/(^|[^_\w])_([^_\s][^_\n]*?)_(?!\w)/g, '$1<em>$2</em>');
  return html;
}

const CITATION = /\[[^\]\n]*@[^\]\n]*\]/g;

/** Plain text (no math): citations as their labels, the rest with marks. */
function inlineText(text: string, opts: BodyOptions): string {
  let out = '';
  let at = 0;
  for (const m of text.matchAll(CITATION)) {
    out += marks(text.slice(at, m.index));
    const label = opts.cite ? citationsAsText(m[0], opts.cite) : m[0];
    out +=
      label !== m[0]
        ? `<span class="mk-cite">${escapeHtml(label)}</span>`
        : `<span class="mk-cite is-unknown">${escapeHtml(m[0])}</span>`;
    at = m.index + m[0].length;
  }
  return out + marks(text.slice(at));
}

/** One line of text with math in it. */
export function inlineHtml(text: string, opts: BodyOptions = {}): string {
  return splitMath(text)
    .map((segment) => {
      if (segment.kind === 'text') return inlineText(segment.text, opts);
      if (!opts.math) return `<span class="mk-math-raw">${escapeHtml(segment.raw)}</span>`;
      return `<span class="mk-math">${opts.math(segment.src, segment.display)}</span>`;
    })
    .join('');
}

const BULLET = /^\s*[-*]\s+/;
const DISPLAY_RUN = /\$\$([^$]+?)\$\$/g;

/** A body line: like `inlineHtml`, but a `$$...$$` run inside the line is
 * display math too (in a body, unlike a title, it always means that). */
function bodyLine(text: string, opts: BodyOptions): string {
  let out = '';
  let at = 0;
  for (const m of text.matchAll(DISPLAY_RUN)) {
    out += inlineHtml(text.slice(at, m.index), opts);
    const tex = m[1]!.trim();
    out += opts.math
      ? `<span class="mk-display">${opts.math(tex, true)}</span>`
      : `<span class="mk-math-raw">${escapeHtml(m[0])}</span>`;
    at = m.index + m[0].length;
  }
  return out + inlineHtml(text.slice(at), opts);
}

/** A whole body: blank lines part paragraphs, "- " lines make a list,
 * a `$$ ... $$` paragraph is display math, single newlines are kept. */
export function renderBody(text: string, opts: BodyOptions = {}): string {
  const paragraphs = text
    .replace(/\r\n?/g, '\n')
    .split(/\n\s*\n/)
    .map((p) => p.replace(/^\n+|\n+$/g, ''))
    .filter((p) => p.trim() !== '');
  return paragraphs
    .map((p) => {
      const display = /^\s*\$\$([\s\S]+?)\$\$\s*$/.exec(p);
      if (display) {
        const tex = display[1]!.trim();
        return opts.math
          ? `<div class="mk-display">${opts.math(tex, true)}</div>`
          : `<div class="mk-display mk-math-raw">${escapeHtml(p.trim())}</div>`;
      }
      const lines = p.split('\n');
      const out: string[] = [];
      let list: string[] = [];
      let prose: string[] = [];
      const flushList = () => {
        if (list.length > 0) out.push(`<ul>${list.map((li) => `<li>${li}</li>`).join('')}</ul>`);
        list = [];
      };
      const flushProse = () => {
        if (prose.length > 0) out.push(`<p>${prose.join('<br>')}</p>`);
        prose = [];
      };
      for (const line of lines) {
        if (BULLET.test(line)) {
          flushProse();
          list.push(bodyLine(line.replace(BULLET, ''), opts));
        } else if (list.length > 0 && /^\s+\S/.test(line)) {
          list[list.length - 1] += ` ${bodyLine(line.trim(), opts)}`;
        } else {
          flushList();
          prose.push(bodyLine(line, opts));
        }
      }
      flushList();
      flushProse();
      return out.join('');
    })
    .join('');
}
