/** The Fill tool's row under the toolbar: what a click places - typed text,
 * a mark or today's date - and a way to put the tool down. Shown only while
 * the tool is held. */
import { For, Show, type JSX } from 'solid-js';
import { activeTab } from '../../stores/tabsStore';
import { setActiveTool, toolState } from '../annotations/toolStore';
import { fillState, setFillMode, type FillMode } from './fillStore';
import { settings } from '../../stores/settings';
import { formatDate } from './fill';

const svg = (children: JSX.Element) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {children}
  </svg>
);

const MODES: { mode: FillMode; label: string; icon: () => JSX.Element }[] = [
  {
    mode: 'text',
    label: 'Type text',
    icon: () => svg(<path d="M4 4h8M8 4v9M6 13h4" />),
  },
  { mode: 'check', label: 'Check mark', icon: () => svg(<path d="M3 8.5l3 3 7-7.5" />) },
  { mode: 'cross', label: 'Cross', icon: () => svg(<path d="M4 4l8 8M12 4l-8 8" />) },
  {
    mode: 'dot',
    label: 'Dot',
    icon: () => svg(<circle cx="8" cy="8" r="3" fill="currentColor" stroke="none" />),
  },
  { mode: 'line', label: 'Strike line', icon: () => svg(<path d="M2.5 8h11" />) },
  { mode: 'circle', label: 'Circle', icon: () => svg(<ellipse cx="8" cy="8" rx="6" ry="4.5" />) },
  {
    mode: 'date',
    label: 'Date',
    icon: () =>
      svg(
        <>
          <rect x="2.5" y="3.5" width="11" height="10" rx="1.5" />
          <path d="M2.5 6.5h11M5.5 2v3M10.5 2v3" />
        </>
      ),
  },
];

export default function FillTools() {
  const visible = () => toolState.active === 'fill' && activeTab()?.documentStore.state.loaded;
  return (
    <Show when={visible()}>
      <div
        class="text-tool-options fill-tool-options"
        role="toolbar"
        aria-label="Fill and sign"
        onPointerDown={(e) => {
          // Keep the caret in the answer being typed.
          if ((e.target as HTMLElement).closest('button')) e.preventDefault();
        }}
      >
        <span class="text-tool-label">Fill</span>
        <div class="text-format-buttons" role="radiogroup" aria-label="Place">
          <For each={MODES}>
            {(m) => (
              <button
                type="button"
                class="icon-btn"
                role="radio"
                classList={{ 'is-active': fillState.mode === m.mode }}
                aria-checked={fillState.mode === m.mode}
                aria-label={m.label}
                title={
                  m.mode === 'date'
                    ? `Date: click to place ${formatDate(new Date(), settings.dateFormat)}`
                    : m.label
                }
                onClick={() => setFillMode(m.mode)}
              >
                {m.icon()}
              </button>
            )}
          </For>
        </div>
        <span class="text-tool-hint">
          {fillState.mode === 'text'
            ? 'Click a blank to type; Tab moves to the next'
            : fillState.mode === 'date'
              ? 'Click to place the date'
              : 'Click to place; on a box it is centred'}
        </span>
        <button
          type="button"
          class="secondary-btn fill-done"
          onClick={() => setActiveTool('select')}
        >
          Done
        </button>
      </div>
    </Show>
  );
}
