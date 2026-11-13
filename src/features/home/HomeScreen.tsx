/** Shown when no tab is active (App boots with no tabs, or Home is clicked).
 * Independent of open tabs — going Home never closes anything.
 *
 * A fixed rail on the left holds the open action and the section list; the
 * right column renders the selected section. Selection is local state, and
 * App unmounts this whole screen once a tab becomes active, so returning home
 * always lands back on Recents — unless another screen asked for a panel on
 * the way in (see `showHomePanel`). */
import { createEffect, createSignal, For } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import type { Component } from 'solid-js';
import { openFromDialog } from '../document/tabsController';
import { IconExtension, IconHome, IconList, IconOpen, IconSettings } from '../../components/icons';
import LaunchToggle from '../projects/LaunchToggle';
import RecentsPanel from './RecentsPanel';
import LibraryPanel from './LibraryPanel';
import ExtensionsPanel from './ExtensionsPanel';
import SettingsPanel from './SettingsPanel';

export type PanelId = 'recents' | 'library' | 'extensions' | 'settings';

const SECTIONS: { id: PanelId; label: string; icon: Component }[] = [
  { id: 'recents', label: 'Recents', icon: IconHome },
  { id: 'library', label: 'Library', icon: IconList },
  { id: 'extensions', label: 'Extensions', icon: IconExtension },
  { id: 'settings', label: 'Settings', icon: IconSettings },
];

const PANELS: Record<PanelId, Component> = {
  recents: RecentsPanel,
  library: LibraryPanel,
  extensions: ExtensionsPanel,
  settings: SettingsPanel,
};

/** A panel another screen asked Home to show. The Projects screen hands its
 * search to the Library panel this way. The request is consumed as soon as
 * it is honoured, so a later plain visit lands on Recents as before. */
const [requestedPanel, setRequestedPanel] = createSignal<PanelId | null>(null);

export function showHomePanel(id: PanelId): void {
  setRequestedPanel(id);
}

export default function HomeScreen() {
  const [active, setActive] = createSignal<PanelId>('recents');

  // Honours a request whether it arrived before this mount or while the
  // screen is already up.
  createEffect(() => {
    const requested = requestedPanel();
    if (requested) {
      setActive(requested);
      setRequestedPanel(null);
    }
  });

  return (
    <div class="home-screen">
      <aside class="home-rail">
        <h1 class="home-wordmark">SpeedyF</h1>
        <button
          type="button"
          class="primary-btn home-rail-action"
          onClick={() => void openFromDialog()}
        >
          <IconOpen /> Open PDF…
        </button>
        <nav class="home-nav" aria-label="Home sections">
          <For each={SECTIONS}>
            {(section) => (
              <button
                type="button"
                class="home-nav-item"
                classList={{ 'is-active': active() === section.id }}
                aria-current={active() === section.id ? 'page' : undefined}
                onClick={() => setActive(section.id)}
              >
                <Dynamic component={section.icon} />
                <span>{section.label}</span>
              </button>
            )}
          </For>
        </nav>
      </aside>
      <main class="home-panel">
        <Dynamic component={PANELS[active()]} />
      </main>
      <LaunchToggle class="home-launch-toggle" />
    </div>
  );
}
