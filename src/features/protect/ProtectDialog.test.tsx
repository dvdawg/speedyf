// @vitest-environment happy-dom
/**
 * The Protect dialog and the save it leads to: staging new protection,
 * refusing bad passwords, unlocking a restricted file with its permissions
 * password, warning before a signed file is rewritten, and handing the
 * request to the engine at save with the reopen password coming back.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PermissionSet, ProtectionInfo } from '../../types/engine';
import type { TabRecord } from '../../stores/tabsStore';
import { createDocumentStore } from '../document/documentStore';
import { engine } from '../../lib/transport/engine';
import { recentStore } from '../../stores/recentStore';
import type * as modalStoreModule from '../../stores/modalStore';

const confirm = vi.hoisted(() => ({ answer: true, asked: [] as string[] }));
vi.mock('../../stores/modalStore', async (importOriginal) => {
  const actual = await importOriginal<typeof modalStoreModule>();
  return {
    ...actual,
    askConfirm: (message: string) => {
      confirm.asked.push(message);
      return Promise.resolve(confirm.answer);
    },
  };
});

import ProtectDialog from './ProtectDialog';
import { openProtect, protectTab } from './protectStore';
import { saveDocument } from '../document/controller';

const READ_ONLY: PermissionSet = {
  print: 'none',
  copy: false,
  edit: false,
  annotate: false,
  fillForms: false,
  assemble: false,
  accessibility: true,
};

function protection(overrides: Partial<ProtectionInfo> = {}): ProtectionInfo {
  return {
    encrypted: true,
    revision: 6,
    method: 'AES-256',
    openPassword: true,
    ownerUnlocked: false,
    permissions: READ_ONLY,
    signatures: 0,
    ...overrides,
  };
}

function makeTab(p?: ProtectionInfo): TabRecord {
  const documentStore = createDocumentStore();
  documentStore.initFromMeta({
    docId: 7,
    path: '/tmp/doc.pdf',
    name: 'doc.pdf',
    pageCount: 1,
    sizes: [[300, 400, 0, 0, 0]],
    estimatedSize: [300, 400],
    ...(p ? { protection: p } : {}),
  });
  return {
    documentStore,
    viewport: { state: { currentPage: 0 }, requestScrollToPage: vi.fn() },
    searchStore: { resetForDocument: vi.fn() },
    citationStore: { syncDocument: vi.fn() },
    structureStore: { syncDocument: vi.fn() },
  } as unknown as TabRecord;
}

function show(tab: TabRecord) {
  openProtect(tab);
  return render(() => <ProtectDialog tab={tab} />);
}

const type = (label: string, value: string) =>
  fireEvent.input(screen.getByLabelText(label), { target: { value } });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  confirm.answer = true;
  confirm.asked = [];
});

describe('Protect dialog', () => {
  it('stages a password to open and a permissions password', async () => {
    const tab = makeTab();
    show(tab);
    screen.getByText('This document is not protected.');
    fireEvent.click(screen.getByLabelText('Require a password to open'));
    type('Password to open', 'open-sesame');
    type('Confirm password to open', 'open-sesame');
    screen.getByText('Strength: fair');
    fireEvent.click(screen.getByLabelText('Restrict what others can do'));
    type('Permissions password', 'owner-pass');
    type('Confirm permissions password', 'owner-pass');
    fireEvent.change(screen.getByLabelText('Printing'), { target: { value: 'low' } });
    fireEvent.click(screen.getByLabelText('Copy text and images'));
    fireEvent.click(screen.getByText('Apply'));

    await waitFor(() => expect(protectTab()).toBeNull());
    const change = tab.documentStore.state.protectChange;
    expect(change).toMatchObject({
      kind: 'set',
      openPassword: 'open-sesame',
      permissionsPassword: 'owner-pass',
    });
    expect(change?.kind === 'set' && change.permissions).toMatchObject({
      print: 'low',
      copy: false,
      annotate: true,
    });
    expect(tab.documentStore.state.dirty).toBe(true);
  });

  it('says what is wrong with the passwords and keeps the dialog open', () => {
    const tab = makeTab();
    show(tab);
    fireEvent.click(screen.getByLabelText('Require a password to open'));
    type('Password to open', 'one');
    type('Confirm password to open', 'two');
    fireEvent.click(screen.getByText('Apply'));
    expect(screen.getByRole('alert').textContent).toBe('The passwords to open do not match.');
    expect(protectTab()).toBe(tab);
    expect(tab.documentStore.state.protectChange).toBeNull();
  });

  it('asks for the permissions password before anything on a restricted file', async () => {
    const tab = makeTab(protection());
    const unlock = vi
      .spyOn(engine, 'unlockPermissions')
      .mockRejectedValueOnce({ code: 'password', message: 'this PDF is password-protected' })
      .mockResolvedValueOnce(protection({ ownerUnlocked: true }));
    show(tab);
    expect(screen.queryByLabelText('Require a password to open')).toBeNull();

    type('Permissions password', 'guess');
    fireEvent.click(screen.getByText('Unlock'));
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toBe('That is not the permissions password.')
    );

    type('Permissions password', 'owner');
    fireEvent.click(screen.getByText('Unlock'));
    await waitFor(() => screen.getByLabelText('Require a password to open'));
    expect(unlock).toHaveBeenLastCalledWith(7, 'owner');
    expect(tab.documentStore.state.protection?.ownerUnlocked).toBe(true);
    // The form starts from the file's own protection.
    expect((screen.getByLabelText('Restrict what others can do') as HTMLInputElement).checked).toBe(
      true
    );

    fireEvent.click(screen.getByText('Remove protection'));
    await waitFor(() => expect(tab.documentStore.state.protectChange).toEqual({ kind: 'remove' }));
  });

  it('warns before a signed file is rewritten, and backs off on cancel', async () => {
    const tab = makeTab(protection({ ownerUnlocked: true, signatures: 1 }));
    show(tab);
    screen.getByText(/signatures will no longer verify/);
    confirm.answer = false;
    fireEvent.click(screen.getByText('Remove protection'));
    await waitFor(() => expect(confirm.asked).toHaveLength(1));
    expect(confirm.asked[0]).toMatch(/1 digital signature\./);
    expect(tab.documentStore.state.protectChange).toBeNull();
    expect(protectTab()).toBe(tab);
  });
});

describe('saving with protection', () => {
  it('hands the request to the engine and reopens with the password it returns', async () => {
    const tab = makeTab();
    const request = {
      kind: 'set' as const,
      openPassword: 'new',
      permissionsPassword: '',
      permissions: {
        print: 'high' as const,
        copy: true,
        edit: true,
        annotate: true,
        fillForms: true,
        assemble: true,
        accessibility: true,
      },
    };
    tab.documentStore.apply({ type: 'setProtection', request });
    const save = vi.spyOn(engine, 'saveDocument').mockResolvedValue({
      path: '/tmp/doc.pdf',
      bytes: 10,
      durationMs: 1,
      reopenPassword: 'new',
    });
    const open = vi.spyOn(engine, 'open').mockResolvedValue({
      docId: 8,
      path: '/tmp/doc.pdf',
      name: 'doc.pdf',
      pageCount: 1,
      sizes: [[300, 400, 0, 0, 0]],
      estimatedSize: [300, 400],
      protection: protection({ ownerUnlocked: true, permissions: request.permissions }),
    });
    vi.spyOn(engine, 'getAnnotations').mockResolvedValue([]);
    vi.spyOn(engine, 'close').mockResolvedValue();
    vi.spyOn(recentStore, 'recordOpen').mockImplementation(() => {});

    expect(await saveDocument(tab, false)).toBe(true);
    expect(save.mock.calls[0]![3]).toEqual(request);
    expect(open).toHaveBeenCalledWith('/tmp/doc.pdf', 'new');
    // The saved file is the new baseline: protected, nothing staged.
    expect(tab.documentStore.state.protectChange).toBeNull();
    expect(tab.documentStore.state.protection?.encrypted).toBe(true);
  });

  it('keeps the file protection by default', async () => {
    const tab = makeTab();
    tab.documentStore.apply({ type: 'setForm', field: 'x', value: 'y' });
    const save = vi.spyOn(engine, 'saveDocument').mockResolvedValue({
      path: '/tmp/doc.pdf',
      bytes: 10,
      durationMs: 1,
      reopenPassword: null,
    });
    const open = vi.spyOn(engine, 'open').mockResolvedValue({
      docId: 9,
      path: '/tmp/doc.pdf',
      name: 'doc.pdf',
      pageCount: 1,
      sizes: [[300, 400, 0, 0, 0]],
      estimatedSize: [300, 400],
    });
    vi.spyOn(engine, 'getAnnotations').mockResolvedValue([]);
    vi.spyOn(engine, 'close').mockResolvedValue();
    vi.spyOn(recentStore, 'recordOpen').mockImplementation(() => {});

    expect(await saveDocument(tab, false)).toBe(true);
    expect(save.mock.calls[0]![3]).toBeUndefined();
    expect(open).toHaveBeenCalledWith('/tmp/doc.pdf', undefined);
  });
});
