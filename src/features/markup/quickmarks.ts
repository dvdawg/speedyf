/** QuickMarks: the stock remarks a referee writes over and over. Typing "/"
 * at the start of a comment lists them; picking one writes its text and
 * sets the comment's kind, and the reviewer may go on typing after it. */
import type { CommentKind } from './types';

export interface QuickMark {
  id: string;
  text: string;
  kind: CommentKind;
}

export const QUICKMARKS: readonly QuickMark[] = [
  { id: 'gap', text: 'Gap in the argument', kind: 'proof' },
  { id: 'justify', text: 'Justify this step', kind: 'proof' },
  { id: 'hypothesis', text: 'Which hypothesis is used here?', kind: 'proof' },
  { id: 'cite', text: 'Citation needed', kind: 'literature' },
  { id: 'related', text: 'Related work to consider', kind: 'literature' },
  { id: 'define', text: 'Define this notation before use', kind: 'exposition' },
  { id: 'unclear', text: 'Unclear; please rephrase', kind: 'exposition' },
  { id: 'reorder', text: 'Consider moving this earlier/later', kind: 'exposition' },
  { id: 'motivate', text: 'Add motivation here', kind: 'exposition' },
  { id: 'typo', text: 'Typo', kind: 'typo' },
];

export function quickmarkById(id: string | null | undefined): QuickMark | null {
  return QUICKMARKS.find((q) => q.id === id) ?? null;
}

/** A "/" being typed at the very start of `text`, the caret still in its
 * first word: the query after the slash. Null otherwise. */
export function slashQuery(text: string, caret: number): string | null {
  if (!text.startsWith('/')) return null;
  const word = /^\/([A-Za-z-]*)/.exec(text)![0];
  if (caret < 1 || caret > word.length) return null;
  return word.slice(1, caret);
}

/** The QuickMarks for a query: prefix matches on the id first, then any
 * whose id contains it or a word of whose text starts with it. */
export function matchQuickmarks(query: string): QuickMark[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...QUICKMARKS];
  const prefix = QUICKMARKS.filter((m) => m.id.startsWith(q));
  const rest = QUICKMARKS.filter(
    (m) =>
      !prefix.includes(m) &&
      (m.id.includes(q) ||
        m.text
          .toLowerCase()
          .split(/[^a-z]+/)
          .some((word) => word.startsWith(q)))
  );
  return [...prefix, ...rest];
}

/** `text` with its leading "/word" replaced by the QuickMark's stock text,
 * and where the caret goes: just past it, after a space. */
export function applyQuickmark(text: string, mark: QuickMark): { text: string; caret: number } {
  const word = /^\/[A-Za-z-]*/.exec(text)?.[0] ?? '';
  const rest = text.slice(word.length).replace(/^\s+/, '');
  const head = mark.text.endsWith('?') || mark.text.endsWith('.') ? mark.text : `${mark.text}.`;
  const out = rest ? `${head} ${rest}` : `${head} `;
  return { text: out, caret: head.length + 1 };
}
