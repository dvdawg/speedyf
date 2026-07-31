/** In-app modals: password prompt, unsaved-changes guard, error notice. */
import { createEffect, createSignal, Show } from 'solid-js';
import { dismissError, modal, resolvePassword, resolveUnsaved } from '../stores/modalStore';

export default function Modals() {
  const [password, setPassword] = createSignal('');
  let modalEl: HTMLDivElement | undefined;
  let restoreFocus: HTMLElement | null = null;

  createEffect(() => {
    const kind = modal.kind;
    if (kind !== null && restoreFocus === null) {
      restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    } else if (kind === null && restoreFocus !== null) {
      const target = restoreFocus;
      restoreFocus = null;
      queueMicrotask(() => {
        if (target.isConnected) target.focus();
      });
    }
  });

  const cancelModal = () => {
    if (modal.kind === 'password') {
      setPassword('');
      resolvePassword(null);
    } else if (modal.kind === 'unsaved') {
      resolveUnsaved('cancel');
    } else if (modal.kind === 'error') {
      dismissError();
    }
  };

  const trapKeys = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      cancelModal();
      return;
    }
    if (e.key !== 'Tab' || !modalEl) return;
    const controls = [
      ...modalEl.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled)'
      ),
    ];
    if (controls.length === 0) return;
    const first = controls[0]!;
    const last = controls[controls.length - 1]!;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <Show when={modal.kind !== null}>
      <div
        class="modal-backdrop"
        onKeyDown={trapKeys}
        onPointerDown={(e) => {
          if (e.target === e.currentTarget && modal.kind === 'error') dismissError();
        }}
      >
        <div
          ref={modalEl}
          class="modal"
          role="dialog"
          aria-modal="true"
          aria-label={modal.kind ?? 'dialog'}
        >
          <Show when={modal.kind === 'password'}>
            <h2>Password required</h2>
            <p>{modal.message}</p>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                const v = password();
                setPassword('');
                resolvePassword(v);
              }}
            >
              <input
                type="password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
                ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                aria-label="PDF password"
              />
              <div class="modal-actions">
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => {
                    setPassword('');
                    resolvePassword(null);
                  }}
                >
                  Cancel
                </button>
                <button type="submit" class="primary-btn">
                  Open
                </button>
              </div>
            </form>
          </Show>

          <Show when={modal.kind === 'unsaved'}>
            <h2>Unsaved changes</h2>
            <p>{modal.message}</p>
            <div class="modal-actions">
              <button type="button" class="secondary-btn" onClick={() => resolveUnsaved('cancel')}>
                Cancel
              </button>
              <button type="button" class="secondary-btn" onClick={() => resolveUnsaved('discard')}>
                Discard changes
              </button>
              <button
                type="button"
                class="primary-btn"
                ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                onClick={() => resolveUnsaved('save')}
              >
                Save
              </button>
            </div>
          </Show>

          <Show when={modal.kind === 'error'}>
            <h2>Something went wrong</h2>
            <p>{modal.message}</p>
            <div class="modal-actions">
              <button
                type="button"
                class="primary-btn"
                ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                onClick={() => dismissError()}
              >
                OK
              </button>
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
}
