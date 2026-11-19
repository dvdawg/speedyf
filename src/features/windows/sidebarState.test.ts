import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// uiStore reads the last-used sidebar at module load, so storage must exist
// before the imports below run.
const { map } = vi.hoisted(() => {
  const map = new Map<string, string>();
  const storage: Storage = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
    removeItem: (k: string) => void map.delete(k),
    clear: () => map.clear(),
    key: (index: number) => [...map.keys()][index] ?? null,
    get length() {
      return map.size;
    },
  };
  (globalThis as { localStorage?: Storage }).localStorage = storage;
  map.set('speedyf-sidebar-width', '240');
  return { map };
});

const { setUi, ui, lastUsedSidebar } = await import('../../stores/uiStore');
const { loadSession } = await import('../../stores/sessionStore');
const { restoreSidebar, trackSidebar, flushSidebar } = await import('./sidebarState');

describe('sidebar across launches', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('starts a new window from a width saved by an older version', () => {
    expect(ui.sidebarWidth).toBe(240);
    expect(ui.sidebarOpen).toBe(true);
    expect(ui.sidebarMode).toBe('pages');
  });

  it('saves debounced, and a restore is not taken for the last-used sidebar', () => {
    map.delete('speedyf-sidebar');
    restoreSidebar({ open: false, width: 300, mode: 'outline' }, 1400);
    const stop = trackSidebar('main');
    expect(ui.sidebarMode).toBe('outline');

    // A drag: many widths, one write.
    for (let w = 200; w < 260; w += 5) setUi('sidebarWidth', w);
    expect(loadSession().sidebar).toBeUndefined();
    vi.advanceTimersByTime(400);
    expect(loadSession().sidebar).toEqual({ open: false, width: 255, mode: 'outline' });
    expect(lastUsedSidebar()).toEqual({ open: false, width: 255, mode: 'outline' });

    // Putting a window back as it was is not a choice a new window copies.
    map.delete('speedyf-sidebar');
    restoreSidebar({ open: true, width: 200, mode: 'notes' }, 1400);
    vi.advanceTimersByTime(400);
    expect(loadSession().sidebar).toEqual({ open: true, width: 200, mode: 'notes' });
    expect(map.has('speedyf-sidebar')).toBe(false);

    // Closing the window saves what is pending at once.
    setUi('sidebarOpen', false);
    flushSidebar();
    expect(loadSession().sidebar?.open).toBe(false);
    expect(lastUsedSidebar()?.open).toBe(false);
    stop();
  });
});
