// @vitest-environment happy-dom
/**
 * Password protection, in the reader: what a protected file's permissions
 * allow, how the Protect form turns into a save request, and how the
 * document store stages that request and refuses what the permissions
 * withhold. Writing the encryption is covered in src-tauri (engine/protect).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PermissionSet, ProtectionInfo } from '../../types/engine';
import { createDocumentStore } from '../document/documentStore';
import { statusMessage } from '../../stores/statusMessage';
import {
  ALL_PERMISSIONS,
  allows,
  COMMAND_RIGHTS,
  deniedReason,
  isRestricted,
  rightForOp,
} from './permissions';
import {
  describeProtection,
  initialForm,
  passwordStrength,
  requestFrom,
  type ProtectForm,
} from './protectForm';
import { COMMANDS, describeDisabled } from '../../app/commands';
import type { TabRecord } from '../../stores/tabsStore';

const READ_ONLY: PermissionSet = {
  print: 'low',
  copy: false,
  edit: false,
  annotate: false,
  fillForms: true,
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

function openStore(p: ProtectionInfo | undefined) {
  const store = createDocumentStore();
  store.initFromMeta({
    docId: 1,
    path: '/tmp/p.pdf',
    name: 'p.pdf',
    pageCount: 2,
    sizes: [
      [300, 400, 0, 0, 0],
      [300, 400, 0, 0, 0],
    ],
    estimatedSize: [300, 400],
    ...(p ? { protection: p } : {}),
  });
  return store;
}

afterEach(() => vi.restoreAllMocks());

describe('permissions', () => {
  it('allow everything on unprotected or unlocked files', () => {
    for (const p of [
      undefined,
      null,
      protection({ encrypted: false }),
      protection({ ownerUnlocked: true }),
    ]) {
      expect(isRestricted(p)).toBe(false);
      expect(allows(p, 'annotate')).toBe(true);
      expect(allows(p, 'protect')).toBe(true);
    }
  });

  it('withhold what the file withholds, and say why', () => {
    const p = protection();
    expect(isRestricted(p)).toBe(true);
    expect(allows(p, 'print')).toBe(true);
    expect(allows(p, 'copy')).toBe(false);
    expect(allows(p, 'fillForms')).toBe(true);
    expect(allows(p, 'assemble')).toBe(false);
    expect(allows(p, 'protect')).toBe(false);
    expect(deniedReason(p, 'copy')).toBe(
      "Copying text is not allowed by this document's permissions"
    );
    expect(deniedReason(p, 'print')).toBeNull();
    expect(allows(protection({ permissions: { ...READ_ONLY, print: 'none' } }), 'print')).toBe(
      false
    );
  });

  it('count commenting as filling, and editing as assembling', () => {
    const p = protection({
      permissions: { ...READ_ONLY, fillForms: false, annotate: true, edit: true },
    });
    expect(allows(p, 'fillForms')).toBe(true);
    expect(allows(p, 'assemble')).toBe(true);
  });

  it('map each forward edit to the right it needs', () => {
    expect(rightForOp('rotate')).toBe('assemble');
    expect(rightForOp('addAnnot')).toBe('annotate');
    expect(rightForOp('setFormValues')).toBe('fillForms');
    expect(rightForOp('setProtection')).toBe('protect');
    expect(rightForOp('restorePage')).toBeNull();
  });

  it('name only commands that exist', () => {
    const ids = new Set(COMMANDS.map((c) => c.id));
    for (const id of Object.keys(COMMAND_RIGHTS)) expect(ids.has(id), id).toBe(true);
  });
});

describe('commands on a protected document', () => {
  it('are disabled with the reason', () => {
    const store = openStore(protection({ permissions: { ...READ_ONLY, print: 'none' } }));
    const tab = { documentStore: store, viewport: { state: { currentPage: 0 } } };
    const ctx = { tab: tab as unknown as TabRecord };
    const print = COMMANDS.find((c) => c.id === 'file.print')!;
    const rotate = COMMANDS.find((c) => c.id === 'page.rotateRight')!;
    const protect = COMMANDS.find((c) => c.id === 'file.protect')!;
    expect(print.enabled(ctx)).toBe(false);
    expect(describeDisabled(print, ctx)).toBe(
      "Printing is not allowed by this document's permissions"
    );
    expect(rotate.enabled(ctx)).toBe(false);
    // Protect... stays reachable: it is where the permissions password goes.
    expect(protect.enabled(ctx)).toBe(true);

    store.setProtectionInfo(protection({ ownerUnlocked: true }));
    expect(print.enabled(ctx)).toBe(true);
    expect(rotate.enabled(ctx)).toBe(true);
  });
});

describe('the document store', () => {
  it('stages protection as one undoable edit', () => {
    const store = openStore(undefined);
    const request = {
      kind: 'set' as const,
      openPassword: 'open',
      permissionsPassword: '',
      permissions: ALL_PERMISSIONS,
    };
    store.apply({ type: 'setProtection', request });
    expect(store.state.protectChange).toEqual(request);
    expect(store.state.dirty).toBe(true);
    store.undo();
    expect(store.state.protectChange).toBeNull();
    expect(store.state.dirty).toBe(false);
    store.redo();
    expect(store.state.protectChange).toEqual(request);
  });

  it('refuses what the permissions withhold, and says so', () => {
    const store = openStore(protection());
    const page = store.state.pages[0]!;
    store.apply({ type: 'rotate', pageId: page.id, delta: 90 });
    expect(store.state.pages[0]!.userRotation).toBe(0);
    expect(statusMessage()?.text).toBe(
      "Changing pages is not allowed by this document's permissions"
    );
    store.apply({ type: 'setProtection', request: { kind: 'remove' } });
    expect(store.state.protectChange).toBeNull();
    expect(store.state.dirty).toBe(false);

    // Filling the form is allowed by this file.
    store.apply({ type: 'setForm', field: 'name', value: 'Ada' });
    expect(store.state.formEdits['name']).toBe('Ada');

    store.setProtectionInfo(protection({ ownerUnlocked: true }));
    store.apply({ type: 'rotate', pageId: page.id, delta: 90 });
    expect(store.state.pages[0]!.userRotation).toBe(90);
  });
});

describe('the Protect form', () => {
  const filled = (overrides: Partial<ProtectForm>): ProtectForm => ({
    ...initialForm(null, null),
    ...overrides,
  });

  it('starts from the pending change, else the file', () => {
    expect(initialForm(null, null).requireOpen).toBe(false);
    const fromFile = initialForm(protection(), null);
    expect(fromFile).toMatchObject({ requireOpen: true, restrict: true, openPassword: '' });
    expect(fromFile.permissions).toEqual(READ_ONLY);
    const pending = initialForm(protection(), {
      kind: 'set',
      openPassword: 'a',
      permissionsPassword: '',
      permissions: ALL_PERMISSIONS,
    });
    expect(pending).toMatchObject({ requireOpen: true, openPassword: 'a', openConfirm: 'a' });
    expect(pending.restrict).toBe(false);
    expect(initialForm(protection(), { kind: 'remove' }).requireOpen).toBe(false);
  });

  it('asks for matching, distinct passwords', () => {
    const err = (form: ProtectForm) => {
      const r = requestFrom(form, null);
      return r.ok ? null : r.error;
    };
    expect(err(filled({ requireOpen: true }))).toBe('Enter a password to open.');
    expect(err(filled({ requireOpen: true, openPassword: 'a', openConfirm: 'b' }))).toBe(
      'The passwords to open do not match.'
    );
    expect(err(filled({ restrict: true }))).toBe('Enter a permissions password.');
    expect(
      err(
        filled({
          requireOpen: true,
          openPassword: 'same',
          openConfirm: 'same',
          restrict: true,
          permissionsPassword: 'same',
          permissionsConfirm: 'same',
        })
      )
    ).toBe('The permissions password must differ from the password to open.');
  });

  it('turns into a save request', () => {
    const owner = filled({
      restrict: true,
      permissionsPassword: 'owner',
      permissionsConfirm: 'owner',
      permissions: READ_ONLY,
    });
    expect(requestFrom(owner, null)).toEqual({
      ok: true,
      request: {
        kind: 'set',
        openPassword: '',
        permissionsPassword: 'owner',
        permissions: READ_ONLY,
      },
    });
    // An open password alone grants every permission.
    const open = filled({
      requireOpen: true,
      openPassword: 'o',
      openConfirm: 'o',
      permissions: READ_ONLY,
    });
    expect(requestFrom(open, null)).toMatchObject({
      request: { permissionsPassword: '', permissions: ALL_PERMISSIONS },
    });
    // Nothing asked: removed from a protected file, no change to a plain one.
    expect(requestFrom(filled({}), protection())).toEqual({
      ok: true,
      request: { kind: 'remove' },
    });
    expect(requestFrom(filled({}), null)).toEqual({ ok: true, request: null });
  });

  it('rates passwords roughly', () => {
    expect(passwordStrength('abc')).toBe('weak');
    expect(passwordStrength('abcdefg1')).toBe('fair');
    expect(passwordStrength('Correct-horse-9')).toBe('strong');
    expect(passwordStrength('a'.repeat(16))).toBe('strong');
  });

  it('describes the protection in a line', () => {
    expect(describeProtection(null, null)).toBe('This document is not protected.');
    expect(describeProtection(protection(), null)).toBe(
      'Protected with AES-256. Opening it asks for a password. Some actions need its permissions password.'
    );
    expect(describeProtection(protection(), { kind: 'remove' })).toBe(
      'Protection will be removed when you save.'
    );
  });
});
