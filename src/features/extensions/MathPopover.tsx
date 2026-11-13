/** The popover `ui.showMath` fills: a formula typeset by KaTeX, copy buttons
 * for the forms people paste it in, and a dim note on where it came from.
 *
 * Copying happens in the button's own click handler, so the write runs inside
 * the user's gesture — the extension never needs the clipboard for this. An
 * action that fails says why in the same place, so "this looks like a scan"
 * appears where the formula would have. Escape or a click elsewhere closes it.
 */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { MathFormula } from '../../components/MathText';
import {
  alignCopy,
  hasLineBreaks,
  latexCopy,
  markdownCopy,
  provenanceNote,
  renderSource,
} from './mathCopy';
import { closeMath, mathPopover, placeMathPopover } from './mathPopoverStore';

const WIDTH = 440;
const HEIGHT_GUESS = 180;

export default function MathPopover() {
  let panel: HTMLElement | undefined;
  const [copied, setCopied] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal<string | null>(null);

  const position = () => {
    const state = mathPopover();
    if (!state) return {};
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const width = Math.min(WIDTH, viewport.width - 16);
    const height = panel?.offsetHeight || HEIGHT_GUESS;
    const { left, top } = placeMathPopover(state.anchor, { width, height }, viewport);
    return { left: `${left}px`, top: `${top}px`, width: `${width}px` };
  };

  const formats = () => {
    const state = mathPopover();
    if (state?.phase !== 'shown') return [];
    const out = [
      { label: 'LaTeX', text: latexCopy(state.latex), hint: 'Copy the LaTeX source' },
      {
        label: 'Markdown',
        text: markdownCopy(state.latex, state.display),
        hint: state.display ? 'Copy as $$...$$' : 'Copy as $...$',
      },
    ];
    if (hasLineBreaks(state.latex)) {
      out.push({ label: 'align', text: alignCopy(state.latex), hint: 'Copy as an align block' });
    }
    return out;
  };

  const copy = async (label: string, text: string) => {
    setFailed(null);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(label);
    } catch (err) {
      setCopied(null);
      setFailed(err instanceof Error ? err.message : String(err));
    }
  };

  onMount(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && mathPopover()) {
        event.preventDefault();
        closeMath();
      }
    };
    const onPointer = (event: PointerEvent) => {
      if (!mathPopover() || !panel) return;
      const target = event.target as Node | null;
      // The equation buttons open it; a press on one is not a click away.
      if (target instanceof Element && target.closest('.equation-action')) return;
      if (target && !panel.contains(target)) closeMath();
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('pointerdown', onPointer, true);
    onCleanup(() => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('pointerdown', onPointer, true);
    });
  });

  const loading = () => {
    const state = mathPopover();
    return state?.phase === 'loading' ? state : null;
  };
  const errored = () => {
    const state = mathPopover();
    return state?.phase === 'error' ? state : null;
  };
  const shown = () => {
    const state = mathPopover();
    return state?.phase === 'shown' ? state : null;
  };

  // A new formula, or a new popover, starts with nothing copied.
  createEffect(
    on(mathPopover, () => {
      setCopied(null);
      setFailed(null);
    })
  );

  return (
    <Show when={mathPopover()}>
      <Portal>
        <section
          ref={panel}
          class="math-popover"
          style={position()}
          role="dialog"
          aria-label="Formula"
        >
          <Show when={loading()}>
            {(state) => (
              <div class="math-popover-status" role="status">
                {state().title}...
              </div>
            )}
          </Show>
          <Show when={errored()}>
            {(state) => (
              <div class="math-popover-error" role="alert">
                {state().message}
              </div>
            )}
          </Show>
          <Show when={shown()}>
            {(state) => (
              <>
                <div class="math-popover-formula">
                  <MathFormula tex={renderSource(state().latex)} display={state().display} />
                </div>
                <div class="math-popover-actions">
                  <span class="math-popover-label">Copy</span>
                  <For each={formats()}>
                    {(format) => (
                      <button
                        type="button"
                        class="math-popover-copy"
                        classList={{ 'is-copied': copied() === format.label }}
                        title={format.hint}
                        onClick={() => void copy(format.label, format.text)}
                      >
                        {copied() === format.label ? 'Copied' : format.label}
                      </button>
                    )}
                  </For>
                </div>
                <div class="math-popover-note">
                  {provenanceNote(state().source, state().confidence)}
                </div>
                <Show when={failed()}>
                  <div class="math-popover-error" role="alert">
                    Could not copy: {failed()}
                  </div>
                </Show>
              </>
            )}
          </Show>
        </section>
      </Portal>
    </Show>
  );
}
