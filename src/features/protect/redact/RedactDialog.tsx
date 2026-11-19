/** Find and redact: every match of a phrase or a kind of personal data,
 * listed with checkboxes; the chosen ones become redaction marks. Also where
 * the overlay text of new marks is set and where marks are applied. */
import { For, Show } from 'solid-js';
import { activeTab } from '../../../stores/tabsStore';
import { markCount } from './redactMarks';
import {
  applyRedactions,
  cancelApply,
  markChosen,
  openRedactPanel,
  PATTERNS,
  redactState,
  runFind,
  setAllChosen,
  setChosen,
  setOverlay,
  setQuery,
  togglePattern,
} from './redactStore';
import './redact.css';
import Sheet from '../../../components/Sheet';

const KIND_LABEL: Record<string, string> = {
  phrase: 'Phrase',
  email: 'Email',
  phone: 'Phone',
  ssn: 'SSN',
  card: 'Card',
  date: 'Date',
};

export default function RedactDialog() {
  const close = () => openRedactPanel(false);
  const store = () => activeTab()?.documentStore;
  const marks = () => {
    const s = store();
    return s ? markCount(s) : 0;
  };
  const chosenCount = () => redactState.chosen.filter(Boolean).length;
  const armed = () => store()?.state.scrub.applyRedactions === true;

  return (
    <Sheet label="Find and redact" class="protect-dialog" onCancel={close}>
      <h2>Find and redact</h2>
      <form
        class="redact-find"
        onSubmit={(e) => {
          e.preventDefault();
          void runFind();
        }}
      >
        <input
          type="text"
          class="protect-input"
          placeholder="Word or phrase"
          aria-label="Word or phrase"
          value={redactState.query.phrase}
          onInput={(e) => setQuery({ phrase: e.currentTarget.value })}
        />
        <button type="submit" class="secondary-btn" disabled={redactState.running}>
          {redactState.running ? 'Searching...' : 'Find'}
        </button>
      </form>
      <div class="redact-options">
        <label class="protect-check">
          <input
            type="checkbox"
            checked={redactState.query.caseSensitive}
            onChange={(e) => setQuery({ caseSensitive: e.currentTarget.checked })}
          />
          Match case
        </label>
        <label class="protect-check">
          <input
            type="checkbox"
            checked={redactState.query.wholeWords}
            onChange={(e) => setQuery({ wholeWords: e.currentTarget.checked })}
          />
          Whole words
        </label>
      </div>
      <fieldset class="redact-patterns">
        <legend>Also find</legend>
        <For each={PATTERNS}>
          {(p) => (
            <label class="protect-check">
              <input
                type="checkbox"
                checked={redactState.query.patterns.includes(p.kind)}
                onChange={(e) => togglePattern(p.kind, e.currentTarget.checked)}
              />
              {p.label}
            </label>
          )}
        </For>
      </fieldset>

      <Show when={redactState.results}>
        {(results) => (
          <div class="redact-results">
            <div class="redact-results-head">
              <span>
                {results().length === 0
                  ? 'No matches.'
                  : `${results().length} match${results().length === 1 ? '' : 'es'}`}
              </span>
              <Show when={results().length > 0}>
                <span class="redact-results-actions">
                  <button type="button" class="link-btn" onClick={() => setAllChosen(true)}>
                    All
                  </button>
                  <button type="button" class="link-btn" onClick={() => setAllChosen(false)}>
                    None
                  </button>
                </span>
              </Show>
            </div>
            <ul class="protect-list redact-list" role="list">
              <For each={results()}>
                {(m, i) => (
                  <li>
                    <label class="protect-row">
                      <input
                        type="checkbox"
                        checked={redactState.chosen[i()] === true}
                        onChange={(e) => setChosen(i(), e.currentTarget.checked)}
                      />
                      <span class="protect-row-text redact-match">
                        <span class="redact-match-text">{m.text}</span>
                        <span class="protect-detail">
                          {KIND_LABEL[m.kind] ?? m.kind}, page {m.page + 1}
                        </span>
                      </span>
                    </label>
                  </li>
                )}
              </For>
            </ul>
          </div>
        )}
      </Show>

      <label class="redact-overlay">
        <span>Overlay text on new marks</span>
        <input
          type="text"
          class="protect-input"
          placeholder="None"
          value={redactState.overlay}
          onInput={(e) => setOverlay(e.currentTarget.value)}
        />
      </label>

      <p class="protect-hint">
        {marks()} mark{marks() === 1 ? '' : 's'} in this document.
        <Show when={armed()}> Redactions will be applied when you save.</Show>
      </p>

      <div class="modal-actions">
        <Show
          when={armed()}
          fallback={
            <button
              type="button"
              class="destructive-btn"
              disabled={marks() === 0}
              onClick={() => void applyRedactions()}
            >
              Apply redactions...
            </button>
          }
        >
          <button type="button" class="secondary-btn" onClick={() => cancelApply()}>
            Do not apply
          </button>
        </Show>
        <span class="protect-spacer" />
        <button type="button" class="secondary-btn" onClick={close}>
          Close
        </button>
        <button
          type="button"
          class="primary-btn"
          disabled={chosenCount() === 0}
          onClick={() => {
            const s = store();
            if (s) markChosen(s);
          }}
        >
          Mark {chosenCount() > 0 ? chosenCount() : ''} for redaction
        </button>
      </div>
    </Sheet>
  );
}
