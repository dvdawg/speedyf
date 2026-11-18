/** The toolbar's Redact button and its small menu: the marking tool, marking
 * the selected text, find and redact, applying, and removing hidden
 * information. Beside the annotation tools, quiet until opened. */
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import IconButton from '../../../components/IconButton';
import { activeTab } from '../../../stores/tabsStore';
import { hasSelection } from '../../../stores/selectionStore';
import { toggleTool, toolState } from '../../annotations/toolStore';
import { openSanitize } from '../sanitize/sanitizeStore';
import { markCount } from './redactMarks';
import { applyRedactions, cancelApply, markSelection, openRedactPanel } from './redactStore';
import './redact.css';

/** A box with a bar struck through it. */
export const IconRedact = () => (
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
    <rect x="2" y="3" width="12" height="10" rx="1" />
    <path d="M4.5 8h7" stroke-width="3" />
  </svg>
);

export function RedactMenu() {
  const [open, setOpen] = createSignal(false);
  const store = () => activeTab()?.documentStore;
  const loaded = () => store()?.state.loaded ?? false;
  const marks = () => {
    const s = store();
    return s ? markCount(s) : 0;
  };
  const armed = () => store()?.state.scrub.applyRedactions === true;
  let wrap: HTMLDivElement | undefined;

  createEffect(() => {
    if (!open()) return;
    const outside = (e: PointerEvent) => {
      if (wrap && e.target instanceof Node && !wrap.contains(e.target)) setOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);
    onCleanup(() => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape);
    });
  });

  const run = (action: () => unknown) => {
    setOpen(false);
    void action();
  };

  return (
    <div class="redact-menu-wrap" ref={wrap}>
      <IconButton
        label="Redact"
        submenu
        disabled={!loaded()}
        active={open() || toolState.active === 'redact'}
        onClick={() => setOpen(!open())}
      >
        <IconRedact />
      </IconButton>
      <Show when={open() && loaded()}>
        <div class="redact-menu" role="menu" aria-label="Redact">
          <button
            type="button"
            role="menuitemcheckbox"
            class="redact-menu-item"
            aria-checked={toolState.active === 'redact'}
            title="Drag over text to mark whole words, or over anything else to mark an area (hold Alt for an area)"
            onClick={() => run(() => toggleTool('redact'))}
          >
            Mark for redaction
          </button>
          <button
            type="button"
            role="menuitem"
            class="redact-menu-item"
            disabled={!hasSelection()}
            onClick={() => run(() => markSelection())}
          >
            Mark selected text
          </button>
          <button
            type="button"
            role="menuitem"
            class="redact-menu-item"
            onClick={() => run(() => openRedactPanel())}
          >
            Find and redact...
          </button>
          <div class="redact-menu-separator" role="separator" />
          <Show
            when={!armed()}
            fallback={
              <button
                type="button"
                role="menuitem"
                class="redact-menu-item"
                onClick={() => run(() => cancelApply())}
              >
                Do not apply redactions
              </button>
            }
          >
            <button
              type="button"
              role="menuitem"
              class="redact-menu-item"
              disabled={marks() === 0}
              onClick={() => run(() => applyRedactions())}
            >
              Apply {marks() > 0 ? `${marks()} ` : ''}redaction{marks() === 1 ? '' : 's'}...
            </button>
          </Show>
          <button
            type="button"
            role="menuitem"
            class="redact-menu-item"
            onClick={() => run(() => openSanitize())}
          >
            Remove hidden information...
          </button>
        </div>
      </Show>
    </div>
  );
}
