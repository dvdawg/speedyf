/** In-app modals: password prompt, unsaved-changes guard, error notice. */
import { createEffect, createSignal, Show } from 'solid-js';
import PrintDialog from '../features/print/PrintDialog';
import { closePrint, printState } from '../features/print/printStore';
import {
  dismissError,
  modal,
  resolveConfirm,
  resolveExternalLink,
  resolvePassword,
  resolveUnsaved,
} from '../stores/modalStore';

export default function Modals() {
  const [password, setPassword] = createSignal('');
  const [rememberHost, setRememberHost] = createSignal(false);
  let modalEl: HTMLDivElement | undefined;
  let restoreFocus: HTMLElement | null = null;

  // Each link is its own decision: never carry a ticked box into the next one.
  createEffect(() => {
    if (modal.kind === 'external-link') setRememberHost(false);
  });

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
    } else if (modal.kind === 'confirm') {
      resolveConfirm(false);
    } else if (modal.kind === 'external-link') {
      resolveExternalLink({ open: false, remember: false });
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
    <>
      {/* Print has its own store rather than a modal kind: it owns a temp
          document that must be released on close, which the single-slot
          resolver pattern has nowhere to put. */}
      <Show when={printState.open}>
        <div
          class="modal-backdrop"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              void closePrint();
            }
          }}
        >
          <div
            class="modal modal-wide"
            role="dialog"
            aria-modal="true"
            aria-label="Print"
            ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
            tabindex="-1"
          >
            <PrintDialog />
          </div>
        </div>
      </Show>
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
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => resolveUnsaved('cancel')}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => resolveUnsaved('discard')}
                >
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

            <Show when={modal.kind === 'confirm'}>
              <p>{modal.message}</p>
              <div class="modal-actions">
                <button type="button" class="secondary-btn" onClick={() => resolveConfirm(false)}>
                  Cancel
                </button>
                <button
                  type="button"
                  class="primary-btn"
                  ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                  onClick={() => resolveConfirm(true)}
                >
                  {modal.confirmLabel ?? 'Confirm'}
                </button>
              </div>
            </Show>

            <Show when={modal.kind === 'external-link'}>
              <h2>Open this link?</h2>
              <p>
                This link comes from the document and opens outside SpeedyF, in your default
                browser.
              </p>
              <p class="link-host">{modal.link?.host ?? 'Your mail app'}</p>
              <p class="link-href">{modal.link?.href}</p>
              <Show when={modal.link?.host}>
                {(host) => (
                  <label class="modal-check">
                    <input
                      type="checkbox"
                      checked={rememberHost()}
                      onChange={(e) => setRememberHost(e.currentTarget.checked)}
                    />
                    <span>Don't ask again for {host()}</span>
                  </label>
                )}
              </Show>
              <div class="modal-actions">
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => resolveExternalLink({ open: false, remember: false })}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  class="primary-btn"
                  ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                  onClick={() => resolveExternalLink({ open: true, remember: rememberHost() })}
                >
                  Open link
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
    </>
  );
}
