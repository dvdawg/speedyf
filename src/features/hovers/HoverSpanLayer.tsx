/** Hover previews for text the engine recognised on a page: cross-references
 * ("Eq. (12)", "Fig. 3", "Theorem 2.1"), footnote marks, and terms an
 * extension contributed (Notation's acronyms).
 *
 * Nothing is drawn on the page - no underline, no tint. The spans are
 * tracked from the pointer's position over the page instead of being laid
 * over the text as hit areas, so selecting text, drawing an annotation or
 * filling a form field works exactly as it does without them: a press on a
 * span still starts a selection. Over one, the cursor turns into a pointer
 * and, after the same dwell as a link, the shared preview popover opens. A
 * click without a drag follows it, through the same path as a link, so Back
 * returns.
 *
 * For keyboards and screen readers each span also has a labelled button,
 * invisible and out of the pointer's way, that previews on focus and follows
 * on Enter. Positions are hit-tested against those buttons' own boxes, which
 * follow the page through zoom and rotation with no arithmetic here. */
import { createResource, For, onCleanup, onMount, useContext } from 'solid-js';
import { TabContext } from '../../app/TabContext';
import { toolState } from '../annotations/toolStore';
import { followSpan } from './followSpan';
import { spansFor, termsVersion } from './hoverStore';
import type { HoverSpan } from './hoverTypes';

interface Props {
  docId: number;
  src: number;
  pageHeightPt: number;
  zoom: number;
}

/** Elements over the page that own the pointer: links, form widgets,
 * annotations, equation buttons. Over one of them a span stays quiet. */
const OWNED =
  '.citation-hotspot, .form-widget, .annot-svg *, .annot-textbox, .annot-note, .equation-actions, .note-popover, input, textarea, select';

/** How far a press may travel and still be a click rather than a drag. */
const CLICK_SLOP_PX = 4;

function ariaLabel(span: HoverSpan): string {
  if (span.kind === 'term') return `Definition of ${span.label}`;
  return `Preview ${span.label}`;
}

/** Resolve once the browser has a moment, so asking never competes with a
 * scroll or a render. */
function whenIdle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (typeof window.requestIdleCallback === 'function') {
      window.requestIdleCallback(() => resolve(), { timeout: 1_500 });
    } else {
      window.setTimeout(resolve, 150);
    }
  });
}

export default function HoverSpanLayer(props: Props) {
  const tab = useContext(TabContext)!;
  const { citationStore } = tab;
  const [spans] = createResource(
    () => [props.docId, props.src, termsVersion(props.docId)] as const,
    async ([docId, src, version]) => {
      await whenIdle();
      return spansFor(docId, src, version);
    }
  );
  let root: HTMLDivElement | undefined;
  /** Each span's buttons, one per rectangle. */
  const targets = new Map<number, HTMLElement[]>();

  onMount(() => {
    const space = root?.parentElement;
    if (!space) return;
    let current: number | null = null;
    let press: { x: number; y: number; hit: number | null } | null = null;

    const hitAt = (x: number, y: number): number | null => {
      for (const [index, elements] of targets) {
        for (const element of elements) {
          const r = element.getBoundingClientRect();
          if (x >= r.left - 1 && x <= r.right + 1 && y >= r.top - 1 && y <= r.bottom + 1) {
            return index;
          }
        }
      }
      return null;
    };
    const owned = (event: Event) =>
      toolState.active !== 'select' ||
      (event.target instanceof Element && event.target.closest(OWNED) !== null);

    const setCurrent = (next: number | null) => {
      if (next === current) return;
      const list = spans() ?? [];
      if (current !== null) citationStore.leave();
      current = next;
      space.classList.toggle('is-over-hover-span', next !== null);
      const span = next === null ? undefined : list[next];
      const anchor = next === null ? undefined : targets.get(next)?.[0];
      if (span && anchor)
        citationStore.enter(props.docId, props.src, { kind: 'span', span }, anchor);
    };

    const onMove = (event: PointerEvent) => {
      // A drag is a selection, or a stroke: no previews while it lasts.
      if (event.buttons !== 0 || owned(event) || !spans()?.length) {
        setCurrent(null);
        return;
      }
      setCurrent(hitAt(event.clientX, event.clientY));
    };
    const onLeave = () => setCurrent(null);
    const onDown = (event: PointerEvent) => {
      press =
        event.button === 0 && !owned(event)
          ? { x: event.clientX, y: event.clientY, hit: hitAt(event.clientX, event.clientY) }
          : null;
    };
    const onClick = (event: MouseEvent) => {
      const started = press;
      press = null;
      if (!started || started.hit === null || owned(event)) return;
      if (event.button !== 0 || event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      const moved = Math.hypot(event.clientX - started.x, event.clientY - started.y);
      if (moved > CLICK_SLOP_PX) return;
      if (hitAt(event.clientX, event.clientY) !== started.hit) return;
      // A click that made a selection (a double click on a word) is about
      // the text, not the reference.
      const selection = window.getSelection();
      if (selection && !selection.isCollapsed) return;
      const span = spans()?.[started.hit];
      if (span) followSpan(tab, span);
    };

    space.addEventListener('pointermove', onMove);
    space.addEventListener('pointerleave', onLeave);
    space.addEventListener('pointerdown', onDown);
    space.addEventListener('click', onClick);
    onCleanup(() => {
      space.removeEventListener('pointermove', onMove);
      space.removeEventListener('pointerleave', onLeave);
      space.removeEventListener('pointerdown', onDown);
      space.removeEventListener('click', onClick);
      space.classList.remove('is-over-hover-span');
      if (current !== null) citationStore.leave();
    });
  });

  return (
    <div class="hover-span-layer" ref={root}>
      <For each={spans() ?? []}>
        {(span, index) => {
          const elements: HTMLElement[] = [];
          onCleanup(() => targets.delete(index()));
          return (
            <For each={span.rects}>
              {([x, y, w, h], rect) => (
                <button
                  type="button"
                  class="hover-span-target"
                  ref={(element) => {
                    elements.push(element);
                    targets.set(index(), elements);
                  }}
                  tabIndex={rect() === 0 ? 0 : -1}
                  aria-hidden={rect() === 0 ? undefined : 'true'}
                  aria-label={ariaLabel(span)}
                  style={{
                    left: `${x * props.zoom}px`,
                    top: `${(props.pageHeightPt - y - h) * props.zoom}px`,
                    width: `${Math.max(2, w * props.zoom)}px`,
                    height: `${Math.max(2, h * props.zoom)}px`,
                  }}
                  onFocus={(event) =>
                    citationStore.enter(
                      props.docId,
                      props.src,
                      { kind: 'span', span },
                      event.currentTarget,
                      true
                    )
                  }
                  onBlur={() => citationStore.leave()}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape') {
                      event.preventDefault();
                      citationStore.close();
                      event.currentTarget.blur();
                    }
                  }}
                  onClick={() => followSpan(tab, span)}
                />
              )}
            </For>
          );
        }}
      </For>
    </div>
  );
}
