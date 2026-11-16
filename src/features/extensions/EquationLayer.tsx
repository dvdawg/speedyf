/** Buttons on a page's display equations, for extensions that contribute
 * `equationActions`, and Markup's Comment while reviewing.
 *
 * Nothing happens here unless an enabled extension contributes one, or a
 * review is on: no
 * request, no listener, no DOM. When one does, the page's equation blocks are
 * asked for once the browser is idle (the engine finds them at idle priority
 * too, and caches them), so scrolling never waits on detection. The blocks are
 * not drawn as hit areas — that would steal text selection from the very
 * formulas someone might want to select — the pointer is tracked on the page
 * instead, and only the button for the block under it is shown. The first
 * button shown in a document tells the equation converter, if one is
 * enabled, to start fetching the paper's exact source.
 */
import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { equationActions, prefetchEquationSource, runEquationAction } from './extensionStore';
import { beginEquationComment, reviewing } from '../markup/markupStore';
import { equationBlocksFor } from './equationBlocks';
import { cssToPdf, type PageGeom } from '../../lib/coordinates/coords';

export { equationBlocksFor, type EquationBlock } from './equationBlocks';

interface Props {
  docId: number;
  /** The reader's page index, which is what an extension is told. */
  page: number;
  /** The page in the file, which is what the engine reads. */
  src: number;
  generation: number;
  zoom: number;
  /** The page's size and how it is turned on screen; unturned without. */
  geom?: PageGeom;
}

/** Resolve once the browser has a moment, so detection never competes with
 * a scroll or a render. */
function whenIdle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (typeof window.requestIdleCallback === 'function') {
      window.requestIdleCallback(() => resolve(), { timeout: 2_000 });
    } else {
      window.setTimeout(resolve, 200);
    }
  });
}

/** How far above a block the pointer may stray and still count as on it: the
 * button sits just above the block's top-right corner. */
const REACH_ABOVE_PT = 18;

export default function EquationLayer(props: Props) {
  const actions = createMemo(() => equationActions());
  return (
    <Show when={actions().length > 0 || reviewing()}>
      <EquationButtons {...props} />
    </Show>
  );
}

function EquationButtons(props: Props) {
  let root: HTMLDivElement | undefined;
  const [blocks] = createResource(
    () => [props.docId, props.src, props.generation] as const,
    async ([docId, src, generation]) => {
      await whenIdle();
      return equationBlocksFor(docId, src, generation);
    }
  );
  const [hovered, setHovered] = createSignal<number | null>(null);

  onMount(() => {
    const space = root?.parentElement;
    if (!space) return;
    const onMove = (event: PointerEvent) => {
      // A drag is a selection being made: no buttons popping up under it.
      // (A press on one of the buttons themselves is a click on it.)
      const onButton = event.target instanceof Node && root?.contains(event.target);
      if (event.buttons !== 0 && !onButton) {
        setHovered(null);
        return;
      }
      const list = blocks();
      if (!list || list.length === 0) return;
      // The layer's box on screen is the page as turned; the blocks are in
      // the page's own space, top-left.
      const box = space.getBoundingClientRect();
      const css = { x: event.clientX - box.left, y: event.clientY - box.top };
      const geom = props.geom;
      let x = css.x / props.zoom;
      let y = css.y / props.zoom;
      if (geom && geom.rotation !== 0) {
        const pt = cssToPdf(css, geom, props.zoom);
        x = pt.x;
        y = geom.heightPt - pt.y;
      }
      const index = list.findIndex(
        ({ bbox: [bx, by, bw, bh] }) =>
          x >= bx && x <= bx + bw && y >= by - REACH_ABOVE_PT && y <= by + bh
      );
      setHovered(index >= 0 ? index : null);
      // A button is about to be clicked, maybe: the converter may start on
      // the paper's source now (once per document; the store dedupes).
      if (index >= 0) prefetchEquationSource(props.docId);
    };
    const onLeave = () => setHovered(null);
    space.addEventListener('pointermove', onMove);
    space.addEventListener('pointerleave', onLeave);
    onCleanup(() => {
      space.removeEventListener('pointermove', onMove);
      space.removeEventListener('pointerleave', onLeave);
    });
  });

  const current = () => {
    const index = hovered();
    const list = blocks();
    return index === null || !list ? null : (list[index] ?? null);
  };

  return (
    <div class="equation-layer" ref={root}>
      <Show when={current()}>
        {(block) => {
          let outline: HTMLDivElement | undefined;
          const [x, y, w, h] = block().bbox;
          return (
            <>
              <div
                ref={outline}
                class="equation-outline"
                aria-hidden="true"
                style={{
                  left: `${x * props.zoom}px`,
                  top: `${y * props.zoom}px`,
                  width: `${w * props.zoom}px`,
                  height: `${h * props.zoom}px`,
                }}
              />
              <div
                class="equation-actions"
                style={{ left: `${(x + w) * props.zoom}px`, top: `${y * props.zoom}px` }}
              >
                <For each={equationActions()}>
                  {(action) => (
                    <button
                      type="button"
                      class="equation-action"
                      aria-label={
                        block().number
                          ? `${action.title} for equation ${block().number}`
                          : action.title
                      }
                      onClick={() => {
                        const rect = (outline ?? root)!.getBoundingClientRect();
                        void runEquationAction(
                          action,
                          { page: props.page, bbox: block().bbox, number: block().number },
                          { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
                        );
                      }}
                    >
                      {action.title}
                    </button>
                  )}
                </For>
                {/* While reviewing (Markup), a comment on the equation as a
                    whole, anchored to its box. */}
                <Show when={reviewing()}>
                  <button
                    type="button"
                    class="equation-action"
                    aria-label={
                      block().number ? `Comment on equation ${block().number}` : 'Comment'
                    }
                    onClick={() =>
                      beginEquationComment({
                        page: props.page,
                        bbox: block().bbox,
                        number: block().number,
                      })
                    }
                  >
                    Comment
                  </button>
                </Show>
              </div>
            </>
          );
        }}
      </Show>
    </div>
  );
}
