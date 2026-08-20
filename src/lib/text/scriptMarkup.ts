/** Reading the script notation the engine reconstructs from a PDF.
 *
 * A PDF stores no markup: "L^2 Optimality" is drawn as an "L" and a smaller
 * "2" sitting a few points higher. The engine recovers that from glyph
 * geometry and writes it back as the LaTeX an author would have typed
 * (`engine/formal.rs`), which this turns into something to render.
 *
 * Only the two constructs the engine emits are understood — `^x`/`_x` and
 * their braced forms. This is deliberately not a LaTeX parser: the input is
 * prose with a few scripts in it, not math source. */

export type ScriptKind = 'normal' | 'super' | 'sub';

export interface ScriptSegment {
  text: string;
  kind: ScriptKind;
}

const MARKERS: Record<string, ScriptKind> = { '^': 'super', _: 'sub' };

export function parseScriptMarkup(value: string): ScriptSegment[] {
  const segments: ScriptSegment[] = [];
  let plain = '';
  const flush = () => {
    if (plain) segments.push({ text: plain, kind: 'normal' });
    plain = '';
  };

  for (let i = 0; i < value.length; i += 1) {
    const kind = MARKERS[value[i]!];
    const next = value[i + 1];
    // A marker with nothing to lift is just a character — "x_" or a stray "^".
    if (!kind || next === undefined) {
      plain += value[i];
      continue;
    }
    if (next === '{') {
      const close = value.indexOf('}', i + 2);
      // An unclosed brace is malformed; treat the marker literally rather
      // than swallowing the rest of the title.
      if (close === -1) {
        plain += value[i];
        continue;
      }
      const inner = value.slice(i + 2, close);
      if (inner) {
        flush();
        segments.push({ text: inner, kind });
      }
      i = close;
      continue;
    }
    flush();
    segments.push({ text: next, kind });
    i += 1;
  }
  flush();
  return segments;
}

/** The same string with its markup removed, for titles, labels and search. */
export function plainScriptText(value: string): string {
  return parseScriptMarkup(value)
    .map((segment) => segment.text)
    .join('');
}
