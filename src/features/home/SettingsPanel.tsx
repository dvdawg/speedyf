/** App preferences. Writes the same `settings` store the StatusBar controls
 * write, so the two surfaces stay in sync without extra wiring. */
import { applyLowMemory, effectiveTheme, settings, updateSettings } from '../../stores/settings';
import type { ThemePref } from '../../stores/settings';

export default function SettingsPanel() {
  return (
    <>
      <div class="home-panel-head">
        <h2>Settings</h2>
      </div>
      <div class="home-panel-body">
        <div class="home-setting">
          <label class="home-setting-text" for="setting-theme">
            <span class="home-setting-title">Theme</span>
            <span class="home-setting-desc">
              Follow the system appearance, or pin SpeedyF to light or dark.
            </span>
          </label>
          <select
            id="setting-theme"
            value={settings.theme}
            onInput={(e) => updateSettings({ theme: e.currentTarget.value as ThemePref })}
            aria-label={`Theme (currently ${effectiveTheme()})`}
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-low-memory">
            <span class="home-setting-title">Low memory</span>
            <span class="home-setting-desc">
              Reduce cache memory budgets. Uses less RAM on large documents, at the cost of
              re-rendering pages more often.
            </span>
          </label>
          <input
            id="setting-low-memory"
            type="checkbox"
            checked={settings.lowMemory}
            onInput={(e) => applyLowMemory(e.currentTarget.checked)}
          />
        </div>
      </div>
    </>
  );
}
