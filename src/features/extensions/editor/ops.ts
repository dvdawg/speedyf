/** Edits to a note as range replacements, in UTF-16 units (JavaScript string
 * offsets). The same rules as `src-tauri/src/library/projectdocs.rs`, which
 * orders edits from every view of a note: a client that maps and moves edits
 * differently from the core would drift from it. */

export interface Op {
  from: number;
  to: number;
  insert: string;
}

/** Apply, clamping a range that runs past the end. */
export function applyOp(text: string, op: Op): string {
  const from = Math.min(Math.max(0, op.from), text.length);
  const to = Math.min(Math.max(from, op.to), text.length);
  return text.slice(0, from) + op.insert + text.slice(to);
}

/** Where `pos` lands once `op` is applied. Inside the replaced range: the end
 * of the inserted text. Exactly at an insertion point: before it, unless
 * `after` (the position belongs to an edit ordered after `op`). */
export function mapPos(op: Op, pos: number, after = false): number {
  const inserted = op.insert.length;
  if (op.from === op.to) {
    return pos < op.from || (pos === op.from && !after) ? pos : pos + inserted;
  }
  if (pos <= op.from) return pos;
  if (pos >= op.to) return pos + inserted - (op.to - op.from);
  return op.from + inserted;
}

/** `op`, made against the same text as `earlier`, moved to apply after it.
 * `later` says which of the two the order puts second. */
export function transformOp(op: Op, earlier: Op, later: boolean): Op {
  const from = mapPos(earlier, op.from, later);
  const to = Math.max(from, mapPos(earlier, op.to, later));
  return { from, to, insert: op.insert };
}

/** The one range replacement taking `a` to `b`, or null when equal: the
 * longest common prefix and suffix are kept. */
export function diffOp(a: string, b: string): Op | null {
  if (a === b) return null;
  let start = 0;
  const min = Math.min(a.length, b.length);
  while (start < min && a.charCodeAt(start) === b.charCodeAt(start)) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a.charCodeAt(endA - 1) === b.charCodeAt(endB - 1)) {
    endA--;
    endB--;
  }
  return { from: start, to: endA, insert: b.slice(start, endB) };
}

/** The edit undoing `op` on the text it was applied to (`before`). */
export function invertOp(op: Op, before: string): Op {
  return {
    from: op.from,
    to: op.from + op.insert.length,
    insert: before.slice(op.from, Math.min(op.to, before.length)),
  };
}
