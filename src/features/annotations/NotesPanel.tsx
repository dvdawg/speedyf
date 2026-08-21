/** Every annotation in the document, in reading order, with what it sits on.
 *
 * The panel that makes annotating worth doing. Before this, marking up a paper
 * was write-only: the highlights were on the page and there was no way to see
 * what you had marked short of scrolling the whole thing again.
 *
 * Highlights show the words underneath them, which the PDF does not store — a
 * highlight is rectangles, not text. Recovering the words means asking for the
 * page's text layout and matching it against those rectangles, so it is done
 * lazily, per page, and only for pages that actually carry a highlight. */
import { createEffect, createMemo, createSignal, For, Show, useContext } from 'solid-js';
import { createStore } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import { TabContext } from '../../app/TabContext';
import { jumpToAnchor } from '../outline/jumpToAnchor';
import { runsUnderQuads } from './annotationText';
import { annotationsToMarkdown, type ExportEntry } from './annotationExport';
import { smartCopyText } from '../../lib/text/smartCopy';
import type { Annotation } from '../../types/model';

/** How much of a highlight's text to show before trailing off. */
const PREVIEW_CHARS = 220;

const KIND_LABEL: Record<string, string> = {
  highlight: 'Highlight',
  ink: 'Drawing',
  rect: 'Box',
  note: 'Note',
  textbox: 'Text',
  image: 'Image',
};

export default function NotesPanel() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;

  /** Recovered highlight text, by annotation id. Filled in as pages are read. */
  const [quotes, setQuotes] = createStore<Record<string, string>>({});
  const [copied, setCopied] = createSignal(false);
  const requested = new Set<number>();

  /** Every annotation with the page it is on, in reading order. */
  const rows = createMemo(() => {
    const out: { annot: Annotation; page: number; srcPage: number }[] = [];
    doc.pages.forEach((page, index) => {
      for (const annot of doc.annotations[page.id] ?? []) {
        out.push({ annot, page: index + 1, srcPage: page.srcIndex ?? index });
      }
    });
    return out;
  });

  /** Ask for one page's text and resolve every highlight on it.
   *
   * Once per page, not once per highlight: the text layout is the expensive
   * part and a page usually carries several. */
  const resolvePage = async (srcPage: number) => {
    if (requested.has(srcPage)) return;
    requested.add(srcPage);
    const docId = doc.docId;
    let layout;
    try {
      layout = await engine.getTextLayout(docId, srcPage);
    } catch {
      return; // the page will simply show its kind instead of its words
    }
    if (doc.docId !== docId) return;
    for (const row of rows()) {
      if (row.srcPage !== srcPage || row.annot.kind !== 'highlight') continue;
      const runs = runsUnderQuads(row.annot.quads ?? [], layout.runs);
      if (runs.length === 0) continue;
      setQuotes(row.annot.id, smartCopyText(runs.map((run) => ({ ...run, page: srcPage }))));
    }
  };

  // Kick off resolution for every page that carries a highlight. Pages without
  // one never cost a text extraction.
  createEffect(() => {
    const pages = new Set(
      rows()
        .filter((row) => row.annot.kind === 'highlight')
        .map((row) => row.srcPage)
    );
    for (const page of pages) void resolvePage(page);
  });

  const entries = (): ExportEntry[] =>
    rows().map((row) => ({
      annot: row.annot,
      page: row.page,
      ...(quotes[row.annot.id] ? { quoted: quotes[row.annot.id] } : {}),
    }));

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(annotationsToMarkdown(doc.name, entries()));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  const preview = (annot: Annotation) => {
    const quote = quotes[annot.id];
    if (!quote) return null;
    return quote.length > PREVIEW_CHARS ? `${quote.slice(0, PREVIEW_CHARS).trimEnd()}…` : quote;
  };

  return (
    <div class="sidebar-scroll notes-panel" aria-label="Notes and annotations">
      <Show
        when={rows().length > 0}
        fallback={
          <div class="panel-note">
            Nothing marked up yet. Highlights, notes and drawings you add show up here.
          </div>
        }
      >
        <div class="notes-head">
          <span>
            {rows().length} {rows().length === 1 ? 'annotation' : 'annotations'}
          </span>
          <button type="button" class="secondary-btn" onClick={() => void copyAll()}>
            {copied() ? 'Copied' : 'Copy all'}
          </button>
        </div>
        <For each={rows()}>
          {(row) => (
            <button
              type="button"
              class="note-row"
              onClick={() => jumpToAnchor(tab.viewport, doc, row.srcPage, row.annot.rect.y)}
            >
              <span
                class="note-swatch"
                style={{ background: row.annot.color }}
                aria-hidden="true"
              />
              <span class="note-body">
                <span class="note-meta">
                  {KIND_LABEL[row.annot.kind] ?? 'Annotation'} · p{row.page}
                </span>
                <Show when={preview(row.annot)}>
                  <span class="note-quote">{preview(row.annot)}</span>
                </Show>
                <Show when={row.annot.text?.trim()}>
                  <span class="note-text">{row.annot.text}</span>
                </Show>
              </span>
            </button>
          )}
        </For>
      </Show>
    </div>
  );
}
