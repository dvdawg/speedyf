/** A note's outline: its headings, for the editor head's Outline list.
 *
 * Read from the blocks the editor already keeps, so the list follows every
 * edit without another pass over the note's text. */
import { inlineSpans, lineShape, type Citation, type Span } from './markdown';

export interface OutlineHeading {
  /** 0 for the shallowest heading level the note uses, then 1 and 2. */
  depth: number;
  /** The heading as it reads: inline Markdown as text, math as its TeX. */
  text: string;
  /** Where its line starts in the note. */
  offset: number;
}

/** Levels shown below the shallowest. */
const MAX_DEPTH = 3;

/** A heading line's level and body, or null. Only a one-line block is a
 * heading: a line in a quote, code or math is not. */
export function headingLine(kind: string, raw: string): { level: number; body: string } | null {
  if (kind !== 'line' || !raw.trimStart().startsWith('#')) return null;
  const shape = lineShape(raw);
  return shape.kind === 'heading' ? { level: shape.level, body: shape.body } : null;
}

/** Spans as plain text: emphasis and links as their words, code and math
 * as written inside them, a citation as `label` reads it. */
export function plainText(spans: Span[], label?: (cite: Citation, raw: string) => string): string {
  let out = '';
  for (const span of spans) {
    switch (span.kind) {
      case 'text':
      case 'code':
        out += span.text;
        break;
      case 'math':
        out += span.tex;
        break;
      case 'em':
      case 'strong':
      case 'link':
        out += plainText(span.children, label);
        break;
      case 'image':
        out += span.alt;
        break;
      case 'cite':
        out += label ? label(span.cite, span.raw) : span.raw;
        break;
    }
  }
  return out;
}

/** The outline of headings found at their offsets: depths counted from the
 * shallowest level used, and only the first three levels kept. */
export function outlineOf(
  found: readonly { level: number; body: string; offset: number }[],
  label?: (cite: Citation, raw: string) => string
): OutlineHeading[] {
  if (found.length === 0) return [];
  const top = Math.min(...found.map((h) => h.level));
  const out: OutlineHeading[] = [];
  for (const h of found) {
    const depth = h.level - top;
    if (depth >= MAX_DEPTH) continue;
    const text = plainText(inlineSpans(h.body), label).replace(/\s+/g, ' ').trim();
    out.push({ depth, text: text || '(untitled)', offset: h.offset });
  }
  return out;
}
