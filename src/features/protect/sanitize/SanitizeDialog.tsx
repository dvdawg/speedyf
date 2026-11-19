/** Remove hidden information: what the document carries besides its pages,
 * as a checklist. Removal happens at the next save, as a clean rewrite. */
import { For, Show } from 'solid-js';
import {
  choose,
  closeSanitize,
  confirmSanitize,
  HIDDEN_LABELS,
  sanitizeState,
} from './sanitizeStore';
import '../redact/redact.css';
import Sheet from '../../../components/Sheet';

export default function SanitizeDialog() {
  const items = () => sanitizeState.info?.items ?? [];
  const anyChosen = () =>
    items().some((i) => i.key === 'revisions' || sanitizeState.chosen[i.key] === true);

  return (
    <Sheet
      label="Remove hidden information"
      class="protect-dialog"
      onCancel={() => closeSanitize()}
    >
      <h2>Remove hidden information</h2>
      <Show
        when={!sanitizeState.scanning}
        fallback={<p class="protect-hint">Inspecting the document...</p>}
      >
        <Show
          when={items().length > 0}
          fallback={<p class="protect-hint">Nothing hidden was found in this document.</p>}
        >
          <p class="protect-hint">
            Found in this document. Ticked items are removed when you next save; the file is
            rewritten in full.
          </p>
          <ul class="protect-list" role="list">
            <For each={items()}>
              {(item) => {
                const fixed = item.key === 'revisions';
                return (
                  <li>
                    <label class="protect-row">
                      <input
                        type="checkbox"
                        checked={fixed || sanitizeState.chosen[item.key] === true}
                        disabled={fixed}
                        onChange={(e) => choose(item.key, e.currentTarget.checked)}
                      />
                      <span class="protect-row-text">
                        <span>
                          {HIDDEN_LABELS[item.key]}
                          <span class="protect-count"> ({item.count})</span>
                        </span>
                        <Show when={fixed}>
                          <span class="protect-detail">Always removed by a clean save.</span>
                        </Show>
                        <Show when={!fixed && item.detail}>
                          <span class="protect-detail">{item.detail}</span>
                        </Show>
                      </span>
                    </label>
                  </li>
                );
              }}
            </For>
          </ul>
          <Show when={sanitizeState.info?.signed}>
            <p class="protect-warn" role="alert">
              This document is digitally signed. Removing anything rewrites it and the signatures
              will no longer be valid.
            </p>
          </Show>
        </Show>
      </Show>
      <div class="modal-actions">
        <button type="button" class="secondary-btn" onClick={() => closeSanitize()}>
          Cancel
        </button>
        <button
          type="button"
          class="primary-btn"
          disabled={sanitizeState.scanning || !anyChosen()}
          onClick={() => confirmSanitize()}
        >
          Remove when saving
        </button>
      </div>
    </Sheet>
  );
}
