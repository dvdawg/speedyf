/** A small text button by a page selection, for the extensions that act on
 * one (`selectionActions`): "Quote" for the Notepad's Quote to Notepad.
 *
 * Nothing is here unless an enabled extension offers an action whose `when`
 * holds now: no listener, no DOM. When one does, the button appears just past
 * the end of the selection once the pointer is let go, and goes away on any
 * scroll, a new or collapsed selection, Escape, the next press, and while an
 * annotation tool other than select is out. Pressing it neither moves focus
 * nor clears the selection: it runs the action with the selected text,
 * exactly as the palette does.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { formatShortcut } from '../../app/shortcutLabel';
import { selectedText, settleSelectionNow } from '../../stores/selectionStore';
import { toolState } from '../annotations/toolStore';
import { canonicalChord } from './capabilities';
import {
  extensionBindings,
  runExtensionAction,
  selectionActions,
  type ExtensionAction,
} from './extensionStore';
import { placeMathPopover } from './mathPopoverStore';

/** At most this many buttons; more would be a menu, which the palette is. */
const MAX_ACTIONS = 3;

/** Size used to place the row before it has been measured. */
const GUESS = { width: 64, height: 22 };

/** A selection in these is someone typing, not reading. */
const FIELD = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';

/** What each button reads: the first word of its title ("Quote" for
 * "Quote to Notepad") when that tells the row's actions apart, else the
 * whole title. */
export function shortLabels(titles: readonly string[]): string[] {
  const firsts = titles.map((title) => title.trim().split(/\s+/)[0] ?? title);
  const unique = new Set(firsts).size === firsts.length;
  return unique ? firsts : titles.map((title) => title.trim());
}

export default function SelectionActions() {
  const actions = createMemo(() => selectionActions().slice(0, MAX_ACTIONS));
  return (
    <Show when={actions().length > 0}>
      <SelectionButtons actions={actions()} />
    </Show>
  );
}

interface Shown {
  left: number;
  top: number;
  /** The range the button was placed for; any other selection hides it. */
  range: Range;
}

function elementOf(node: Node): Element | null {
  return node instanceof Element ? node : node.parentElement;
}

/** The selection on a page, if there is one worth acting on. */
function pageRange(): Range | null {
  const selection = document.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  const common = elementOf(range.commonAncestorContainer);
  const start = elementOf(range.startContainer);
  if (!common?.closest('.viewer') || !start?.closest('.text-layer')) return null;
  if (common.closest(FIELD)) return null;
  const focus = document.activeElement;
  if (focus instanceof Element && focus !== document.body && focus.closest(FIELD)) return null;
  if (range.toString().trim() === '') return null;
  return range;
}

/** The selection's last line on screen: where the button goes. */
function endRect(range: Range): { left: number; top: number; right: number; bottom: number } {
  const rects =
    typeof range.getClientRects === 'function'
      ? Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0)
      : [];
  const last = rects.at(-1) ?? range.getBoundingClientRect();
  return { left: last.right, right: last.right, top: last.top, bottom: last.bottom };
}

function sameRange(a: Range, b: Range): boolean {
  return (
    a.startContainer === b.startContainer &&
    a.startOffset === b.startOffset &&
    a.endContainer === b.endContainer &&
    a.endOffset === b.endOffset
  );
}

function SelectionButtons(props: { actions: ExtensionAction[] }) {
  let row: HTMLDivElement | undefined;
  const [shown, setShown] = createSignal<Shown | null>(null);
  let dragging = false;
  let showTimer: ReturnType<typeof setTimeout> | undefined;

  const hide = () => {
    clearTimeout(showTimer);
    if (shown()) setShown(null);
  };

  const show = () => {
    if (toolState.active !== 'select') return hide();
    const range = pageRange();
    if (!range) return hide();
    const size = {
      width: row?.offsetWidth || GUESS.width * props.actions.length,
      height: row?.offsetHeight || GUESS.height,
    };
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const { left, top } = placeMathPopover(endRect(range), size, viewport);
    setShown({ left, top, range: range.cloneRange() });
  };

  const labels = createMemo(() => shortLabels(props.actions.map((a) => a.title)));
  const hint = (action: ExtensionAction) => {
    const binding = extensionBindings().find((b) => `${b.extensionId}/${b.command}` === action.id);
    return binding
      ? `${action.title} (${formatShortcut(canonicalChord(binding.chord))})`
      : action.title;
  };

  const run = (action: ExtensionAction) => {
    // The text is captured a moment after the selection settles; a press
    // sooner than that takes it now.
    settleSelectionNow();
    const text = selectedText();
    hide();
    if (text.length > 0) void runExtensionAction(action, text);
  };

  onMount(() => {
    const inRow = (target: EventTarget | null) =>
      row !== undefined && target instanceof Node && row.contains(target);
    const onPointerDown = (event: PointerEvent) => {
      if (inRow(event.target)) return;
      dragging = true;
      hide();
    };
    const onPointerUp = (event: PointerEvent) => {
      if (inRow(event.target) || !dragging) return;
      dragging = false;
      // After the browser has settled the selection: a plain click
      // collapses it, which this then sees.
      clearTimeout(showTimer);
      showTimer = setTimeout(show, 0);
    };
    const onSelectionChange = () => {
      const current = shown();
      if (!current) return;
      const selection = document.getSelection();
      if (
        !selection ||
        selection.isCollapsed ||
        selection.rangeCount === 0 ||
        !sameRange(selection.getRangeAt(0), current.range)
      ) {
        hide();
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') hide();
    };
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('pointerup', onPointerUp, true);
    document.addEventListener('selectionchange', onSelectionChange);
    document.addEventListener('scroll', hide, true);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', hide);
    onCleanup(() => {
      clearTimeout(showTimer);
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('pointerup', onPointerUp, true);
      document.removeEventListener('selectionchange', onSelectionChange);
      document.removeEventListener('scroll', hide, true);
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', hide);
    });
  });

  // Taking up a drawing tool puts the button away.
  createEffect(() => {
    if (toolState.active !== 'select') hide();
  });

  return (
    <Show when={shown()}>
      {(at) => (
        <Portal>
          <div
            ref={row}
            class="selection-actions"
            style={{ left: `${at().left}px`, top: `${at().top}px` }}
          >
            <For each={props.actions}>
              {(action, i) => (
                <button
                  type="button"
                  class="selection-action"
                  title={hint(action)}
                  aria-label={action.title}
                  // Neither the selection nor the focus moves.
                  onPointerDown={(e) => e.preventDefault()}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => run(action)}
                >
                  {labels()[i()]}
                </button>
              )}
            </For>
          </div>
        </Portal>
      )}
    </Show>
  );
}
