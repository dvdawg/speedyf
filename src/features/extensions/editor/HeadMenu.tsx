/** A dim text button in the note editor's head that opens a small list
 * under it: the note's Outline, and Export.
 *
 * Keyboard first: the list takes focus when it opens, the arrows (and Home,
 * End) move, Enter chooses, Escape closes and gives focus back to the
 * button. A click elsewhere closes it. Choosing runs inside the click or
 * key press, so a choice that writes to the clipboard can. */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';

export interface HeadMenuRow {
  text: string;
  /** Indent steps (an outline's heading level). */
  depth?: number;
}

export interface HeadMenuProps {
  label: string;
  /** The button's tooltip. */
  title: string;
  /** The list's accessible name. */
  listLabel: string;
  rows: readonly HeadMenuRow[];
  onChoose: (index: number) => void;
  class?: string;
}

let nextMenu = 1;

export default function HeadMenu(props: HeadMenuProps) {
  const id = `note-head-menu-${nextMenu++}`;
  const [open, setOpen] = createSignal(false);
  const [current, setCurrent] = createSignal(0);
  let root!: HTMLSpanElement;
  let button!: HTMLButtonElement;
  let list: HTMLDivElement | undefined;

  /** The row the keys are on, kept within the list as it changes. */
  const at = () => Math.max(0, Math.min(current(), props.rows.length - 1));

  const show = () => {
    setCurrent(0);
    setOpen(true);
    queueMicrotask(() => list?.focus({ preventScroll: true }));
  };
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.focus();
  };
  const choose = (index: number) => {
    if (index < 0 || index >= props.rows.length) return;
    setOpen(false);
    props.onChoose(index);
  };

  createEffect(() => {
    if (!open()) return;
    const onDown = (e: PointerEvent) => {
      if (!root.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('pointerdown', onDown, true);
    onCleanup(() => document.removeEventListener('pointerdown', onDown, true));
  });

  // Keep the current row in view as the keys move it.
  createEffect(() => {
    if (!open()) return;
    const row = list?.querySelector<HTMLElement>(`#${id}-${at()}`);
    row?.scrollIntoView?.({ block: 'nearest' });
  });

  const onListKey = (e: KeyboardEvent) => {
    const n = props.rows.length;
    if (e.key === 'ArrowDown') setCurrent((at() + 1) % Math.max(1, n));
    else if (e.key === 'ArrowUp') setCurrent((at() - 1 + n) % Math.max(1, n));
    else if (e.key === 'Home') setCurrent(0);
    else if (e.key === 'End') setCurrent(Math.max(0, n - 1));
    else if (e.key === 'Enter' || e.key === ' ') choose(at());
    else if (e.key === 'Escape') close(true);
    else {
      // Tab moves on, and the list goes.
      if (e.key === 'Tab') setOpen(false);
      return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <span class={`note-head-menu ${props.class ?? ''}`} ref={root}>
      <button
        ref={button}
        type="button"
        class="note-head-toggle"
        aria-haspopup="listbox"
        aria-expanded={open()}
        title={props.title}
        onClick={() => (open() ? close(false) : show())}
        onKeyDown={(e) => {
          if (!open() && e.key === 'ArrowDown') {
            e.preventDefault();
            show();
          }
        }}
      >
        {props.label}
      </button>
      <Show when={open()}>
        <div
          ref={list}
          id={id}
          class="note-head-menu-list"
          role="listbox"
          tabindex="-1"
          aria-label={props.listLabel}
          aria-activedescendant={`${id}-${at()}`}
          onKeyDown={onListKey}
        >
          <For each={props.rows}>
            {(row, i) => (
              <div
                id={`${id}-${i()}`}
                role="option"
                class="note-head-menu-row"
                classList={{ 'is-current': i() === at() }}
                aria-selected={i() === at()}
                style={row.depth ? { 'padding-left': `${8 + row.depth * 14}px` } : {}}
                title={row.text}
                onMouseDown={(e) => e.preventDefault()}
                onMouseMove={() => setCurrent(i())}
                onClick={() => choose(i())}
              >
                {row.text}
              </div>
            )}
          </For>
        </div>
      </Show>
    </span>
  );
}
