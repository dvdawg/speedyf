/** Measuring export blocks the way the print window will set them: in a
 * hidden shadow root holding only print.css and KaTeX's stylesheet, so
 * nothing of the app's own styles leaks in, at the column width the block
 * will have, after the fonts have loaded. CSS px == pt throughout. */
import katexCss from 'katex/dist/katex.min.css?inline';
import printCss from './print.css?inline';
import { loadKatex } from '../../components/MathText';
import type { MeasureItem, Measured } from './exportBuild';

/** Wait for the fonts in use to be ready (KaTeX's arrive on first use). */
async function fontsReady(): Promise<void> {
  const fonts = (document as Document & { fonts?: { ready: Promise<unknown> } }).fonts;
  if (fonts) await fonts.ready.catch(() => undefined);
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
}

export async function measureBlocks(items: MeasureItem[]): Promise<Measured[]> {
  if (items.length === 0) return [];
  await loadKatex();
  const host = document.createElement('div');
  host.setAttribute('aria-hidden', 'true');
  host.style.cssText =
    'position:fixed;left:-100000px;top:0;width:0;height:0;overflow:hidden;visibility:hidden;contain:layout style;';
  document.body.appendChild(host);
  try {
    const root = host.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = `${katexCss}\n${printCss}`;
    root.appendChild(style);
    const slots = items.map((item) => {
      const sheet = document.createElement('div');
      sheet.className = `mk-sheet mk-${item.sheet} mk-measuring`;
      sheet.style.cssText = `position:absolute;left:0;top:0;width:${item.width + 400}px;height:auto;`;
      const flow = document.createElement('div');
      flow.className = 'mk-flow';
      flow.style.cssText = `position:relative;left:0;top:0;width:${item.width}px;`;
      // The block markup is ours, built from escaped text and KaTeX output.
      flow.innerHTML = item.html;
      sheet.appendChild(flow);
      root.appendChild(sheet);
      return flow;
    });
    await fontsReady();
    return slots.map((flow) => {
      const block = flow.firstElementChild as HTMLElement | null;
      const box = (block ?? flow).getBoundingClientRect();
      const links = [...flow.querySelectorAll<HTMLElement>('[data-link]')].map((el) => {
        const r = el.getBoundingClientRect();
        return {
          key: el.dataset.link ?? '',
          x: r.left - box.left,
          y: r.top - box.top,
          w: r.width,
          h: r.height,
        };
      });
      return { height: Math.ceil(box.height * 100) / 100, links };
    });
  } finally {
    host.remove();
  }
}
