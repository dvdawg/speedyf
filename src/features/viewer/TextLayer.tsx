/** Transparent text-selection layer: one positioned span per extracted run,
 * horizontally scaled to match the rendered glyph widths (pdf.js technique).
 * Runs arrive in unrotated PDF page space; the parent container handles
 * rotation, so this layer only flips y. */
import { For } from 'solid-js';
import type { TextRunDto } from '../../types/engine';
import { registerCopyRun } from '../../lib/text/copySelection';
import { paragraphRunRange } from '../../lib/text/paragraph';

const measureCache = new Map<string, number>();
let measureCtx: CanvasRenderingContext2D | null = null;

function measureAt100px(text: string): number {
  let w = measureCache.get(text);
  if (w !== undefined) return w;
  if (!measureCtx) {
    measureCtx = document.createElement('canvas').getContext('2d');
    if (!measureCtx) return text.length * 50;
  }
  measureCtx.font = '100px Helvetica, Arial, sans-serif';
  w = measureCtx.measureText(text).width;
  if (measureCache.size > 8000) measureCache.clear();
  measureCache.set(text, w);
  return w;
}

interface Props {
  runs: TextRunDto[];
  pageHeightPt: number;
  zoom: number;
  /** position in the document, so a selection spanning pages can be ordered */
  page: number;
}

/** Select the paragraph holding `span`, a run of `layer`, the way a drag
 * across it would: one DOM range from its first run to its last, so copy,
 * the Quote button and everything else that reads the selection see it. */
export function selectParagraphAt(
  layer: HTMLElement,
  span: Element,
  runs: readonly TextRunDto[]
): boolean {
  const spans = layer.children;
  const index = Array.prototype.indexOf.call(spans, span) as number;
  const range = paragraphRunRange(runs, index);
  const selection = document.getSelection();
  if (!range || !selection) return false;
  const first = spans[range[0]];
  const last = spans[range[1]];
  if (!first || !last) return false;
  const dom = document.createRange();
  dom.setStart(first.firstChild ?? first, 0);
  const end = last.lastChild ?? last;
  dom.setEnd(end, end.nodeType === Node.TEXT_NODE ? (end.textContent ?? '').length : 0);
  selection.removeAllRanges();
  selection.addRange(dom);
  return true;
}

/** Pointer distance, in pixels, within which a click between words still
 * lands on the nearest run. */
const NEAR_PX = 24;

/** The run closest to a click that landed in the gaps between runs. */
function nearestSpan(layer: HTMLElement, x: number, y: number): Element | null {
  let best: Element | null = null;
  let bestDistance = NEAR_PX;
  for (const span of layer.children) {
    const r = span.getBoundingClientRect();
    const dx = Math.max(r.left - x, 0, x - r.right);
    const dy = Math.max(r.top - y, 0, y - r.bottom);
    const distance = Math.hypot(dx, dy);
    if (distance < bestDistance) {
      best = span;
      bestDistance = distance;
    }
  }
  return best;
}

export default function TextLayer(props: Props) {
  return (
    <div
      class="text-layer"
      // A triple-click selects the paragraph, not the whole page the
      // browser would take for one block of absolutely placed spans.
      onMouseDown={(e) => {
        if (e.button !== 0 || e.detail !== 3) return;
        const layer = e.currentTarget;
        const hit = e.target instanceof Element ? e.target.closest('span') : null;
        const span = hit?.parentElement === layer ? hit : nearestSpan(layer, e.clientX, e.clientY);
        if (span && selectParagraphAt(layer, span, props.runs)) e.preventDefault();
      }}
    >
      <For each={props.runs}>
        {(run) => {
          const fontPx = () => Math.max(1, run.h * props.zoom * 0.88);
          const scaleX = () => {
            const natural = (measureAt100px(run.text) * fontPx()) / 100;
            return natural > 0 ? (run.w * props.zoom) / natural : 1;
          };
          return (
            <span
              ref={(el) => registerCopyRun(el, { ...run, page: props.page })}
              style={{
                left: `${run.x * props.zoom}px`,
                top: `${(props.pageHeightPt - run.y - run.h) * props.zoom}px`,
                'font-size': `${fontPx()}px`,
                'line-height': `${run.h * props.zoom}px`,
                transform: `scaleX(${scaleX()})`,
              }}
            >
              {run.text}
            </span>
          );
        }}
      </For>
    </div>
  );
}
