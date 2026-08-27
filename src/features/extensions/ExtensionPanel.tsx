/** Draws what an extension described.
 *
 * The extension sends rows; this renders them with the same components the
 * built-in panels use. That is the whole bargain — the extension gets a panel
 * that matches SpeedyF and follows every future restyle, and SpeedyF keeps
 * control of the DOM.
 */
import { For, Show, createEffect, createSignal } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconRefresh } from '../../components/icons';
import { activeTab } from '../../stores/tabsStore';
import {
  panelDocId,
  panelUi,
  refreshPanel,
  runPanelAction,
  type ExtensionPanel,
} from './extensionStore';

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

  // Rendered when what is on screen is not about the paper being read — on
  // first open, and after that whenever the two drift apart. Not on a timer:
  // an extension should not be woken because a document scrolled.
  //
  // Deliberately a comparison rather than a reaction to a tab switch. Every
  // tab mounts its own sidebar, so changing documents does not re-run an
  // effect in this component — it disposes it and mounts a new one, which
  // finds the store already holding the *previous* document's rows and, on
  // any mount-time or change-based test, concludes there is nothing to do.
  // That is precisely how this panel spent a release showing the last paper's
  // references. Asking "is this content about the open document?" is true
  // regardless of which components came and went to get here.
  createEffect(() => {
    const open = activeTab()?.documentStore.state.docId ?? null;
    const shown = panelDocId(props.panel.id);
    if (panelUi(props.panel.id) === undefined || shown !== open) void refresh();
  });

  return (
    <div class="sidebar-scroll ext-panel" aria-label={props.panel.title}>
      <div class="ext-panel-head">
        <span class="ext-panel-title">{props.panel.title}</span>
        {/* The same control the viewer uses for undo, redo and page steps.
            Spinning while it works says "busy" without the button changing
            width, which a text label could not do. */}
        <IconButton label="Refresh" disabled={refreshing()} onClick={() => void refresh()}>
          <span class="ext-refresh-icon" classList={{ 'is-spinning': refreshing() }}>
            <IconRefresh />
          </span>
        </IconButton>
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
                        classList={{
                          'is-actionable': !!row.action,
                          'is-nested': !!row.nested,
                        }}
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
