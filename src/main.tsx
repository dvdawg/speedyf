/* @refresh reload */
import { render } from 'solid-js/web';
import App from './app/App';
import SettingsWindow from './features/settings/SettingsWindow';
import './styles/global.css';
import { markPlatform } from './lib/platform';

markPlatform(document.documentElement);

const root = document.getElementById('root');
if (!root) throw new Error('missing #root element');

// One bundle, two kinds of window: documents, and the Settings window.
const settingsWindow = new URLSearchParams(window.location.search).get('window') === 'settings';
render(() => (settingsWindow ? <SettingsWindow /> : <App />), root);

// Development only: the markup export self-test (src-tauri/src/markup_selftest.rs).
if (import.meta.env.DEV && !settingsWindow) {
  void import('./features/markup/selftest')
    .then((m) => m.installMarkupSelftest())
    .catch(() => undefined);
}
