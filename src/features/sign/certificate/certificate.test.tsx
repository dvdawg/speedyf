// @vitest-environment happy-dom
/**
 * Certificate signing from the outside: the geometry of a drawn box, the
 * date a signature shows, the order the signing flow saves and signs in, and
 * the dialog's guard rails. The engine side - the PDF update, the CMS, the
 * identities - is tested in src-tauri/src/engine/signing.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { testField } from '../../forms/testField';
import type { FormFieldInfo } from '../../../types/engine';
import type { TabRecord } from '../../../stores/tabsStore';
import type * as formStoreModule from '../../forms/formStore';
import type * as engineModule from '../../../lib/transport/engine';
import type * as tabsModule from '../../../stores/tabsStore';
import type { Identity, SignRequest } from './api';

const api = vi.hoisted(() => ({
  identities: [] as Identity[],
  sign: vi.fn(),
  create: vi.fn(),
  importP12: vi.fn(),
}));
vi.mock('./api', () => ({
  certApi: {
    identities: () => Promise.resolve(api.identities),
    sign: api.sign,
    create: api.create,
    importP12: api.importP12,
    remove: () => Promise.resolve(),
  },
}));

const controller = vi.hoisted(() => ({
  calls: [] as string[],
  saveDocument: vi.fn(),
  reloadTabFromDisk: vi.fn(),
}));
vi.mock('../../document/controller', () => ({
  saveDocument: controller.saveDocument,
  reloadTabFromDisk: controller.reloadTabFromDisk,
}));

const engineSave = vi.hoisted(() => vi.fn());
vi.mock('../../../lib/transport/engine', async (importOriginal) => {
  const actual = await importOriginal<typeof engineModule>();
  return { ...actual, engine: { ...actual.engine, saveDocument: engineSave } };
});

const dialog = vi.hoisted(() => ({ saveAs: null as string | null }));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: () => Promise.resolve(null),
  save: () => Promise.resolve(dialog.saveAs),
}));

let fields: FormFieldInfo[] = [];
vi.mock('../../forms/formStore', async (importOriginal) => {
  const actual = await importOriginal<typeof formStoreModule>();
  return { ...actual, formFields: () => Promise.resolve(fields) };
});

let currentTab: TabRecord | null = null;
vi.mock('../../../stores/tabsStore', async (importOriginal) => {
  const actual = await importOriginal<typeof tabsModule>();
  return { ...actual, activeTab: () => currentTab };
});

import {
  boxAt,
  boxFrom,
  certState,
  fieldChoice,
  finishPlacing,
  formatSignDate,
  openCertDialog,
  resetCertStore,
  setCertTarget,
  signDocument,
  startPlacing,
  type SignOptions,
} from './certStore';
import CertSignDialog from './CertSignDialog';
import { createDocumentStore } from '../../document/documentStore';

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const identity = (over: Partial<Identity> = {}): Identity => ({
  id: 'file:abc',
  store: 'file',
  name: 'Ada Lovelace',
  email: 'ada@example.org',
  organization: '',
  issuer: 'Ada Lovelace',
  notBefore: 1_700_000_000,
  notAfter: 4_000_000_000,
  selfSigned: true,
  needsPassphrase: true,
  ...over,
});

function newTab(path: string | null = '/tmp/contract.pdf'): TabRecord {
  const documentStore = createDocumentStore();
  documentStore.initFromMeta({
    docId: 7,
    path,
    name: 'contract.pdf',
    pageCount: 2,
    sizes: [
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
    ],
    estimatedSize: [612, 792],
  });
  return { id: 'tab-1', documentStore } as unknown as TabRecord;
}

const options = (over: Partial<SignOptions> = {}): SignOptions => ({
  identity: 'file:abc',
  passphrase: 'pw',
  saveAs: false,
  request: {
    reason: 'Approved',
    location: '',
    contact: '',
    art: null,
    certify: null,
    lock: { kind: 'none' },
    subFilter: 'cades',
    tsaUrl: null,
  },
  ...over,
});

beforeEach(() => {
  api.identities = [];
  api.sign.mockReset();
  api.sign.mockImplementation((args: { destPath: string }) => {
    controller.calls.push('sign');
    return Promise.resolve({ path: args.destPath, bytes: 1, timestamped: false });
  });
  controller.calls = [];
  controller.saveDocument.mockReset();
  controller.saveDocument.mockImplementation((tab: TabRecord) => {
    controller.calls.push('save');
    tab.documentStore.markSaved();
    return Promise.resolve(true);
  });
  controller.reloadTabFromDisk.mockReset();
  controller.reloadTabFromDisk.mockImplementation((_tab: TabRecord, dest: string) => {
    controller.calls.push(`reload ${dest}`);
    return Promise.resolve();
  });
  engineSave.mockReset();
  engineSave.mockImplementation((_id: number, _plan: unknown, dest: string) => {
    controller.calls.push(`save-to ${dest}`);
    return Promise.resolve({});
  });
  dialog.saveAs = null;
  fields = [];
  currentTab = null;
  resetCertStore();
});

afterEach(() => cleanup());

describe('drawing the box', () => {
  it('makes a box from a drag in any direction, kept on the page', () => {
    const page = { w: 612, h: 792 };
    expect(boxFrom({ x: 300, y: 100 }, { x: 100, y: 160 }, page)).toEqual({
      x: 100,
      y: 100,
      w: 200,
      h: 60,
    });
    expect(boxFrom({ x: 600, y: 10 }, { x: 700, y: -20 }, page)).toBeNull();
    expect(boxFrom({ x: 550, y: 700 }, { x: 900, y: 900 }, page)).toEqual({
      x: 550,
      y: 700,
      w: 62,
      h: 92,
    });
    expect(boxFrom({ x: 10, y: 10 }, { x: 15, y: 40 }, page)).toBeNull();
  });

  it('gives a click a signature-sized box that stays on the page', () => {
    const box = boxAt({ x: 5, y: 790 }, { w: 612, h: 792 });
    expect(box.x).toBe(0);
    expect(box.y + box.h).toBe(792);
    expect(box.w).toBeGreaterThan(box.h);
  });

  it('turns a target into what the engine is asked for', () => {
    expect(fieldChoice({ kind: 'existing', name: 'Approver', pageIndex: 1 })).toEqual({
      kind: 'existing',
      name: 'Approver',
    });
    expect(fieldChoice({ kind: 'new', pageIndex: 1, rect: { x: 1, y: 2, w: 30, h: 40 } })).toEqual({
      kind: 'new',
      page: 1,
      rect: { x: 1, y: 2, w: 30, h: 40 },
    });
  });

  it('sets the dialog aside while placing and brings it back with the box', () => {
    currentTab = newTab();
    openCertDialog();
    startPlacing();
    expect(certState.open).toBe(false);
    expect(certState.placing).toBe(true);
    finishPlacing({ kind: 'new', pageIndex: 0, rect: { x: 1, y: 2, w: 30, h: 40 } });
    expect(certState.open).toBe(true);
    expect(certState.placing).toBe(false);
    expect(certState.target?.kind).toBe('new');
  });
});

describe('the date a signature shows', () => {
  it('reads as year.month.day time and offset', () => {
    const text = formatSignDate(new Date(2026, 10, 7, 9, 5, 3));
    expect(text).toMatch(/^2026\.11\.07 09:05:03 [+-]\d\d'\d\d'$/);
  });
});

describe('signing', () => {
  it('signs a clean document in place and reopens it', async () => {
    const tab = newTab();
    setCertTarget({ kind: 'new', pageIndex: 1, rect: { x: 10, y: 20, w: 150, h: 50 } });
    expect(await signDocument(tab, options(), new Date(2026, 0, 2))).toBe(true);
    expect(controller.calls).toEqual(['sign', 'reload /tmp/contract.pdf']);
    const args = api.sign.mock.calls[0]![0] as {
      docId: number;
      srcPath: string;
      destPath: string;
      passphrase: string;
      request: SignRequest;
    };
    expect(args.docId).toBe(7);
    expect(args.srcPath).toBe('/tmp/contract.pdf');
    expect(args.destPath).toBe('/tmp/contract.pdf');
    expect(args.passphrase).toBe('pw');
    expect(args.request.field).toEqual({
      kind: 'new',
      page: 1,
      rect: { x: 10, y: 20, w: 150, h: 50 },
    });
    expect(args.request.dateText.startsWith('2026.01.02 00:00:00')).toBe(true);
    expect(args.request.tsaUrl).toBeNull();
  });

  it('saves pending edits before signing in place', async () => {
    const tab = newTab();
    tab.documentStore.apply({ type: 'setFlattenSignatures', value: true });
    expect(tab.documentStore.state.dirty).toBe(true);
    setCertTarget({ kind: 'existing', name: 'Approver', pageIndex: 0 });
    expect(await signDocument(tab, options())).toBe(true);
    expect(controller.calls).toEqual(['save', 'sign', 'reload /tmp/contract.pdf']);
  });

  it('writes pending edits into the new file, not this one, when saving as', async () => {
    const tab = newTab();
    tab.documentStore.apply({ type: 'setFlattenSignatures', value: true });
    dialog.saveAs = '/tmp/contract (signed)';
    setCertTarget({ kind: 'existing', name: 'Approver', pageIndex: 0 });
    expect(await signDocument(tab, options({ saveAs: true }))).toBe(true);
    expect(controller.calls).toEqual([
      'save-to /tmp/contract (signed).pdf',
      'sign',
      'reload /tmp/contract (signed).pdf',
    ]);
    const args = api.sign.mock.calls[0]![0] as { srcPath: string; destPath: string };
    expect(args.srcPath).toBe('/tmp/contract (signed).pdf');
    expect(args.destPath).toBe('/tmp/contract (signed).pdf');
  });

  it('does nothing when the save dialog is cancelled', async () => {
    const tab = newTab();
    setCertTarget({ kind: 'existing', name: 'Approver', pageIndex: 0 });
    expect(await signDocument(tab, options({ saveAs: true }))).toBe(false);
    expect(controller.calls).toEqual([]);
  });

  it('asks for a place first, and reports what the engine said', async () => {
    const tab = newTab();
    expect(await signDocument(tab, options())).toBe(false);
    expect(certState.error).toMatch(/signature field/);
    setCertTarget({ kind: 'existing', name: 'Approver', pageIndex: 0 });
    api.sign.mockImplementation(() =>
      Promise.reject({ code: 'unsupported', message: 'the passphrase is not right' })
    );
    expect(await signDocument(tab, options())).toBe(false);
    expect(certState.error).toBe('Could not sign: the passphrase is not right');
    expect(controller.reloadTabFromDisk).not.toHaveBeenCalled();
  });
});

describe('the dialog', () => {
  it('signs only once there is an identity, a place and the passphrase', async () => {
    currentTab = newTab();
    api.identities = [identity()];
    fields = [testField({ name: 'Approver', kind: 'signature', page: 0, signed: false })];
    openCertDialog();
    const view = render(() => <CertSignDialog />);
    await tick();
    await tick();
    const signBtn = view.getByRole('button', { name: 'Sign' }) as HTMLButtonElement;
    expect(signBtn.disabled).toBe(true);
    expect(view.getByText('Ada Lovelace')).toBeTruthy();

    fireEvent.click(view.getByRole('button', { name: /Approver/ }));
    expect(certState.target).toEqual({ kind: 'existing', name: 'Approver', pageIndex: 0 });
    expect(signBtn.disabled).toBe(true);

    fireEvent.input(view.getByLabelText('Passphrase'), { target: { value: 'pw' } });
    expect(signBtn.disabled).toBe(false);

    // The online timestamp is off until asked for, and needs an address.
    const stamp = view.getByLabelText(/connects to the internet/) as HTMLInputElement;
    expect(stamp.checked).toBe(false);
    fireEvent.click(stamp);
    expect(signBtn.disabled).toBe(true);
    fireEvent.input(view.getByLabelText('Time-stamping server'), {
      target: { value: 'https://tsa.example.org' },
    });
    expect(signBtn.disabled).toBe(false);
    fireEvent.click(stamp);
  });

  it('offers to make or import an identity when there is none', async () => {
    currentTab = newTab();
    openCertDialog();
    const view = render(() => <CertSignDialog />);
    await tick();
    expect(view.getByText('No signing identity yet.')).toBeTruthy();
    fireEvent.click(view.getByRole('button', { name: 'Create self-signed...' }));
    expect(view.getByLabelText('Name')).toBeTruthy();
    fireEvent.click(view.getByRole('button', { name: 'Import .p12...' }));
    expect(view.getByLabelText('File password')).toBeTruthy();
  });
});
