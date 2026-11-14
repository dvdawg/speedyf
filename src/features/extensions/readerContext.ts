/** What the reader's document is, as far as projects are concerned: the
 * library item it is, the projects holding it, and which of them the
 * reader is working in.
 *
 * Which project: the one the paper was opened from (the Projects screen's
 * list, graph or detail for a project, or a notepad link); otherwise, when
 * the paper is in exactly one project, that one; when it is in several, the
 * one picked for it this session (the drawer's switcher, or the small picker
 * a reader action asks with); when in none, none, and nothing project-shaped
 * is offered - the reader stays a PDF viewer first.
 *
 * Window-level, like the rest of the extension host: every tab's document
 * is looked up when it becomes the active one. */
import { createEffect, createRoot, createSignal, untrack } from 'solid-js';
import { createStore } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import { activeTab } from '../../stores/tabsStore';
import type { Paper } from '../../types/engine';

export interface ReaderItem {
  id: string;
  citeKey: string;
  title: string;
  projectIds: string[];
}

/** The document a lookup was for, and what it found; an answer for another
 * document than the active one is never shown. */
const [found, setFound] = createSignal<{ path: string; item: ReaderItem | null } | null>(null);

function activePath(): string | null {
  const doc = activeTab()?.documentStore.state;
  return doc?.loaded && doc.path ? doc.path : null;
}

export function toReaderItem(paper: Paper | null): ReaderItem | null {
  if (!paper) return null;
  return {
    id: paper.id,
    citeKey: paper.citeKey,
    title: paper.meta.title?.value ?? '',
    projectIds: [...paper.projectIds],
  };
}

let seq = 0;
async function lookUp(path: string | null): Promise<void> {
  const mine = ++seq;
  if (!path) {
    setFound(null);
    return;
  }
  let paper: Paper | null = null;
  try {
    paper = await engine.libraryPaperForPath(path);
  } catch {
    paper = null;
  }
  if (mine === seq) setFound({ path, item: toReaderItem(paper) });
}

let tracking = false;

/** Start following the active document (once per window). */
export function trackReader(): void {
  if (tracking) return;
  tracking = true;
  createRoot(() => {
    createEffect(() => void lookUp(activePath()));
  });
  void engine
    .onLibraryChanged(() => void lookUp(activePath()))
    .catch(() => {
      /* no library events outside the app */
    });
}

/** The active document's library item, when it is one. */
export function readerItem(): ReaderItem | null {
  const current = found();
  return current && current.path === activePath() ? current.item : null;
}

/** For tests: set what the reader shows without asking the engine. */
export function setReaderItemForTest(path: string, item: ReaderItem | null): void {
  setFound({ path, item });
}

/* ---- where a paper was opened from ---- */

const origins = new Map<string, string>();
const [originsVersion, setOriginsVersion] = createSignal(0);

/** Record that `path` was opened from `projectId`'s screen or notepad. */
export function noteOrigin(path: string, projectId: string | null): void {
  if (projectId) origins.set(path, projectId);
  else origins.delete(path);
  setOriginsVersion((v) => v + 1);
}

export function originOf(path: string): string | null {
  originsVersion();
  return origins.get(path) ?? null;
}

/* ---- the choice made for a paper in several projects ---- */

const [choices, setChoices] = createStore<Record<string, string>>({});

export function chooseProjectFor(itemId: string, projectId: string): void {
  setChoices(itemId, projectId);
}

/** The project the reader works in for the active document, or null: see
 * the module comment for the order. Null also when the paper is in several
 * projects and none has been picked yet. */
export function readerProjectId(): string | null {
  const item = readerItem();
  const path = activePath();
  if (!item || !path || item.projectIds.length === 0) return null;
  const origin = originOf(path);
  if (origin && item.projectIds.includes(origin)) return origin;
  if (item.projectIds.length === 1) return item.projectIds[0]!;
  const chosen = choices[item.id];
  return chosen && item.projectIds.includes(chosen) ? chosen : null;
}

/** `documentInProject`: the active document is an item in a project. */
export function documentInProject(): boolean {
  return (readerItem()?.projectIds.length ?? 0) > 0;
}

/** Whether a contribution's `when` holds now. */
export function whenHolds(when: string | null | undefined): boolean {
  if (!when) return true;
  if (when === 'documentInProject') return documentInProject();
  return false;
}

/* ---- asking which project ---- */

export interface ProjectQuestion {
  options: { id: string; name: string }[];
  answer: (id: string | null) => void;
}

const [question, setQuestion] = createSignal<ProjectQuestion | null>(null);
export { question as projectQuestion };

/** Ask once which project, among the active paper's; the answer is kept
 * for the session. Resolves null when dismissed. */
export async function askReaderProject(): Promise<string | null> {
  const item = readerItem();
  if (!item || item.projectIds.length === 0) return null;
  const known = readerProjectId();
  if (known) return known;
  const projects = await engine.libraryProjects().catch(() => []);
  const options = projects
    .filter((p) => item.projectIds.includes(p.id))
    .map((p) => ({ id: p.id, name: p.name }));
  if (options.length === 0) return null;
  if (options.length === 1) return options[0]!.id;
  const answer = await new Promise<string | null>((resolve) => {
    untrack(question)?.answer(null);
    setQuestion({
      options,
      answer: (id) => {
        setQuestion(null);
        resolve(id);
      },
    });
  });
  if (answer) chooseProjectFor(item.id, answer);
  return answer;
}
