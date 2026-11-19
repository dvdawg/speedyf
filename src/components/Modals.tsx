/** In-app sheets: password prompt, unsaved-changes guard, confirmations,
 * external links, error notice, and the feature dialogs with stores of
 * their own. Every one is a Sheet, so they look and answer keys alike. */
import { createEffect, createSignal, Show } from 'solid-js';
import Sheet from './Sheet';
import PrintDialog from '../features/print/PrintDialog';
import { closePrint, printState } from '../features/print/printStore';
import SignDialog from '../features/sign/SignDialog';
import { signState } from '../features/sign/signStore';
import RedactDialog from '../features/protect/redact/RedactDialog';
import { redactState } from '../features/protect/redact/redactStore';
import SanitizeDialog from '../features/protect/sanitize/SanitizeDialog';
import { sanitizeState } from '../features/protect/sanitize/sanitizeStore';
import ProtectDialog from '../features/protect/ProtectDialog';
import { protectTab } from '../features/protect/protectStore';
import CertSignDialog from '../features/sign/certificate/CertSignDialog';
import { certState } from '../features/sign/certificate/certStore';
import {
  dismissError,
  modal,
  resolveConfirm,
  resolveExternalLink,
  resolvePassword,
  resolveUnsaved,
} from '../stores/modalStore';

const LABELS: Record<NonNullable<typeof modal.kind>, string> = {
  password: 'Password required',
  unsaved: 'Unsaved changes',
  confirm: 'Confirm',
  'external-link': 'Open this link?',
  error: 'Something went wrong',
};

export default function Modals() {
  const [password, setPassword] = createSignal('');
  const [rememberHost, setRememberHost] = createSignal(false);

  // Each link is its own decision: never carry a ticked box into the next one.
  createEffect(() => {
    if (modal.kind === 'external-link') setRememberHost(false);
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

  return (
    <>
      {/* Print has its own store rather than a modal kind: it owns a temp
          document that must be released on close, which the single-slot
          resolver pattern has nowhere to put. */}
      <Show when={printState.open}>
        <Sheet label="Print" class="modal-wide" onCancel={() => void closePrint()}>
          <PrintDialog />
        </Sheet>
      </Show>
      {/* Visual signatures: their own store, like print. */}
      <Show when={signState.dialog}>
        <SignDialog />
      </Show>
      {/* Redaction and hidden information: their own stores, like print. */}
      <Show when={redactState.panelOpen}>
        <RedactDialog />
      </Show>
      <Show when={sanitizeState.open}>
        <SanitizeDialog />
      </Show>
      {/* Protect...: its own store too, keyed by the tab it protects. */}
      <Show when={protectTab()} keyed>
        {(tab) => <ProtectDialog tab={tab} />}
      </Show>
      {/* Certificate signing: its own store too. */}
      <Show when={certState.open}>
        <CertSignDialog />
      </Show>
      <Show when={modal.kind} keyed>
        {(kind) => (
          <Sheet label={LABELS[kind]} onCancel={cancelModal}>
            <Show when={kind === 'password'}>
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

            <Show when={kind === 'unsaved'}>
              <h2>Unsaved changes</h2>
              <p>{modal.message}</p>
              <div class="modal-actions">
                <button
                  type="button"
                  class="destructive-btn modal-actions-lead"
                  onClick={() => resolveUnsaved('discard')}
                >
                  Discard changes
                </button>
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => resolveUnsaved('cancel')}
                >
                  Cancel
                </button>
                <button type="button" class="primary-btn" onClick={() => resolveUnsaved('save')}>
                  Save
                </button>
              </div>
            </Show>

            <Show when={kind === 'confirm'}>
              <p>{modal.message}</p>
              <Show
                when={modal.destructive}
                fallback={
                  <div class="modal-actions">
                    <button
                      type="button"
                      class="secondary-btn"
                      onClick={() => resolveConfirm(false)}
                    >
                      Cancel
                    </button>
                    <button type="button" class="primary-btn" onClick={() => resolveConfirm(true)}>
                      {modal.confirmLabel ?? 'Confirm'}
                    </button>
                  </div>
                }
              >
                {/* Cannot be taken back: drawn destructive, and Cancel is
                    the default Return presses. */}
                <div class="modal-actions">
                  <button
                    type="button"
                    class="destructive-btn"
                    onClick={() => resolveConfirm(true)}
                  >
                    {modal.confirmLabel ?? 'Confirm'}
                  </button>
                  <button type="button" class="primary-btn" onClick={() => resolveConfirm(false)}>
                    Cancel
                  </button>
                </div>
              </Show>
            </Show>

            <Show when={kind === 'external-link'}>
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
                  onClick={() => resolveExternalLink({ open: true, remember: rememberHost() })}
                >
                  Open link
                </button>
              </div>
            </Show>

            <Show when={kind === 'error'}>
              <h2>Something went wrong</h2>
              <p>{modal.message}</p>
              <div class="modal-actions">
                <button type="button" class="primary-btn" onClick={() => dismissError()}>
                  OK
                </button>
              </div>
            </Show>
          </Sheet>
        )}
      </Show>
    </>
  );
}
