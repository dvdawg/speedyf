/** Notes on screen that take a drop: rows pulled from the Projects list
 * land on a notepad as citations. The drag itself is the list's pointer
 * drag (projects/pointerDrag.ts); this is only where it can land, found the
 * way dnd.ts finds a project row, by asking the DOM what is under a point. */

export interface NoteDropTarget {
  /** Insert `text` where the point is in the note, or at its end when the
   * point is in no line. Through the editor's own edit path, so undo takes
   * it back. */
  insertAt(x: number, y: number, text: string): void;
}

const targets = new Map<Element, NoteDropTarget>();

/** An editor's element takes drops; returns the unregister. */
export function registerNoteDrop(el: Element, target: NoteDropTarget): () => void {
  targets.set(el, target);
  return () => {
    if (targets.get(el) === target) targets.delete(el);
  };
}

/** The note under a point, if any. */
export function noteDropAt(x: number, y: number): NoteDropTarget | null {
  const hit =
    typeof document.elementFromPoint === 'function' ? document.elementFromPoint(x, y) : null;
  for (let node: Element | null = hit; node; node = node.parentElement) {
    const target = targets.get(node);
    if (target) return target;
  }
  return null;
}

/** Cite keys as the note writes them: `@a`, or `@a; @b` for several. */
export function citeText(keys: string[]): string {
  return keys.map((key) => `@${key}`).join('; ');
}
