/** Prepare form mode's slim row under the toolbar: the field each tool
 * draws, and Done. Shown only while the mode is on.
 *
 * Also owns the mode's keys, while no text box has focus: the arrows nudge
 * the selection (Shift: by ten points), Delete removes it, Cmd/Ctrl+D
 * duplicates it, and Escape puts the tool down, then clears the selection,
 * then leaves the mode. */
import { createEffect, For, onCleanup, Show, type JSX } from 'solid-js';
import { activeTab } from '../../stores/tabsStore';
import { deleteFields, duplicateFields, moveFields, type PrepareTool } from './model';
import {
  commitDesign,
  designOf,
  ensureBase,
  prepareState,
  preparing,
  select,
  selectedIds,
  setTool,
  stopPreparing,
} from './prepareStore';

const svg = (children: JSX.Element) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.4"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {children}
  </svg>
);

export const TOOLS: {
  tool: Exclude<PrepareTool, 'select'>;
  label: string;
  icon: () => JSX.Element;
}[] = [
  {
    tool: 'text',
    label: 'Text field',
    icon: () =>
      svg(
        <>
          <rect x="1.5" y="4.5" width="13" height="7" rx="1" />
          <path d="M4 6.5v3" />
        </>
      ),
  },
  {
    tool: 'multiline',
    label: 'Multiline text field',
    icon: () =>
      svg(
        <>
          <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
          <path d="M4 5.5h8M4 8h8M4 10.5h5" />
        </>
      ),
  },
  {
    tool: 'checkbox',
    label: 'Checkbox',
    icon: () =>
      svg(
        <>
          <rect x="2.5" y="2.5" width="11" height="11" rx="1.5" />
          <path d="M5 8.2l2 2 4-4.5" />
        </>
      ),
  },
  {
    tool: 'radio',
    label: 'Radio button',
    icon: () =>
      svg(
        <>
          <circle cx="8" cy="8" r="5.5" />
          <circle cx="8" cy="8" r="2.2" fill="currentColor" stroke="none" />
        </>
      ),
  },
  {
    tool: 'combo',
    label: 'Dropdown',
    icon: () =>
      svg(
        <>
          <rect x="1.5" y="4.5" width="13" height="7" rx="1" />
          <path d="M9.5 7l1.5 1.5L12.5 7" />
        </>
      ),
  },
  {
    tool: 'list',
    label: 'List box',
    icon: () =>
      svg(
        <>
          <rect x="1.5" y="2.5" width="13" height="11" rx="1" />
          <path d="M4 5.5h8M4 8h8M4 10.5h8" />
        </>
      ),
  },
  {
    tool: 'signature',
    label: 'Signature field',
    icon: () =>
      svg(
        <>
          <path d="M2 12.5h12" />
          <path d="M3.5 10c1.5-4 2.5-5 3-3.5s-.5 3 1 2.5 1.5-2 3-1.5" />
        </>
      ),
  },
  {
    tool: 'date',
    label: 'Date field',
    icon: () =>
      svg(
        <>
          <rect x="2.5" y="3.5" width="11" height="10" rx="1.5" />
          <path d="M2.5 6.5h11M5.5 2v3M10.5 2v3" />
        </>
      ),
  },
];

/** Whether a key press belongs to a text box rather than to the page. */
function typing(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
}

let nudgeGroup: symbol | null = null;

/** The mode's keys; true when one was handled. */
export function prepareKey(e: KeyboardEvent): boolean {
  const tab = activeTab();
  if (!tab || !preparing(tab) || typing(e.target)) return false;
  const mod = e.metaKey || e.ctrlKey;
  const ids = selectedIds(tab);
  const arrows: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, 1],
    ArrowDown: [0, -1],
  };
  const arrow = arrows[e.key];
  if (arrow && ids.length > 0 && !mod && !e.altKey) {
    const step = e.shiftKey ? 10 : 1;
    nudgeGroup ??= Symbol('nudge');
    commitDesign(tab, moveFields(designOf(tab), ids, arrow[0] * step, arrow[1] * step), nudgeGroup);
    return true;
  }
  nudgeGroup = null;
  if ((e.key === 'Delete' || e.key === 'Backspace') && ids.length > 0 && !mod) {
    commitDesign(tab, deleteFields(designOf(tab), ids));
    select([]);
    return true;
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'd' && ids.length > 0) {
    const { design, ids: copies } = duplicateFields(designOf(tab), ids);
    commitDesign(tab, design);
    select(copies);
    return true;
  }
  if (e.key === 'Escape') {
    if (prepareState.tool !== 'select') setTool('select');
    else if (ids.length > 0) select([]);
    else stopPreparing();
    return true;
  }
  return false;
}

export default function PrepareTools() {
  const visible = () => preparing();

  createEffect(() => {
    const tab = activeTab();
    if (tab && preparing(tab)) void ensureBase(tab);
  });

  createEffect(() => {
    if (!visible()) return;
    const onKey = (e: KeyboardEvent) => {
      if (prepareKey(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={visible()}>
      <div class="text-tool-options prepare-tool-options" role="toolbar" aria-label="Prepare form">
        <span class="text-tool-label">Prepare form</span>
        <div class="text-format-buttons" role="radiogroup" aria-label="Draw">
          <For each={TOOLS}>
            {(t) => (
              <button
                type="button"
                class="icon-btn"
                role="radio"
                classList={{ 'is-active': prepareState.tool === t.tool }}
                aria-checked={prepareState.tool === t.tool}
                aria-label={t.label}
                title={t.label}
                onClick={() => setTool(prepareState.tool === t.tool ? 'select' : t.tool)}
              >
                {t.icon()}
              </button>
            )}
          </For>
        </div>
        <span class="text-tool-hint">
          {prepareState.tool === 'select'
            ? 'Click a field to edit it; drag to move; arrow keys nudge'
            : 'Drag to draw, or click for the usual size; blanks snap'}
        </span>
        <button type="button" class="secondary-btn fill-done" onClick={() => stopPreparing()}>
          Done
        </button>
      </div>
    </Show>
  );
}
