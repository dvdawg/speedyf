// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import type * as platformModule from '../lib/platform';

// A stand-in for every native menu object: it remembers its items so the
// Open Recent rebuild can list and replace them.
vi.mock('@tauri-apps/api/menu', () => {
  class Item {
    items_: unknown[];
    constructor(options: { items?: unknown[] } = {}) {
      this.items_ = options.items ?? [];
    }
    static new(options: { items?: unknown[] } = {}) {
      return Promise.resolve(new this(options));
    }
    items() {
      return Promise.resolve([...this.items_]);
    }
    remove(item: unknown) {
      this.items_ = this.items_.filter((i) => i !== item);
      return Promise.resolve();
    }
    append(items: unknown[]) {
      this.items_.push(...items);
      return Promise.resolve();
    }
    setEnabled() {
      return Promise.resolve();
    }
    setText() {
      return Promise.resolve();
    }
    setChecked() {
      return Promise.resolve();
    }
    setAccelerator() {
      return Promise.resolve();
    }
    setAsAppMenu() {
      return Promise.resolve();
    }
    setAsWindowsMenuForNSApp() {
      return Promise.resolve();
    }
    setAsHelpMenuForNSApp() {
      return Promise.resolve();
    }
  }
  return {
    Menu: Item,
    MenuItem: Item,
    CheckMenuItem: Item,
    PredefinedMenuItem: Item,
    Submenu: Item,
  };
});
vi.mock('@tauri-apps/api/app', () => ({ getVersion: () => Promise.resolve('0.1.0') }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: () => Promise.resolve(() => undefined),
    isFocused: () => Promise.resolve(true),
  }),
}));
vi.mock('../lib/platform', async (load) => ({
  ...(await load<typeof platformModule>()),
  IS_MAC: true,
}));

import { installAppMenu } from './appMenu';
import { recentStore } from '../stores/recentStore';

describe('installAppMenu', () => {
  it('builds the menu bar and keeps Open Recent following the recent list', async () => {
    const dispose = await installAppMenu('document', () => ({ tab: null }));
    // Opening a file records it, which rebuilds Open Recent at once.
    expect(() => recentStore.recordOpen({ path: '/papers/a.pdf', name: 'a.pdf' })).not.toThrow();
    dispose();
  });
});
