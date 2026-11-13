/** Where the Projects screen's drags land.
 *
 * Two drags start on the screen — rows pulled onto a project, a project
 * pulled among the others — and one arrives from outside: a PDF from the
 * desktop, which App receives from Tauri as a path and a position. All three
 * find their target the same way, by asking the DOM what is under a point;
 * a project row says so with `data-drop-project`, and the screen itself with
 * its class. The in-screen drags are pointer events (pointerDrag.ts); this
 * file is only the hit-testing and the one signal the column highlights
 * from. */
import { createSignal } from 'solid-js';
import { projectsStore } from './projectsStore';

/** The row a drag is over: a project taking rows, or the gap before or
 * after a project taking a reorder. One signal for both, since one drag runs
 * at a time. */
export type DropHover = { kind: 'into'; id: string } | { kind: 'slot'; id: string; after: boolean };

const [hover, setHover] = createSignal<DropHover | null>(null);

export const dropHover = hover;

const hoverKey = (h: DropHover | null) =>
  h === null ? '' : `${h.kind}:${h.id}:${h.kind === 'slot' ? String(h.after) : ''}`;

/** Deduplicated: a drag reports on every move, and every row's classes
 * would otherwise be re-evaluated on every one. */
export function setDropHover(next: DropHover | null): void {
  if (hoverKey(hover()) !== hoverKey(next)) setHover(next);
}

function elementAt(x: number, y: number): Element | null {
  return typeof document.elementFromPoint === 'function' ? document.elementFromPoint(x, y) : null;
}

function projectRowOf(el: Element | null): { el: HTMLElement; id: string } | null {
  const row = el?.closest<HTMLElement>('[data-drop-project]') ?? null;
  const id = row?.dataset.dropProject;
  return row && id ? { el: row, id } : null;
}

/** The project row under a point. */
export function projectRowAt(x: number, y: number): { el: HTMLElement; id: string } | null {
  return projectRowOf(elementAt(x, y));
}

/** Where a reorder would land: before or after the row under the pointer,
 * by which half of it the pointer is in. */
export function reorderSlotAt(x: number, y: number): { id: string; after: boolean } | null {
  const row = projectRowAt(x, y);
  if (!row) return null;
  const rect = row.el.getBoundingClientRect();
  return { id: row.id, after: y > rect.top + rect.height / 2 };
}

export type ProjectsDropTarget = { kind: 'project'; id: string; name: string } | { kind: 'screen' };

/** What a file dropped from the desktop at a point would land on: a project
 * row, the Projects screen around it, or (null) nothing of ours — another
 * screen, or the chrome around this one. Null whenever the screen is not
 * mounted. */
export function projectsDropTarget(x: number, y: number): ProjectsDropTarget | null {
  const el = elementAt(x, y);
  if (!el?.closest('.projects-screen')) return null;
  const row = projectRowOf(el);
  if (row) {
    const project = projectsStore.state.projects.find((p) => p.id === row.id);
    if (project) return { kind: 'project', id: project.id, name: project.name };
  }
  return { kind: 'screen' };
}

/** What the overlay says while a file is held over the window. */
export function dropOverlayText(target: ProjectsDropTarget | null): string {
  if (target === null) return 'Drop PDF to open';
  return target.kind === 'project'
    ? `Drop PDF to add to ${target.name}`
    : 'Drop PDF to add to the library';
}
