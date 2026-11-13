/** Hover spans per page, and the terms contributed per document.
 *
 * A page asks once per document, page and terms version; the answer is
 * cached here (bounded) and in the engine. Contributed terms change the
 * answer, so setting them bumps the document's version and every mounted
 * page asks again. A saved file reopens under a new document id, so nothing
 * from before the save is ever reused. */
import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import type { PreviewSpec } from '../../types/engine';
import type { HoverRect, HoverSpan, PageHovers } from './hoverTypes';

/** A term as the engine takes it: `targetPage` in the file. */
export interface EngineTerm {
  text: string;
  caseSensitive: boolean;
  kind: string | null;
  title: string;
  body: string;
  targetPage: number;
  targetRect: HoverRect | null;
}

const [versions, setVersions] = createSignal<Record<number, number>>({});

/** Bumped whenever a document's terms change. */
export function termsVersion(docId: number): number {
  return versions()[docId] ?? 0;
}

function bump(docId: number): void {
  setVersions((current) => ({ ...current, [docId]: (current[docId] ?? 0) + 1 }));
}

const cache = new Map<string, Promise<HoverSpan[]>>();
const CACHE_LIMIT = 256;

export function spansFor(docId: number, src: number, version: number): Promise<HoverSpan[]> {
  const key = `${docId}:${src}:${version}`;
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  const pending = invoke<PageHovers>('get_page_hovers', { docId, src })
    .then((page) => page.spans)
    .catch(() => {
      cache.delete(key);
      return [] as HoverSpan[];
    });
  cache.set(key, pending);
  while (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
  return pending;
}

/** Documents that have terms in the engine, so a disabled provider can take
 * them back. */
const withTerms = new Set<number>();

export async function setDocumentTerms(docId: number, terms: EngineTerm[]): Promise<number> {
  const kept = await invoke<number>('set_hover_terms', { docId, terms });
  if (terms.length > 0) withTerms.add(docId);
  else withTerms.delete(docId);
  bump(docId);
  return kept;
}

/** Take every contributed term back: the provider was switched off. */
export async function clearAllTerms(): Promise<void> {
  const docs = [...withTerms];
  withTerms.clear();
  await Promise.all(
    docs.map((docId) =>
      invoke('set_hover_terms', { docId, terms: [] })
        .catch(() => undefined)
        .then(() => bump(docId))
    )
  );
}

/** The raster crop a span previews. */
export function hoverPreview(docId: number, span: HoverSpan): Promise<PreviewSpec> {
  return invoke<PreviewSpec>('get_hover_preview', {
    docId,
    src: span.target.page,
    kind: span.kind,
    region: span.target.region,
  });
}
