// @vitest-environment happy-dom
/**
 * The lookup settings have an effect beyond the store: the engine keeps its
 * own copy and decides per request whether anything may go out. So every
 * change to any of the three — the switch, the contact, the Semantic Scholar
 * key — must reach it as one call carrying all three, or the engine is left
 * running on a stale pair.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const librarySetLookup = vi.fn(async (_enabled: boolean, _contact: string, _key: string) => {});

vi.mock('../../lib/transport/engine', () => ({
  engine: {
    librarySetLookup: (...args: unknown[]) =>
      librarySetLookup(...(args as [boolean, string, string])),
    libraryDataDir: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    libraryMoveData: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    setLowMemory: vi.fn(async () => {}),
  },
  isEngineError: (e: unknown) => typeof e === 'object' && e !== null && 'code' in e,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));

const { default: SettingsPanel } = await import('./SettingsPanel');
const { settings, updateSettings } = await import('../../stores/settings');

beforeEach(() => updateSettings({ onlineLookup: false, lookupContact: '', s2Key: '' }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('SettingsPanel lookups', () => {
  it('says what leaves the machine', () => {
    render(() => <SettingsPanel />);
    expect(
      screen.getByText(
        /Sends DOIs and arXiv ids, and for items without either, their first-page title/
      )
    ).toBeTruthy();
    expect(screen.getByText(/Semantic Scholar, INSPIRE, DBLP and Open Library/)).toBeTruthy();
    expect(screen.getByText(/a better rate limit, nothing else/)).toBeTruthy();
  });

  it('saves the Semantic Scholar key and hands all three settings to the engine', async () => {
    render(() => <SettingsPanel />);
    const key = screen.getByLabelText(/Semantic Scholar API key/) as HTMLInputElement;
    expect(key.value).toBe('');
    fireEvent.change(key, { target: { value: '  s2-abc123 ' } });
    expect(settings.s2Key).toBe('s2-abc123');
    await waitFor(() => expect(librarySetLookup).toHaveBeenCalledWith(false, '', 's2-abc123'));
    expect(librarySetLookup).toHaveBeenCalledTimes(1);
  });

  it('passes the key along when the switch or the contact changes', async () => {
    updateSettings({ s2Key: 'kept' });
    render(() => <SettingsPanel />);
    fireEvent.input(screen.getByLabelText(/Look up metadata online/), {
      target: { checked: true },
    });
    await waitFor(() => expect(librarySetLookup).toHaveBeenCalledWith(true, '', 'kept'));

    fireEvent.change(screen.getByLabelText(/Contact for lookups/), {
      target: { value: 'you@example.edu' },
    });
    await waitFor(() =>
      expect(librarySetLookup).toHaveBeenCalledWith(true, 'you@example.edu', 'kept')
    );
    expect(librarySetLookup).toHaveBeenCalledTimes(2);
  });
});
