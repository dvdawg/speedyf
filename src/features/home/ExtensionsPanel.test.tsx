// @vitest-environment happy-dom
/**
 * What the extensions shelf offers for each kind of extension.
 *
 * The distinction this locks down is built-in versus installed. A built-in's
 * files are inside the app bundle, so Remove cannot mean anything for one — it
 * would either fail or last until the next update. The backend refuses the call
 * regardless; this is the half of the contract the user actually sees, and it
 * is one `Show` away from silently coming back.
 */
import { render, screen, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ExtensionRecord } from '../extensions/extensionStore';

const uninstallExtension = vi.fn(async () => {});

const state: { loaded: boolean; extensions: ExtensionRecord[] } = {
  loaded: true,
  extensions: [],
};

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));

vi.mock('../extensions/extensionStore', () => ({
  extensionState: state,
  bindingErrors: () => [],
  installExtension: vi.fn(async () => {}),
  installExtensionFromGit: vi.fn(async () => {}),
  reloadExtension: vi.fn(async () => {}),
  setExtensionEnabled: vi.fn(async () => {}),
  setSettingValue: vi.fn(),
  settingValue: () => '',
  uninstallExtension: (...args: unknown[]) => uninstallExtension(...(args as [])),
}));

const { default: ExtensionsPanel } = await import('./ExtensionsPanel');

function extension(overrides: Partial<ExtensionRecord>): ExtensionRecord {
  return {
    id: 'com.example.thing',
    name: 'Thing',
    version: '1.0.0',
    source: '/somewhere/thing',
    permissions: [],
    enabled: true,
    builtin: false,
    manifest: null,
    commands: [],
    panels: [],
    selectionActions: [],
    citationActions: [],
    documentTasks: [],
    settings: [],
    keybindings: [],
    iconUrl: null,
    error: null,
    log: [],
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
  state.extensions = [];
  vi.clearAllMocks();
});

describe('ExtensionsPanel', () => {
  it('offers Remove for an installed extension', () => {
    state.extensions = [extension({})];
    render(() => <ExtensionsPanel />);
    expect(screen.getByRole('button', { name: 'Remove' })).toBeTruthy();
    expect(screen.queryByText('Built in')).toBeNull();
  });

  it('never offers Remove for a built-in', () => {
    state.extensions = [extension({ id: 'speedyf.references', name: 'References', builtin: true })];
    render(() => <ExtensionsPanel />);
    expect(screen.queryByRole('button', { name: 'Remove' })).toBeNull();
    // Disabling is the equivalent, and it is the control that must stay.
    expect(screen.getByRole('button', { name: 'Enabled' })).toBeTruthy();
    expect(screen.getByText('Built in')).toBeTruthy();
  });

  it('marks only the built-in when both kinds are listed', () => {
    state.extensions = [
      extension({ id: 'speedyf.references', name: 'References', builtin: true }),
      extension({}),
    ];
    render(() => <ExtensionsPanel />);
    expect(screen.getAllByText('Built in')).toHaveLength(1);
    expect(screen.getAllByRole('button', { name: 'Remove' })).toHaveLength(1);
  });
});
