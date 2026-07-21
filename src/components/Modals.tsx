/** In-app modals: password prompt, unsaved-changes guard, error notice. */
import { createSignal, Show } from 'solid-js';
import {
  dismissError,
  modal,
  resolvePassword,
  resolveUnsaved,
} from '../stores/modalStore';

export default function Modals() {
  const [password, setPassword] = createSignal('');

  return (
    <Show when={modal.kind !== null}>
      <div
        class="modal-backdrop"
        onPointerDown={(e) => {
          if (e.target === e.currentTarget && modal.kind === 'error') dismissError();
        }}
      >
        <div class="modal" role="dialog" aria-modal="true" aria-label={modal.kind ?? 'dialog'}>
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
                 
                ref={(el) => setTimeout(() => el.focus(), 0)}
                aria-label="PDF password"
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    setPassword('');
                    resolvePassword(null);
                  }
                }}
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
                 
                ref={(el) => setTimeout(() => el.focus(), 0)}
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
                 
                ref={(el) => setTimeout(() => el.focus(), 0)}
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
