/** The Settings window: the same panel Home used to show, in a small window
 * of its own, as desktop apps keep their settings (Cmd+, on macOS, Ctrl+,
 * elsewhere). There is only ever one; asking again raises it (desktop.rs).
 *
 * It writes the one settings store every window reads, and each window
 * follows the change at once (settings.ts). Cmd+W (Ctrl+W) closes it. */
import { createEffect, onCleanup, onMount } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import SettingsPanel from '../home/SettingsPanel';
import Modals from '../../components/Modals';
import { applyPageAppearance, followSettingsFromOtherWindows } from '../../stores/settings';
import { modal } from '../../stores/modalStore';
import { installContextMenuGuard } from '../../lib/nativeMenu';
import { installWebviewGuards } from '../../app/webviewGuards';
import { installAppMenu } from '../../app/appMenu';
import { revealWhenPainted } from '../windows/windowState';
import { IS_MAC } from '../../lib/platform';

export default function SettingsWindow() {
  createEffect(() => applyPageAppearance(document.documentElement));

  onMount(() => {
    const stops = [
      installWebviewGuards(),
      installContextMenuGuard(),
      followSettingsFromOtherWindows(),
    ];
    let disposed = false;
    void installAppMenu('settings', () => ({ tab: null }))
      .then((stop) => {
        if (disposed) stop();
        else stops.push(stop);
      })
      .catch(() => undefined);

    const onKey = (e: KeyboardEvent) => {
      if (modal.kind !== null) return;
      const mod = IS_MAC ? e.metaKey : e.ctrlKey;
      if (!mod || e.key.toLowerCase() !== 'w') return;
      e.preventDefault();
      void getCurrentWindow()
        .close()
        .catch(() => undefined);
    };
    window.addEventListener('keydown', onKey);
    revealWhenPainted();

    onCleanup(() => {
      disposed = true;
      window.removeEventListener('keydown', onKey);
      for (const stop of stops) stop();
    });
  });

  return (
    <div class="settings-window">
      <main class="home-panel">
        <SettingsPanel />
      </main>
      <Modals />
    </div>
  );
}
