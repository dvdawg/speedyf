/** Buttons on a page's display equations, for extensions that contribute
 * `equationActions`.
 *
 * Nothing happens here unless an enabled extension contributes one: no
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
import { invoke } from '@tauri-apps/api/core';
import { equationActions, prefetchEquationSource, runEquationAction } from './extensionStore';

export interface EquationBlock {
  /** `[x, y, w, h]`, page points from the displayed page's top-left. */
  bbox: [number, number, number, number];
  number: string | null;
}

interface Props {
  docId: number;
  /** The reader's page index, which is what an extension is told. */
  page: number;
  /** The page in the file, which is what the engine reads. */
  src: number;
  generation: number;
  zoom: number;
}

/** Blocks already asked for, per document page and generation, so a page that
 * scrolls back into view does not ask again. Bounded: a long session reading
 * many documents must not keep every page's answer. */
const cache = new Map<string, Promise<EquationBlock[]>>();
const CACHE_LIMIT = 512;

export function equationBlocksFor(
  docId: number,
  src: number,
  generation: number
): Promise<EquationBlock[]> {
  const key = `${docId}:${src}:${generation}`;
  let hit = cache.get(key);
  if (!hit) {
    hit = invoke<EquationBlock[]>('get_equation_blocks', { docId, src }).catch(() => []);
    if (cache.size >= CACHE_LIMIT) cache.clear();
    cache.set(key, hit);
  }
  return hit;
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
    <Show when={actions().length > 0}>
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
      const list = blocks();
      if (!list || list.length === 0) return;
      const box = space.getBoundingClientRect();
      const x = (event.clientX - box.left) / props.zoom;
      const y = (event.clientY - box.top) / props.zoom;
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
              </div>
            </>
          );
        }}
      </Show>
    </div>
  );
}
