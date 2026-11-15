/** Sending the review back: "Export PDF..." writes one PDF - the paper with
 * its comments marked and a margin of notes beside every page, and the
 * letter at the end - and "Copy as text" puts the letter on the clipboard
 * for an email. */
import { save } from '@tauri-apps/plugin-dialog';
import { activeTab } from '../../stores/tabsStore';
import { flashStatus } from '../../stores/statusMessage';
import { openInNewTabOrFocus } from '../document/tabsController';
import { readerItem } from '../extensions/readerContext';
import { plainMath } from '../../components/mathSplit';
import { buildExportRequest, DEFAULT_MARGIN_PT, type MeasureFn } from './exportBuild';
import { measureBlocks } from './measure';
import { loadMath, mathRenderer } from './mathHtml';
import { longDate, plainLetter } from './model';
import {
  activePath,
  citeIndex,
  ensureLoaded,
  entriesOf,
  flushSaves,
  reviewFor,
  settleLabelSources,
} from './markupStore';
import { markupApi, type MarkupExportResult } from '../../lib/transport/markup';
import type { MarkupTransport } from './markupStore';
import type { MarkupDoc } from './types';

let transport: MarkupTransport = markupApi;
let measure: MeasureFn = measureBlocks;

/** For tests: a fake backend and measurer. */
export function setExportDeps(deps: { transport?: MarkupTransport; measure?: MeasureFn }): void {
  if (deps.transport) transport = deps.transport;
  if (deps.measure) measure = deps.measure;
}

/** "paper.pdf" -> "paper - markup.pdf", beside it. */
export function exportPathFor(source: string): string {
  const slash = Math.max(source.lastIndexOf('/'), source.lastIndexOf('\\'));
  const dir = source.slice(0, slash + 1);
  const name = source.slice(slash + 1);
  const stem = name.replace(/\.pdf$/i, '') || 'paper';
  return `${dir}${stem} - markup.pdf`;
}

/** The paper's title for the letter: the library's, else the file name. */
function paperTitle(): string {
  const doc = activeTab()?.documentStore.state;
  const title = readerItem()?.title.trim();
  if (title) return title;
  return (doc?.name ?? 'the paper').replace(/\.pdf$/i, '');
}

const label = (key: string) => citeIndex().byKey.get(key)?.label ?? null;
const reference = (key: string) => citeIndex().byKey.get(key)?.reference ?? null;

/** The review of the active document, once it has loaded; null (and a
 * word in the status bar) when there is nothing to send. */
function readyReview() {
  const path = activePath();
  if (!path) return null;
  ensureLoaded(path);
  const review = reviewFor(path);
  if (!review || review.status === 'loading') {
    flashStatus('The review is still loading');
    return null;
  }
  if (review.status === 'failed') {
    flashStatus('The saved review could not be read');
    return null;
  }
  if (review.doc.comments.length === 0 && !review.doc.summary.trim()) {
    flashStatus('Nothing to send yet: select text and press C to comment');
    return null;
  }
  return { path, doc: review.doc };
}

/** The display size of a page of the file. */
function pageSize(page: number): { w: number; h: number } | null {
  const doc = activeTab()?.documentStore.state;
  const entry = doc?.pages.find((p) => p.srcIndex === page);
  return entry ? { w: entry.widthPt, h: entry.heightPt } : null;
}

let exporting = false;

/** Lay out `review` of the active document and write it to `dest`. */
export async function exportReview(
  source: string,
  review: MarkupDoc,
  dest: string
): Promise<MarkupExportResult> {
  const doc = activeTab()?.documentStore.state;
  if (!doc) throw new Error('no document is open');
  await Promise.all([loadMath(), settleLabelSources(review.comments)]);
  const request = await buildExportRequest({
    source,
    dest,
    title: paperTitle(),
    date: longDate(new Date()),
    summary: review.summary,
    entries: entriesOf(review),
    sourcePages: doc.searchIndex.total || doc.pages.length,
    pageSize,
    marginPt: DEFAULT_MARGIN_PT,
    math: mathRenderer(),
    cite: label,
    reference,
    measure,
  });
  return transport.exportPdf(request);
}

export async function exportMarkupPdf(): Promise<string | null> {
  if (exporting) return null;
  const review = readyReview();
  const doc = activeTab()?.documentStore.state;
  if (!review || !doc) return null;
  const picked = await save({
    title: 'Export the review as PDF',
    defaultPath: exportPathFor(review.path),
    filters: [{ name: 'PDF', extensions: ['pdf'] }],
  }).catch(() => null);
  if (typeof picked !== 'string') return null;
  const dest = /\.pdf$/i.test(picked) ? picked : `${picked}.pdf`;
  if (dest === review.path) {
    flashStatus('Choose another name: the review never writes over the paper');
    return null;
  }
  exporting = true;
  flashStatus('Exporting the review...', { ms: 60_000 });
  try {
    flushSaves();
    const result = await exportReview(review.path, review.doc, dest);
    const name = result.path.split(/[\\/]/).pop() ?? result.path;
    flashStatus(`Saved ${name}`, {
      action: { label: 'Open', run: () => void openInNewTabOrFocus(result.path) },
    });
    return result.path;
  } catch (err) {
    flashStatus(`Could not export the review: ${err instanceof Error ? err.message : String(err)}`);
    return null;
  } finally {
    exporting = false;
  }
}

/** The review as a plain-text letter. */
export function reviewLetter(): string | null {
  const review = readyReview();
  if (!review) return null;
  return plainLetter({
    title: plainMath(paperTitle()),
    date: longDate(new Date()),
    summary: review.doc.summary,
    entries: entriesOf(review.doc),
    label,
    reference: (key) => {
      const line = reference(key);
      return line ? plainMath(line) : null;
    },
  });
}

export async function copyMarkupText(): Promise<boolean> {
  const letter = reviewLetter();
  if (letter === null) return false;
  try {
    await navigator.clipboard.writeText(letter);
    flashStatus('Copied the review as text');
    return true;
  } catch {
    flashStatus('The clipboard refused the write. Try again.');
    return false;
  }
}
