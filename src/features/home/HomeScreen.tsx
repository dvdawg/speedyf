/** Shown when no tab is active (App boots with no tabs, or Home is clicked).
 * Independent of open tabs — going Home never closes anything.
 *
 * A fixed rail on the left holds the open action and the section list; the
 * right column renders the selected section. Selection is local state, and
 * App unmounts this whole screen once a tab becomes active, so returning home
 * always lands back on Recents. */
import { createSignal, For } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import type { Component } from 'solid-js';
import { openFromDialog } from '../document/tabsController';
import { IconExtension, IconHome, IconList, IconOpen, IconSettings } from '../../components/icons';
import RecentsPanel from './RecentsPanel';
import LibraryPanel from './LibraryPanel';
import ExtensionsPanel from './ExtensionsPanel';
import SettingsPanel from './SettingsPanel';

type PanelId = 'recents' | 'library' | 'extensions' | 'settings';

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

export default function HomeScreen() {
  const [active, setActive] = createSignal<PanelId>('recents');

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
    </div>
  );
}
