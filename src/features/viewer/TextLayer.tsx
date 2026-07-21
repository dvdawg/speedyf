/** Transparent text-selection layer: one positioned span per extracted run,
 * horizontally scaled to match the rendered glyph widths (pdf.js technique).
 * Runs arrive in unrotated PDF page space; the parent container handles
 * rotation, so this layer only flips y. */
import { For } from 'solid-js';
import type { TextRunDto } from '../../types/engine';

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
}

export default function TextLayer(props: Props) {
  return (
    <div class="text-layer">
      <For each={props.runs}>
        {(run) => {
          const fontPx = () => Math.max(1, run.h * props.zoom * 0.88);
          const scaleX = () => {
            const natural = (measureAt100px(run.text) * fontPx()) / 100;
            return natural > 0 ? (run.w * props.zoom) / natural : 1;
          };
          return (
            <span
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
