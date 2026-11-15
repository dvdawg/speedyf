/** Transparent text-selection layer: one positioned span per extracted run,
 * horizontally scaled to match the rendered glyph widths (pdf.js technique).
 * Runs arrive in unrotated PDF page space; the parent container handles
 * rotation, so this layer only flips y.
 *
 * The spans carry the DOM selection, but neither choose it nor show it: a
 * press is hit-tested against the page's lines (`textSelection`), and the
 * selection is painted here as one band per line, under the spans, with
 * the browser's own per-span highlight switched off. */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Index,
  onCleanup,
  onMount,
  untrack,
} from 'solid-js';
import type { TextRunDto } from '../../types/engine';
import type { Rotation } from '../../types/model';
import type { EquationBlock } from '../extensions/EquationLayer';
import { registerCopyRun, setSpanDisplay } from '../../lib/text/copySelection';
import { buildTextModel, selectionRects } from '../../lib/text/textHit';
import { toolState } from '../annotations/toolStore';
import {
  pressTextLayer,
  registerTextLayer,
  selectedPieces,
  type LayerEntry,
} from './textSelection';

export { selectParagraphAt } from './textSelection';

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
  pageWidthPt: number;
  pageHeightPt: number;
  /** total displayed rotation, which the parent applies */
  rotation: Rotation;
  zoom: number;
  /** position in the document, so a selection spanning pages can be ordered */
  page: number;
  /** the page's display equations, boxes from its top-left */
  equations?: readonly EquationBlock[];
}

const measureChar = (text: string) => measureAt100px(text);

export default function TextLayer(props: Props) {
  let layer!: HTMLDivElement;
  const model = createMemo(() =>
    buildTextModel(
      props.runs,
      measureChar,
      (props.equations ?? []).map(({ bbox: [x, y, w, h], number }) => ({
        x,
        y: props.pageHeightPt - y - h,
        w,
        h,
        number,
      }))
    )
  );
  const [painted, setPainted] = createSignal<[number, number, number, number][]>([]);

  const repaint = () => {
    const pieces = selectedPieces(document.getSelection(), layer);
    if (pieces.length === 0 && painted().length === 0) return;
    setPainted(selectionRects(model(), pieces));
  };

  // What a selection's geometry reads: a span of a display equation stands
  // for the whole display. And a selection already there is repainted when
  // the equations arrive.
  createEffect(() => {
    const m = model();
    const spans = layer.children;
    for (let i = 0; i < spans.length; i += 1) {
      const display = m.displays[m.displayOfRun[i] ?? -1];
      setSpanDisplay(spans[i]!, display ? display.box : null);
    }
    untrack(repaint);
  });

  let entry: LayerEntry | undefined;
  onMount(() => {
    entry = {
      layer,
      page: props.page,
      model,
      geom: () => ({
        widthPt: props.pageWidthPt,
        heightPt: props.pageHeightPt,
        rotation: props.rotation,
      }),
      zoom: () => props.zoom,
    };
    const unregister = registerTextLayer(entry);
    document.addEventListener('selectionchange', repaint);
    repaint();
    onCleanup(() => {
      unregister();
      document.removeEventListener('selectionchange', repaint);
    });
  });

  return (
    <>
      <div class="text-selection" aria-hidden="true">
        <Index each={painted()}>
          {(rect) => (
            <div
              style={{
                left: `${rect()[0] * props.zoom}px`,
                top: `${(props.pageHeightPt - rect()[1] - rect()[3]) * props.zoom}px`,
                width: `${rect()[2] * props.zoom}px`,
                height: `${rect()[3] * props.zoom}px`,
              }}
            />
          )}
        </Index>
      </div>
      <div
        ref={layer}
        class="text-layer"
        onMouseDown={(e) => {
          if (entry && toolState.active === 'select') pressTextLayer(entry, e);
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
    </>
  );
}
