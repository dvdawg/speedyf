/** Draws what an extension described.
 *
 * The extension sends rows; this renders them with the same components the
 * built-in panels use. That is the whole bargain — the extension gets a panel
 * that matches SpeedyF and follows every future restyle, and SpeedyF keeps
 * control of the DOM.
 */
import { For, Show, createEffect, createSignal } from 'solid-js';
import { panelUi, refreshPanel, runPanelAction, type ExtensionPanel } from './extensionStore';

export default function ExtensionPanelView(props: { panel: ExtensionPanel }) {
  const [refreshing, setRefreshing] = createSignal(false);
  const ui = () => panelUi(props.panel.id);

  const refresh = async () => {
    setRefreshing(true);
    try {
      await refreshPanel(props.panel.id);
    } finally {
      setRefreshing(false);
    }
  };

  // Rendered on open rather than on a timer: an extension should not be woken
  // because a document scrolled, and a panel nobody is looking at costs nothing.
  createEffect(() => {
    if (panelUi(props.panel.id) === undefined) void refresh();
  });

  return (
    <div class="sidebar-scroll ext-panel" aria-label={props.panel.title}>
      <div class="ext-panel-head">
        <span class="ext-panel-title">{props.panel.title}</span>
        <button
          type="button"
          class="ext-action"
          disabled={refreshing()}
          onClick={() => void refresh()}
        >
          {refreshing() ? '…' : 'Refresh'}
        </button>
      </div>

      <Show when={ui()} fallback={<div class="panel-note">Loading…</div>}>
        <Show when={ui()!.error}>
          <div class="ext-error" role="status">
            {ui()!.error}
          </div>
        </Show>

        <Show
          when={ui()!.sections.some((section) => section.rows.length > 0)}
          fallback={
            <Show when={ui()!.empty}>
              <div class="panel-note">{ui()!.empty}</div>
            </Show>
          }
        >
          <For each={ui()!.sections}>
            {(section) => (
              <Show when={section.rows.length > 0 || section.title}>
                <div class="ext-panel-section">
                  <Show when={section.title}>
                    <div class="ext-panel-section-title">{section.title}</div>
                  </Show>
                  <For each={section.rows}>
                    {(row) => (
                      <div
                        class="ext-panel-row"
                        classList={{ 'is-actionable': !!row.action }}
                        // A row without an action is text, not a control, so it
                        // stays out of the tab order and off the click path.
                        role={row.action ? 'button' : undefined}
                        tabindex={row.action ? 0 : undefined}
                        onClick={() =>
                          row.action &&
                          void runPanelAction(
                            props.panel.extensionId,
                            row.action.command,
                            row.action.value
                          )
                        }
                        onKeyDown={(e) => {
                          if (!row.action) return;
                          if (e.key !== 'Enter' && e.key !== ' ') return;
                          e.preventDefault();
                          void runPanelAction(
                            props.panel.extensionId,
                            row.action.command,
                            row.action.value
                          );
                        }}
                      >
                        <div class="ext-panel-row-text">
                          <span class="ext-panel-row-title">{row.title}</span>
                          <Show when={row.subtitle}>
                            <span class="ext-panel-row-subtitle">{row.subtitle}</span>
                          </Show>
                        </div>
                        <Show when={row.badge}>
                          <span class="ext-panel-badge">{row.badge}</span>
                        </Show>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
