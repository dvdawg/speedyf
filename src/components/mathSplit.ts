/** Splitting text into plain runs and TeX math runs. Kept free of Solid and
 * KaTeX so the node-environment tests reach it and the list views can call
 * it per row without loading anything. */

export type MathSegment =
  | { kind: 'text'; text: string }
  /** `src` is the TeX between the delimiters; `raw` is the original text,
   * delimiters included, shown until KaTeX has loaded. */
  | { kind: 'math'; src: string; raw: string; display: boolean };

/** Find the closing `$` of an inline `$...$` run opened at `open`.
 *
 * The rules are the ones Pandoc uses to tell math from prices: the opening
 * `$` needs a non-space right after it, the closing one a non-space right
 * before it and no digit right after it. Only the first unescaped `$` is
 * considered, so "$5 and $10" stays plain and "$5 and $x$" keeps "$5 and "
 * as text. */
function closeDollar(text: string, open: number): number {
  const first = text[open + 1];
  if (first === undefined || first === '$' || /\s/.test(first)) return -1;
  for (let j = open + 1; j < text.length; j++) {
    const c = text[j];
    if (c === '\\') {
      j += 1;
      continue;
    }
    if (c !== '$') continue;
    const before = text[j - 1]!;
    const after = text[j + 1];
    if (/\s/.test(before)) return -1;
    if (after !== undefined && /[0-9]/.test(after)) return -1;
    return j;
  }
  return -1;
}

function split(text: string): MathSegment[] {
  const out: MathSegment[] = [];
  let plain = '';
  const flush = () => {
    if (plain) out.push({ kind: 'text', text: plain });
    plain = '';
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i]!;
    const next = text[i + 1];
    if (c === '\\' && next === '$') {
      plain += '$';
      i += 2;
      continue;
    }
    if (c === '\\' && (next === '(' || next === '[')) {
      const closer = next === '(' ? '\\)' : '\\]';
      const end = text.indexOf(closer, i + 2);
      const src = end === -1 ? '' : text.slice(i + 2, end);
      if (src.trim()) {
        flush();
        out.push({ kind: 'math', src, raw: text.slice(i, end + 2), display: next === '[' });
        i = end + 2;
        continue;
      }
    }
    if (c === '$') {
      if (next === '$') {
        // `$$` is display math in TeX; titles never mean it, so it stays text.
        plain += '$$';
        i += 2;
        continue;
      }
      const end = closeDollar(text, i);
      if (end !== -1) {
        flush();
        out.push({
          kind: 'math',
          src: text.slice(i + 1, end),
          raw: text.slice(i, end + 1),
          display: false,
        });
        i = end + 1;
        continue;
      }
    }
    plain += c;
    i += 1;
  }
  flush();
  return out;
}

const CACHE_LIMIT = 4000;
const cache = new Map<string, MathSegment[]>();

/** Split `text` into plain and math runs, memoized per string: the paper list
 * asks for the same titles on every scroll. */
export function splitMath(text: string): MathSegment[] {
  let hit = cache.get(text);
  if (hit === undefined) {
    hit = split(text);
    if (cache.size >= CACHE_LIMIT) cache.clear();
    cache.set(text, hit);
  }
  return hit;
}

/** Whether `text` has any math run at all. */
export function hasMath(text: string): boolean {
  return splitMath(text).some((segment) => segment.kind === 'math');
}

const PLAIN: Record<string, string> = {
  '\\geq': '>=',
  '\\ge': '>=',
  '\\leq': '<=',
  '\\le': '<=',
  '\\neq': '!=',
  '\\leftarrow': '<-',
  '\\rightarrow': '->',
  '\\to': '->',
  '\\times': 'x',
  '\\approx': '~',
};

/** `text` with each math run spelled in plain ASCII, for the places that
 * cannot typeset (tooltips, aria labels): `$\geq 2$` becomes `>= 2`. */
export function plainMath(text: string): string {
  return splitMath(text)
    .map((segment) => {
      if (segment.kind === 'text') return segment.text;
      return segment.src
        .replace(/\\[a-zA-Z]+/g, (command) => PLAIN[command] ?? command)
        .replace(/\s+/g, ' ')
        .trim();
    })
    .join('');
}
