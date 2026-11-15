/**
 * The hidden print page behind Markup export (`markup-print.html`).
 *
 * Rust opens it in an invisible window, one per export. It fetches the
 * sheets, then for each one: lays it out alone at the page's top-left at its
 * exact size (CSS px == pt), waits until its fonts and images are ready, and
 * asks Rust to capture it (WKWebView createPDF, one vector page per sheet).
 * Nothing here waits on a timer or an animation frame: a hidden web view may
 * not run either.
 */
import 'katex/dist/katex.min.css';
import './print.css';
import { markupPrintApi, type PrintSheet } from '../../lib/transport/markup';

function host(): HTMLElement {
  const el = document.getElementById('sheet');
  if (!el) throw new Error('missing #sheet element');
  return el;
}

/** Show `sheet` alone, at its exact size, at the document's origin. */
export function mountSheet(el: HTMLElement, sheet: PrintSheet): void {
  el.style.width = `${sheet.widthPt}px`;
  el.style.height = `${sheet.heightPt}px`;
  el.innerHTML = sheet.html;
  document.documentElement.style.width = `${sheet.widthPt}px`;
  document.documentElement.style.height = `${sheet.heightPt}px`;
  window.scrollTo(0, 0);
}

/** Resolves once the sheet's layout is settled: fonts loaded, images decoded. */
async function settle(el: HTMLElement): Promise<void> {
  // Reading layout makes the engine resolve styles, which starts the font
  // loads the sheet needs; `fonts.ready` then waits for them.
  void el.getBoundingClientRect();
  await document.fonts.ready;
  const images = Array.from(el.querySelectorAll('img'));
  await Promise.all(images.map((img) => img.decode().catch(() => undefined)));
  void el.getBoundingClientRect();
}

async function run(): Promise<void> {
  const payload = await markupPrintApi.payload();
  if (payload.css) {
    const style = document.createElement('style');
    style.textContent = payload.css;
    document.head.append(style);
  }
  const el = host();
  for (const [index, sheet] of payload.sheets.entries()) {
    mountSheet(el, sheet);
    await settle(el);
    await markupPrintApi.capture(index);
  }
  await markupPrintApi.finish(null);
}

run().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  markupPrintApi.finish(message || 'print page failed').catch(() => undefined);
});
