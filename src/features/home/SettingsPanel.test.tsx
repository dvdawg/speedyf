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
const backups = [
  {
    file: 'library-20261112-094000.json.gz',
    at: '2026-11-12T09:40:00Z',
    bytes: 15000,
    papers: 120,
    projects: 4,
  },
  {
    file: 'library-20261110-080000.json.gz',
    at: '2026-11-10T08:00:00Z',
    bytes: 14000,
    papers: 1,
    projects: 1,
  },
];
const setLowMemory = vi.fn(async (_on: boolean) => {});
const libraryBackups = vi.fn(async () => backups);
const libraryRestoreBackup = vi.fn(async (_file: string) => {});
const defaults = {
  includeAbstract: false,
  includeUrl: false,
  includeFile: false,
  protectTitle: true,
  abbreviateJournals: false,
};
const librarySetBibtexOptions = vi.fn(async (options: typeof defaults) => options);

vi.mock('../../lib/transport/engine', () => ({
  engine: {
    librarySetLookup: (...args: unknown[]) =>
      librarySetLookup(...(args as [boolean, string, string])),
    libraryDataDir: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    libraryMoveData: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    setLowMemory: (on: boolean) => setLowMemory(on),
    libraryBackups: () => libraryBackups(),
    libraryRestoreBackup: (file: string) => libraryRestoreBackup(file),
    libraryBibtexOptions: vi.fn(async () => ({ ...defaults })),
    librarySetBibtexOptions: (options: typeof defaults) => librarySetBibtexOptions(options),
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

describe('SettingsPanel backups', () => {
  it('shows the last backup and restores one only after a confirm in place', async () => {
    render(() => <SettingsPanel />);
    await waitFor(() => expect(screen.getByText(/^Last backup /)).toBeTruthy());
    expect(screen.queryByRole('region', { name: 'Backups to restore' })).toBeNull();

    fireEvent.click(screen.getByText('Restore...'));
    const list = await screen.findByRole('region', { name: 'Backups to restore' });
    await waitFor(() => expect(list.textContent).toContain('120 items, 4 projects'));
    expect(list.textContent).toContain('1 item, 1 project');

    const buttons = screen.getAllByRole('button', { name: /^Restore the backup from / });
    fireEvent.click(buttons[0]!);
    expect(libraryRestoreBackup).not.toHaveBeenCalled();
    expect(screen.getByText('Replace the library with this backup?')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByText('Replace the library with this backup?')).toBeNull();

    fireEvent.click(screen.getAllByRole('button', { name: /^Restore the backup from / })[1]!);
    fireEvent.click(screen.getByRole('button', { name: 'Replace' }));
    await waitFor(() =>
      expect(libraryRestoreBackup).toHaveBeenCalledWith('library-20261110-080000.json.gz')
    );
    await waitFor(() => expect(screen.getByRole('status').textContent).toMatch(/^Restored/));
  });

  it('says so when there is no backup yet', async () => {
    libraryBackups.mockResolvedValueOnce([]);
    render(() => <SettingsPanel />);
    await waitFor(() => expect(screen.getByText('No backup yet')).toBeTruthy());
  });
});

describe('SettingsPanel BibTeX options', () => {
  it('sit in a closed Advanced disclosure and each switch saves all five', async () => {
    const { container } = render(() => <SettingsPanel />);
    const details = container.querySelector('details.settings-advanced') as HTMLDetailsElement;
    expect(details.open).toBe(false);
    expect(details.querySelector('summary')?.textContent).toBe('Advanced');

    const title = await screen.findByLabelText(/Protect title case/);
    expect((title as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText(/Include abstract/) as HTMLInputElement).checked).toBe(false);

    fireEvent.input(screen.getByLabelText(/Abbreviate journal names/), {
      target: { checked: true },
    });
    await waitFor(() =>
      expect(librarySetBibtexOptions).toHaveBeenCalledWith({
        ...defaults,
        abbreviateJournals: true,
      })
    );
    fireEvent.input(title, { target: { checked: false } });
    await waitFor(() =>
      expect(librarySetBibtexOptions).toHaveBeenLastCalledWith({
        ...defaults,
        abbreviateJournals: true,
        protectTitle: false,
      })
    );
  });
});

// These used to sit in the status bar too; Settings is now their only control
// besides the palette commands, so each one must still save and take effect.
describe('SettingsPanel display settings', () => {
  it('saves the theme, page inversion and low memory', () => {
    updateSettings({ theme: 'system', invertPages: false, lowMemory: false });
    render(() => <SettingsPanel />);

    fireEvent.input(screen.getByLabelText(/Theme/), { target: { value: 'dark' } });
    expect(settings.theme).toBe('dark');

    fireEvent.input(screen.getByLabelText(/Invert page colors/), { target: { checked: true } });
    expect(settings.invertPages).toBe(true);

    fireEvent.input(screen.getByLabelText(/Low memory/), { target: { checked: true } });
    expect(settings.lowMemory).toBe(true);
    expect(setLowMemory).toHaveBeenCalledWith(true);

    updateSettings({ theme: 'system', invertPages: false, lowMemory: false });
  });
});
