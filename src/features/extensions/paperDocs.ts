/** What the reader knows about a library item's document, for a notepad:
 * its formal statements (theorems, lemmas, definitions...) and numbered
 * equations, and pictures of places in it - whether or not it is open.
 *
 * An item open in a tab is read through that tab's document. Any other is
 * opened in the engine in the background, a few kept at once, the oldest
 * closed. Statements come from the engine's hover index (`engine::hovers`),
 * cached there by content fingerprint, so a paper read once answers again
 * at once; here they are kept per document for the session. */
import { invoke } from '@tauri-apps/api/core';
import { engine } from '../../lib/transport/engine';
import { renderUrl } from '../../lib/rendering/renderSource';
import { pagesInverted } from '../../stores/settings';
import { findTabByPath } from '../../stores/tabsStore';
import type { PreviewSpec } from '../../types/engine';
import { pathFor } from '../projects/openPaper';

export type Rect = [number, number, number, number];

/** A formal statement or numbered equation, as the engine found it.
 * Rectangles are y-up page points of the file's page `page`. */
export interface Statement {
  /** "theorem", "lemma", "proposition", ..., or "equation". */
  kind: string;
  /** "Proposition", "Equation". */
  word: string;
  /** "2.2", "A.3", "12". */
  number: string;
  title: string | null;
  page: number;
  labelRects: Rect[];
  region: Rect;
  /** The statement as printed, label included; empty for an equation. */
  text: string;
}

/** An item's document in the engine. */
export interface PaperDoc {
  docId: number;
  generation: number;
  /** Height in points of a page of the file, when known. */
  pageHeight(page: number): number | null;
}

/** Documents opened in the background, oldest first. */
const background: { path: string; docId: number; heights: number[] }[] = [];
const BACKGROUND_LIMIT = 3;
const opening = new Map<string, Promise<PaperDoc | null>>();

/** Close the oldest background documents past the limit. */
function trimBackground(): void {
  while (background.length > BACKGROUND_LIMIT) {
    const old = background.shift()!;
    statementCache.delete(old.docId);
    void engine.close(old.docId).catch(() => {});
  }
}

/** The document at `path`: its tab's, else one opened in the background. */
export function docForPath(path: string): Promise<PaperDoc | null> {
  const tab = findTabByPath(path);
  const doc = tab?.documentStore.state;
  if (tab && doc?.loaded) {
    return Promise.resolve({
      docId: doc.docId,
      generation: doc.generation,
      pageHeight: (page) => {
        const index = doc.pages.findIndex((p) => p.srcIndex === page);
        return tab.zoom.pagesGeom()[index]?.heightPt ?? null;
      },
    });
  }
  const kept = background.find((b) => b.path === path);
  if (kept) {
    // Most recently used last.
    background.splice(background.indexOf(kept), 1);
    background.push(kept);
    return Promise.resolve(backgroundDoc(kept));
  }
  let pending = opening.get(path);
  if (!pending) {
    pending = engine
      .open(path)
      .then((meta) => {
        const entry = { path, docId: meta.docId, heights: meta.sizes.map((s) => s[1]) };
        background.push(entry);
        trimBackground();
        return backgroundDoc(entry);
      })
      .catch(() => null)
      .finally(() => opening.delete(path));
    opening.set(path, pending);
  }
  return pending;
}

function backgroundDoc(entry: { docId: number; heights: number[] }): PaperDoc {
  return {
    docId: entry.docId,
    generation: 0,
    pageHeight: (page) => entry.heights[page] ?? null,
  };
}

/** Library item id -> its file's path, for the session. */
const paths = new Map<string, Promise<string | null>>();

export function pathForItem(itemId: string): Promise<string | null> {
  let hit = paths.get(itemId);
  if (!hit) {
    hit = engine
      .libraryPaper(itemId)
      .then(pathFor)
      .catch(() => null);
    paths.set(itemId, hit);
    if (paths.size > 500) paths.clear();
  }
  return hit;
}

/** An item's document, opened if need be. */
export async function docForItem(itemId: string): Promise<PaperDoc | null> {
  const path = await pathForItem(itemId);
  return path ? docForPath(path) : null;
}

const statementCache = new Map<number, Promise<Statement[]>>();

/** A document's statements and numbered equations, in document order. */
export function docStatements(docId: number): Promise<Statement[]> {
  let hit = statementCache.get(docId);
  if (!hit) {
    hit = invoke<Statement[]>('get_doc_statements', { docId }).catch(() => {
      statementCache.delete(docId);
      return [];
    });
    statementCache.set(docId, hit);
    if (statementCache.size > 64) statementCache.delete(statementCache.keys().next().value!);
  }
  return hit;
}

/** A library item's statements, its document opened if need be. */
export async function itemStatements(itemId: string): Promise<Statement[]> {
  const doc = await docForItem(itemId);
  return doc ? docStatements(doc.docId) : [];
}

/** A y-up rectangle of a page `height` points tall, top-left. */
export function topLeft(rect: Rect, height: number): Rect {
  return [rect[0], height - (rect[1] + rect[3]), rect[2], rect[3]];
}

/** A top-left box on a page `height` points tall, y-up. */
export function yUp(box: Rect, height: number): Rect {
  return [box[0], height - (box[1] + box[3]), box[2], box[3]];
}

/** A picture of `box` (top-left page points) on page `page` of an item's
 * file: the URL of the raster and its size in CSS pixels at preview
 * scale. Null when the item has no file or the place is off its pages. */
export async function placePreview(
  itemId: string,
  page: number,
  box: Rect,
  kind = 'theorem'
): Promise<{ url: string; width: number; height: number } | null> {
  const doc = await docForItem(itemId);
  if (!doc) return null;
  const height = doc.pageHeight(page);
  if (height === null) return null;
  try {
    const spec = await invoke<PreviewSpec>('get_hover_preview', {
      docId: doc.docId,
      src: page,
      kind,
      region: yUp(box, height),
    });
    const scale = spec.scaleMilli / 1000;
    return {
      url: renderUrl({
        docId: spec.docId,
        srcIndex: spec.src,
        rotation: 0,
        scaleMilli: spec.scaleMilli,
        generation: doc.generation,
        kind: 'preview',
        tile: spec.tile,
        invert: pagesInverted(),
      }),
      width: spec.tile.w / scale,
      height: spec.tile.h / scale,
    };
  } catch {
    return null;
  }
}

/** For tests: forget everything kept. */
export function resetPaperDocs(): void {
  background.length = 0;
  opening.clear();
  paths.clear();
  statementCache.clear();
}

/** A statement with where it is: its box top-left on its page. */
export interface StatementPlace {
  statement: Statement;
  /** Top-left page points, rounded. */
  box: Rect;
}

/** A library item's statements and numbered equations with their boxes,
 * its document opened if need be: what the `@` picker offers. */
export async function statementPlaces(itemId: string): Promise<StatementPlace[]> {
  const doc = await docForItem(itemId);
  if (!doc) return [];
  const statements = await docStatements(doc.docId);
  const out: StatementPlace[] = [];
  for (const statement of statements) {
    const height = doc.pageHeight(statement.page);
    if (height === null) continue;
    out.push({
      statement,
      box: topLeft(statement.region, height).map((v) => Math.round(v)) as Rect,
    });
  }
  return out;
}
