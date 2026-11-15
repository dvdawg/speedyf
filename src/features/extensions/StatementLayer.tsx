/** Buttons by the labels of a page's formal statements (theorems, lemmas,
 * definitions...), for extensions that contribute `statementActions`.
 *
 * The same pattern as the equation buttons (`EquationLayer.tsx`): nothing
 * at all unless an enabled extension contributes one; then the document's
 * statements are asked for once the browser is idle (the engine reads them
 * with its hover index, at idle, and caches them), and the pointer is
 * tracked on the page rather than drawn as hit areas, so text selection is
 * untouched. Only the button for the label under the pointer shows, just
 * above the label. */
import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { runStatementAction, statementActions } from './extensionStore';
import { docStatements, topLeft, type Rect, type Statement } from './paperDocs';

interface Props {
  docId: number;
  /** The reader's page index, which is what an extension is told. */
  page: number;
  /** The page in the file, which is what the engine reads. */
  src: number;
  pageHeightPt: number;
  zoom: number;
}

/** Resolve once the browser has a moment. */
function whenIdle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (typeof window.requestIdleCallback === 'function') {
      window.requestIdleCallback(() => resolve(), { timeout: 2_000 });
    } else {
      window.setTimeout(resolve, 200);
    }
  });
}

/** How far above a label the pointer may stray and still reach its button,
 * and how far right of it. */
const REACH_ABOVE_PT = 18;
const REACH_RIGHT_PT = 90;

interface Target {
  statement: Statement;
  labelBox: Rect;
  box: Rect;
}

function union(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null;
  const x0 = Math.min(...rects.map((r) => r[0]));
  const y0 = Math.min(...rects.map((r) => r[1]));
  const x1 = Math.max(...rects.map((r) => r[0] + r[2]));
  const y1 = Math.max(...rects.map((r) => r[1] + r[3]));
  return [x0, y0, x1 - x0, y1 - y0];
}

export default function StatementLayer(props: Props) {
  const actions = createMemo(() => statementActions());
  return (
    <Show when={actions().length > 0}>
      <StatementButtons {...props} />
    </Show>
  );
}

function StatementButtons(props: Props) {
  let root: HTMLDivElement | undefined;
  const [targets] = createResource(
    () => [props.docId, props.src, props.pageHeightPt] as const,
    async ([docId, src, height]) => {
      await whenIdle();
      const all = await docStatements(docId);
      const out: Target[] = [];
      for (const statement of all) {
        if (statement.page !== src || statement.kind === 'equation') continue;
        const labelBox = union(statement.labelRects.map((r) => topLeft(r, height)));
        if (labelBox) out.push({ statement, labelBox, box: topLeft(statement.region, height) });
      }
      return out;
    }
  );
  const [hovered, setHovered] = createSignal<Target | null>(null);

  onMount(() => {
    const space = root?.parentElement;
    if (!space) return;
    const onMove = (event: PointerEvent) => {
      const list = targets();
      if (!list || list.length === 0) return;
      const box = space.getBoundingClientRect();
      const x = (event.clientX - box.left) / props.zoom;
      const y = (event.clientY - box.top) / props.zoom;
      const found = list.find(
        ({ labelBox: [lx, ly, lw, lh] }) =>
          x >= lx &&
          x <= lx + Math.max(lw, REACH_RIGHT_PT) &&
          y >= ly - REACH_ABOVE_PT &&
          y <= ly + lh
      );
      setHovered(found ?? null);
    };
    const onLeave = () => setHovered(null);
    space.addEventListener('pointermove', onMove);
    space.addEventListener('pointerleave', onLeave);
    onCleanup(() => {
      space.removeEventListener('pointermove', onMove);
      space.removeEventListener('pointerleave', onLeave);
    });
  });

  return (
    <div class="statement-layer" ref={root}>
      <Show when={hovered()}>
        {(target) => {
          const [x, y] = target().labelBox;
          const name = () => `${target().statement.word} ${target().statement.number}`;
          return (
            <div
              class="statement-actions"
              style={{ left: `${x * props.zoom}px`, top: `${y * props.zoom}px` }}
            >
              <For each={statementActions()}>
                {(action) => (
                  <button
                    type="button"
                    class="equation-action"
                    aria-label={`${action.title}: ${name()}`}
                    onClick={() =>
                      void runStatementAction(action, {
                        page: props.page,
                        box: target().box,
                        labelBox: target().labelBox,
                        statement: target().statement,
                      })
                    }
                  >
                    {action.title}
                  </button>
                )}
              </For>
            </div>
          );
        }}
      </Show>
    </div>
  );
}
