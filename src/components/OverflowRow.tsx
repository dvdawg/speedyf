/** A toolbar row that never wraps: the main toolbar, and the option rows
 * under it (text style, Fill, Prepare form). Groups that do not fit fold,
 * least important first (toolbarOverflow.ts), into a trailing overflow
 * button whose menu shows them as labelled rows; a group that `drops` (a
 * hint) just leaves. Widths are measured as the groups render and
 * remembered while they are folded, so the row can tell when one fits again
 * without showing it to find out. */
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { IconOverflow } from './icons';
import { ToolbarMenuContext } from './toolbarMenu';
import { foldedGroups } from './toolbarOverflow';

export interface ToolbarGroup {
  id: string;
  /** Names the group for assistive technology, in the row and the menu. */
  label: string;
  /** Higher stays longer; Infinity never folds. */
  priority: number;
  /** Leaves the row when it does not fit, with no place in the menu. */
  drops?: boolean;
  /** The group's role in the row; in the menu it is always a group. */
  role?: 'group' | 'radiogroup';
  render: () => JSX.Element;
}

/** The overflow button: an .icon-btn, 28px square. */
const BUTTON_WIDTH = 28;

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled)';

/** A control that opens a native panel of its own: a colour well, a select. */
function opensPanel(node: Node | null): boolean {
  return (
    node instanceof HTMLSelectElement || (node instanceof HTMLInputElement && node.type === 'color')
  );
}

function sameList(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((id, i) => b[i] === id);
}

export default function OverflowRow(props: {
  groups: readonly ToolbarGroup[];
  /** The group pushed to the far end of the row. */
  trailingId?: string;
  /** Extra classes for the row. */
  class?: string;
  /** Extra classes for the menu, which is portalled out of the row. */
  menuClass?: string;
  /** Names the overflow button and its menu. */
  moreLabel?: string;
}) {
  const moreLabel = () => props.moreLabel ?? 'More tools';
  const [folded, setFolded] = createSignal<readonly string[]>([]);
  const [open, setOpen] = createSignal(false);
  const [anchor, setAnchor] = createSignal({ top: 0, right: 0 });
  const widths = new Map<string, number>();
  const shown = new Map<string, HTMLElement>();
  let row!: HTMLDivElement;
  let button: HTMLButtonElement | undefined;
  let menu: HTMLDivElement | undefined;
  /** A colour well or select in the menu has opened its native panel. */
  let picking = false;

  const relayout = () => {
    if (!row?.isConnected || row.clientWidth === 0) return;
    for (const [id, el] of shown) widths.set(id, el.getBoundingClientRect().width);
    const style = getComputedStyle(row);
    const available =
      row.clientWidth -
      (parseFloat(style.paddingLeft) || 0) -
      (parseFloat(style.paddingRight) || 0);
    const next = foldedGroups(
      props.groups.map((g) => ({
        id: g.id,
        priority: g.priority,
        drops: g.drops,
        width: widths.get(g.id) ?? 0,
      })),
      {
        available,
        gap: parseFloat(style.columnGap) || 0,
        buttonWidth: BUTTON_WIDTH,
      }
    );
    if (!sameList(next, folded())) setFolded(next);
  };

  const observer =
    typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(() => relayout());
  onMount(() => {
    observer?.observe(row);
    relayout();
  });
  onCleanup(() => observer?.disconnect());

  const Slot = (slot: { group: ToolbarGroup }) => {
    let el!: HTMLDivElement;
    onMount(() => {
      shown.set(slot.group.id, el);
      observer?.observe(el);
      relayout();
    });
    onCleanup(() => {
      shown.delete(slot.group.id);
      observer?.unobserve(el);
    });
    return (
      <div
        ref={el}
        class="tb-group"
        classList={{ 'tb-right': slot.group.id === props.trailingId }}
        role={slot.group.role ?? 'group'}
        aria-label={slot.group.label}
        data-window-drag=""
      >
        {slot.group.render()}
      </div>
    );
  };

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button?.focus();
  };
  const items = () => (menu ? [...menu.querySelectorAll<HTMLElement>(FOCUSABLE)] : []);
  const focusItem = (which: 'first' | 'last' | 1 | -1) => {
    const list = items();
    if (list.length === 0) return;
    const at = list.indexOf(document.activeElement as HTMLElement);
    const index =
      which === 'first'
        ? 0
        : which === 'last'
          ? list.length - 1
          : (at + which + list.length) % list.length;
    list[at < 0 && typeof which === 'number' ? 0 : index]?.focus();
  };
  const show = () => {
    if (!button) return;
    const box = button.getBoundingClientRect();
    setAnchor({ top: box.bottom + 4, right: Math.max(4, window.innerWidth - box.right) });
    picking = false;
    setOpen(true);
    queueMicrotask(() => focusItem('first'));
  };

  const inMenu = () => props.groups.filter((g) => !g.drops && folded().includes(g.id));

  // Nothing folded, nothing to show; and a press elsewhere dismisses.
  createEffect(() => {
    if (inMenu().length === 0) setOpen(false);
  });
  createEffect(() => {
    if (!open()) return;
    const outside = (e: PointerEvent) => {
      const target = e.target as Node | null;
      picking = opensPanel(target) && !!menu?.contains(target);
      if (target && (menu?.contains(target) || button?.contains(target))) return;
      close(false);
    };
    // A native colour panel or pop-up list can take the window's focus while
    // it is open: the control that opened it stays, so its choice lands.
    const blur = () => {
      if (!picking) close(false);
    };
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('blur', blur);
    onCleanup(() => {
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('blur', blur);
    });
  });

  const onMenuKey = (e: KeyboardEvent) => {
    const keys: Record<string, () => void> = {
      ArrowDown: () => focusItem(1),
      ArrowUp: () => focusItem(-1),
      Home: () => focusItem('first'),
      End: () => focusItem('last'),
      Escape: () => close(true),
      Tab: () => close(true),
    };
    const action = keys[e.key];
    if (!action) return;
    // A text field keeps Home and End for its caret.
    if ((e.key === 'Home' || e.key === 'End') && e.target instanceof HTMLInputElement) return;
    e.preventDefault();
    e.stopPropagation();
    action();
  };

  // A choice in a menu nested inside this one (Sign, Redact) is a choice
  // made: the overflow goes with it.
  const onMenuClick = (e: MouseEvent) => {
    if (opensPanel(e.target as Node | null)) picking = true;
    const item = (e.target as Element | null)?.closest('[role^="menuitem"]');
    if (item && item.closest('[role="menu"]') !== menu) close(true);
  };

  return (
    <div
      class={props.class ? `toolbar-row ${props.class}` : 'toolbar-row'}
      ref={row}
      data-window-drag=""
    >
      <For each={props.groups}>
        {(group) => (
          <Show when={!folded().includes(group.id)}>
            <Slot group={group} />
          </Show>
        )}
      </For>
      <Show when={inMenu().length > 0}>
        <button
          ref={button}
          type="button"
          class="icon-btn tb-overflow"
          classList={{ 'is-active': open() }}
          title={moreLabel()}
          aria-label={moreLabel()}
          aria-haspopup="menu"
          aria-expanded={open()}
          onClick={() => (open() ? close(false) : show())}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown' && !open()) {
              e.preventDefault();
              show();
            }
          }}
        >
          <IconOverflow />
        </button>
      </Show>
      <Show when={open()}>
        <Portal>
          <ToolbarMenuContext.Provider value={{ close: () => close(true) }}>
            <div
              ref={menu}
              class={props.menuClass ? `tb-menu ${props.menuClass}` : 'tb-menu'}
              role="menu"
              aria-label={moreLabel()}
              style={{ top: `${anchor().top}px`, right: `${anchor().right}px` }}
              onKeyDown={onMenuKey}
              onClick={onMenuClick}
            >
              <For each={inMenu()}>
                {(group, i) => (
                  <>
                    <Show when={i() > 0}>
                      <div class="tb-menu-separator" role="separator" />
                    </Show>
                    <div class="tb-menu-group" role="group" aria-label={group.label}>
                      {group.render()}
                    </div>
                  </>
                )}
              </For>
            </div>
          </ToolbarMenuContext.Provider>
        </Portal>
      </Show>
    </div>
  );
}
