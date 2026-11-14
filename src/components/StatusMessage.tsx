import { Show } from 'solid-js';
import { dismissStatus, holdStatus, statusMessage } from '../stores/statusMessage';

/** The status bar's passing remark (statusMessage.ts): muted text, its
 * action a text button after it. Cut short with an ellipsis, never
 * wrapped; announced politely. */
export default function StatusMessage() {
  return (
    <span
      class="sb-flash"
      aria-live="polite"
      onMouseEnter={() => holdStatus(true)}
      onMouseLeave={() => holdStatus(false)}
    >
      <Show when={statusMessage()} keyed>
        {(message) => (
          <>
            <span class="sb-flash-text" title={message.text}>
              {message.text}
              {message.action ? '.' : ''}
            </span>
            <Show when={message.action} keyed>
              {(action) => (
                <button
                  type="button"
                  class="sb-flash-action"
                  onClick={() => {
                    dismissStatus();
                    action.run();
                  }}
                >
                  {action.label}
                </button>
              )}
            </Show>
          </>
        )}
      </Show>
    </span>
  );
}
