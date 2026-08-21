/** The print flow: prepare, preview, submit, clean up.
 *
 * Printing goes through a temp PDF built by the engine rather than the file on
 * disk, because the file on disk is not what the user is looking at — it has
 * none of this session's edits. That temp document is then opened like any
 * other, so the dialog can preview the exact bytes that will reach the printer.
 *
 * The one hard rule here is that the temp document and its file are always
 * released: on print, on cancel, on error, and when the tab it came from goes
 * away. */
import { untrack } from 'solid-js';
import { createStore } from 'solid-js/store';
import { engine, isEngineError } from '../../lib/transport/engine';
import type { PrintOption, Printer } from '../../types/engine';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { showError } from '../../stores/modalStore';
import type { TabRecord } from '../../stores/tabsStore';
import { limitToRange, withoutAnnotations } from './printPlan';

interface PrintState {
  /** true from the moment the dialog is asked for until it closes */
  open: boolean;
  /** the temp document being previewed, or -1 before it is ready */
  previewDocId: number;
  previewGeneration: number;
  /** path of the temp PDF, kept so it can be printed and then deleted */
  path: string | null;
  pageCount: number;
  /** the document this job came from, for staleness checks */
  sourceDocId: number;
  preparing: boolean;
  submitting: boolean;
  /** whether the previewed PDF carries this session's markup */
  includeAnnotations: boolean;
  /** true when the document has markup to include in the first place */
  hasAnnotations: boolean;
  /** the tab the job came from, so the plan can be rebuilt on a toggle */
  source: TabRecord | null;
  printers: Printer[];
  options: PrintOption[];
}

const EMPTY: PrintState = {
  open: false,
  previewDocId: -1,
  previewGeneration: 0,
  path: null,
  pageCount: 0,
  sourceDocId: -1,
  preparing: false,
  submitting: false,
  includeAnnotations: true,
  hasAnnotations: false,
  source: null,
  printers: [],
  options: [],
};

const [state, setState] = createStore<PrintState>({ ...EMPTY });
export { state as printState };

/** Release the temp document and its file. Safe to call more than once —
 * cancel-then-close and print-then-close both land here. */
async function release() {
  // A snapshot on purpose: this runs once, to let go of what the dialog was
  // holding. Tracking it would mean re-releasing on every unrelated change.
  const previewDocId = untrack(() => state.previewDocId);
  const path = untrack(() => state.path);
  setState({ previewDocId: -1, path: null, pageCount: 0 });
  if (previewDocId >= 0) {
    try {
      await engine.close(previewDocId);
    } catch {
      /* the engine is already rid of it; nothing left to do */
    }
  }
  if (path) {
    try {
      await engine.discardPrintPdf(path);
    } catch {
      /* a stray temp file is swept at next startup */
    }
  }
}

/** Build the print PDF for the current settings and preview it.
 *
 * Rebuilt rather than filtered on screen when annotations are toggled: the
 * preview's whole claim is that it shows the bytes that will print, and a
 * preview that only approximates them would be worth less than none. */
async function prepare(tab: TabRecord, includeAnnotations: boolean): Promise<void> {
  const doc = tab.documentStore.state;
  const sourceDocId = doc.docId;
  setState({ preparing: true });
  const previous = untrack(() => ({ id: state.previewDocId, path: state.path }));

  try {
    const full = tab.documentStore.buildEditPlan();
    const plan = includeAnnotations
      ? full
      : withoutAnnotations(full, tab.documentStore.ownedSrcAnnots());
    const path = await engine.buildPrintPdf(sourceDocId, plan);
    if (!state.open || tab.documentStore.state.docId !== sourceDocId) {
      await engine.discardPrintPdf(path).catch(() => undefined);
      return;
    }
    const meta = await engine.open(path);
    if (!state.open) {
      setState({ previewDocId: meta.docId, path });
      await release();
      return;
    }
    // Only let go of the old preview once the new one is in hand, so the
    // dialog never blinks through an empty state.
    if (previous.id >= 0) await engine.close(previous.id).catch(() => undefined);
    if (previous.path) await engine.discardPrintPdf(previous.path).catch(() => undefined);
    setState({
      previewDocId: meta.docId,
      // A freshly opened document is at generation 0, and nothing ever bumps
      // this one — it exists only to be looked at and then thrown away.
      previewGeneration: 0,
      path,
      pageCount: meta.pageCount,
      preparing: false,
    });
  } catch (e) {
    setState({ preparing: false });
    showError(
      isEngineError(e)
        ? `Could not prepare the document for printing: ${e.message}`
        : 'Could not prepare the document for printing.'
    );
  }
}

/** Open the print dialog for `tab`. */
export async function beginPrint(tab: TabRecord): Promise<void> {
  const doc = tab.documentStore.state;
  if (!doc.loaded || state.open) return;
  if (doc.pages.length === 0) {
    showError('There are no pages to print.');
    return;
  }

  const plan = tab.documentStore.buildEditPlan();
  const hasAnnotations = plan.pages.some(
    (page) => page.annots.length > 0 || page.texts.length > 0 || page.images.length > 0
  );
  setState({
    ...EMPTY,
    open: true,
    preparing: true,
    sourceDocId: doc.docId,
    source: tab,
    hasAnnotations,
  });
  await prepare(tab, true);
  if (state.open) void loadPrinters();
}

/** Rebuild the job with or without this session's markup. */
export async function setIncludeAnnotations(include: boolean): Promise<void> {
  const tab = untrack(() => state.source);
  if (!tab || state.preparing || include === state.includeAnnotations) return;
  setState('includeAnnotations', include);
  await prepare(tab, include);
}

async function loadPrinters() {
  try {
    const printers = await engine.listPrinters();
    if (!state.open) return;
    setState('printers', printers);
    const chosen = printers.find((p) => p.isDefault) ?? printers[0];
    if (chosen) await loadOptions(chosen.name);
  } catch {
    if (state.open) setState('printers', []);
  }
}

/** Reload the option list for a printer — duplex and paper differ per queue. */
export async function loadOptions(printer: string): Promise<void> {
  try {
    const options = await engine.printerOptions(printer);
    if (state.open) setState('options', options);
  } catch {
    if (state.open) setState('options', []);
  }
}

export async function submitPrint(
  printer: string,
  copies: number,
  range: string | null,
  options: [string, string][]
): Promise<void> {
  const path = state.path;
  if (!path || state.submitting) return;
  setState('submitting', true);
  try {
    await engine.submitPrint({ printer, copies, range, options }, path);
    await closePrint();
  } catch (e) {
    setState('submitting', false);
    showError(isEngineError(e) ? `The printer refused the job: ${e.message}` : 'Could not print.');
  }
}

/** "Print to PDF": keep the file we were going to print.
 *
 * A page range has to be applied here rather than left to the printer — a
 * saved file has no printer to do the cutting — so a narrowed job is rebuilt
 * before it is copied out. */
export async function exportAsPdf(range: string | null): Promise<void> {
  const tab = untrack(() => state.source);
  const path = untrack(() => state.path);
  if (!tab || !path || state.submitting) return;

  const suggested = tab.documentStore.state.name.replace(/\.pdf$/i, '') || 'document';
  const dest = await saveDialog({
    defaultPath: `${suggested}.pdf`,
    filters: [{ name: 'PDF documents', extensions: ['pdf'] }],
  });
  if (!dest) return;

  setState('submitting', true);
  try {
    if (range === null) {
      await engine.exportPrintPdf(path, dest);
    } else {
      // Rebuild with only the chosen pages, then hand that over instead.
      const full = tab.documentStore.buildEditPlan();
      const base = state.includeAnnotations
        ? full
        : withoutAnnotations(full, tab.documentStore.ownedSrcAnnots());
      const narrowed = limitToRange(base, range);
      const temp = await engine.buildPrintPdf(state.sourceDocId, narrowed);
      try {
        await engine.exportPrintPdf(temp, dest);
      } finally {
        await engine.discardPrintPdf(temp).catch(() => undefined);
      }
    }
    await closePrint();
  } catch (e) {
    setState('submitting', false);
    showError(
      isEngineError(e) ? `Could not save the PDF: ${e.message}` : 'Could not save the PDF.'
    );
  }
}

export async function closePrint(): Promise<void> {
  if (!state.open) return;
  setState({ open: false, preparing: false, submitting: false });
  await release();
  setState({ ...EMPTY });
}

/** Called when a document goes away underneath an open dialog. */
export function abandonPrintFor(docId: number): void {
  if (state.open && state.sourceDocId === docId) void closePrint();
}
