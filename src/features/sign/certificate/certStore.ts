/** Signing a document with a certificate: the identities to sign with, where
 * the signature goes, and the signing itself.
 *
 * Unlike a visual signature, this changes the file the moment it is made: the
 * signature is appended to the saved file as an incremental update, so any
 * unsaved edits are saved first, and every later save of the document appends
 * after it (src-tauri/src/engine/signing/sign.rs).
 *
 * Identities are personal: in the Keychain on macOS, otherwise in the app
 * data directory with the key encrypted under the user's passphrase. Nothing
 * here goes online unless the user turns on the timestamp in the dialog. */
import { createStore } from 'solid-js/store';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import type { TabRecord } from '../../../stores/tabsStore';
import { activeTab } from '../../../stores/tabsStore';
import { engine, isEngineError } from '../../../lib/transport/engine';
import { flashStatus } from '../../../stores/statusMessage';
import { reloadTabFromDisk, saveDocument } from '../../document/controller';
import { formFields } from '../../forms/formStore';
import type { FormFieldInfo } from '../../../types/engine';
import type { PdfRect } from '../../../types/model';
import { isOpenSignatureField } from '../signStore';
import { certApi, type FieldChoice, type Identity, type SignRequest } from './api';

/** Where the signature goes: an empty signature field already in the form,
 * or a box the user drew. `pageIndex` is the page's position in the
 * document as shown, which is its index in the file once it is saved. */
export type CertTarget =
  | { kind: 'existing'; name: string; pageIndex: number }
  | { kind: 'new'; pageIndex: number; rect: PdfRect };

interface CertState {
  /** The dialog is showing. */
  open: boolean;
  /** The dialog is set aside while the user draws a box on a page. */
  placing: boolean;
  target: CertTarget | null;
  identities: Identity[];
  loaded: boolean;
  busy: boolean;
  error: string | null;
}

const [certState, setCertState] = createStore<CertState>({
  open: false,
  placing: false,
  target: null,
  identities: [],
  loaded: false,
  busy: false,
  error: null,
});
export { certState };

/** macOS keeps identities in the Keychain; elsewhere SpeedyF keeps them. */
export const usesKeychain = () =>
  typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

export async function refreshIdentities(): Promise<void> {
  try {
    const identities = await certApi.identities();
    setCertState({ identities, loaded: true });
  } catch (e) {
    setCertState({
      identities: [],
      loaded: true,
      error: `Could not list identities: ${String(e)}`,
    });
  }
}

export function setCertError(error: string | null): void {
  setCertState('error', error);
}

export function openCertDialog(target: CertTarget | null = null): void {
  if (!activeTab()?.documentStore.state.loaded) return;
  setCertState({ open: true, placing: false, target, error: null });
  void refreshIdentities();
}

export function closeCertDialog(): void {
  window.removeEventListener('keydown', onPlacingKey, true);
  setCertState({ open: false, placing: false, target: null, busy: false, error: null });
}

// ---------- choosing where the signature goes ----------

function onPlacingKey(e: KeyboardEvent) {
  if (e.key !== 'Escape') return;
  e.preventDefault();
  e.stopPropagation();
  cancelPlacing();
}

/** Set the dialog aside so the user can draw a box on a page. */
export function startPlacing(): void {
  setCertState({ open: false, placing: true, error: null });
  window.addEventListener('keydown', onPlacingKey, true);
  flashStatus('Drag a box for the signature, or click an empty signature field. Esc goes back.', {
    ms: 8_000,
  });
}

export function finishPlacing(target: CertTarget): void {
  window.removeEventListener('keydown', onPlacingKey, true);
  setCertState({ open: true, placing: false, target });
}

export function cancelPlacing(): void {
  window.removeEventListener('keydown', onPlacingKey, true);
  setCertState({ open: true, placing: false });
}

export function setCertTarget(target: CertTarget | null): void {
  setCertState('target', target);
}

/** Empty signature fields a certificate signature can go in. */
export async function openSignatureFields(docId: number): Promise<FormFieldInfo[]> {
  const fields = await formFields(docId);
  return fields.filter(isOpenSignatureField);
}

/** Fields a signature can lock: everything but signature fields. */
export async function lockableFields(docId: number): Promise<string[]> {
  const fields = await formFields(docId);
  const names = fields.filter((f) => f.kind !== 'signature' && f.name).map((f) => f.name);
  return [...new Set(names)];
}

/** The smallest box a signature is drawn into, in points. */
export const MIN_BOX = 12;

/** A box from two corners of a drag, in page points. Null when too small. */
export function boxFrom(
  a: { x: number; y: number },
  b: { x: number; y: number },
  page: { w: number; h: number }
): PdfRect | null {
  const clamp = (v: number, max: number) => Math.min(max, Math.max(0, v));
  const x0 = clamp(Math.min(a.x, b.x), page.w);
  const x1 = clamp(Math.max(a.x, b.x), page.w);
  const y0 = clamp(Math.min(a.y, b.y), page.h);
  const y1 = clamp(Math.max(a.y, b.y), page.h);
  if (x1 - x0 < MIN_BOX || y1 - y0 < MIN_BOX) return null;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** A box for a single click: the usual size of a signature, centred on the
 * click and kept on the page. */
export function boxAt(at: { x: number; y: number }, page: { w: number; h: number }): PdfRect {
  const w = Math.min(180, page.w);
  const h = Math.min(54, page.h);
  const x = Math.min(Math.max(0, at.x - w / 2), page.w - w);
  const y = Math.min(Math.max(0, at.y - h / 2), page.h - h);
  return { x, y, w, h };
}

export function fieldChoice(target: CertTarget): FieldChoice {
  return target.kind === 'existing'
    ? { kind: 'existing', name: target.name }
    : { kind: 'new', page: target.pageIndex, rect: { ...target.rect } };
}

// ---------- the date the appearance shows ----------

const two = (n: number) => String(n).padStart(2, '0');

/** `2026.11.17 14:03:22 -08'00'`: the form Acrobat prints in a signature. */
export function formatSignDate(d: Date): string {
  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? '+' : '-';
  const abs = Math.abs(offset);
  return (
    `${d.getFullYear()}.${two(d.getMonth() + 1)}.${two(d.getDate())} ` +
    `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())} ` +
    `${sign}${two(Math.floor(abs / 60))}'${two(abs % 60)}'`
  );
}

// ---------- identities ----------

export async function createIdentity(args: {
  name: string;
  email: string;
  organization: string;
  passphrase: string | null;
}): Promise<Identity> {
  const made = await certApi.create({
    ...args,
    store: usesKeychain() ? 'keychain' : 'file',
    passphrase: usesKeychain() ? null : args.passphrase,
  });
  await refreshIdentities();
  return made;
}

export async function importIdentity(args: {
  path: string;
  password: string;
  passphrase: string | null;
}): Promise<Identity> {
  const made = await certApi.importP12({
    ...args,
    store: usesKeychain() ? 'keychain' : 'file',
    passphrase: usesKeychain() ? null : args.passphrase,
  });
  await refreshIdentities();
  return made;
}

export async function removeIdentity(id: string): Promise<void> {
  await certApi.remove(id);
  await refreshIdentities();
}

// ---------- signing ----------

export interface SignOptions {
  identity: string;
  passphrase: string | null;
  /** Everything but the field and the date, which come from here. */
  request: Omit<SignRequest, 'field' | 'dateText'>;
  /** Write the signed copy to a new file instead of this one. */
  saveAs: boolean;
}

/** An engine error's own words, without its code. */
export function errorText(e: unknown): string {
  if (isEngineError(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

function withPdf(path: string): string {
  return path.toLowerCase().endsWith('.pdf') ? path : `${path}.pdf`;
}

function signedName(path: string): string {
  return path.replace(/(\.pdf)?$/i, ' (signed).pdf');
}

/** Sign the tab's document. Saves pending edits first, asks where to save
 * when it should, appends the signature, and reopens the signed file.
 * Returns whether it signed. */
export async function signDocument(
  tab: TabRecord | null | undefined,
  opts: SignOptions,
  now: Date = new Date()
): Promise<boolean> {
  const store = tab?.documentStore;
  const target = certState.target;
  if (!tab || !store?.state.loaded || certState.busy) return false;
  if (!target) {
    setCertError('Choose a signature field, or draw a box on the page.');
    return false;
  }
  const original = store.state.path;
  let dest = original;
  if (opts.saveAs || !original) {
    const picked = await saveDialog({
      defaultPath: signedName(original ?? store.state.name ?? 'document.pdf'),
      filters: [{ name: 'PDF documents', extensions: ['pdf'] }],
    });
    if (!picked) return false;
    dest = withPdf(picked);
  }
  if (!dest) return false;

  setCertState({ busy: true, error: null });
  try {
    let src = original;
    let docId = store.state.docId;
    if (store.state.dirty || !src) {
      if (dest === src) {
        // Pending edits go into this file first; the signature follows them.
        if (!(await saveDocument(tab, false))) return false;
        docId = store.state.docId;
        src = store.state.path;
      } else {
        // Into the new file first, leaving this one as it was.
        await engine.saveDocument(docId, store.buildEditPlan(), dest);
        src = dest;
      }
    }
    if (!src) return false;
    const request: SignRequest = {
      ...opts.request,
      field: fieldChoice(target),
      dateText: formatSignDate(now),
    };
    let result;
    try {
      result = await certApi.sign({
        docId,
        srcPath: src,
        destPath: dest,
        identity: opts.identity,
        passphrase: opts.passphrase,
        request,
      });
    } catch (e) {
      // Replacing the open file closes it first; if the replace itself
      // failed, reopen the file, which is still the one from before.
      if (errorText(e).includes('cannot replace') && dest === store.state.path) {
        await reloadTabFromDisk(tab, dest).catch(() => undefined);
      }
      throw e;
    }
    store.markSaved();
    await reloadTabFromDisk(tab, result.path);
    setCertState({ open: false, placing: false, target: null });
    flashStatus(result.timestamped ? 'Signed, with a timestamp' : 'Signed');
    return true;
  } catch (e) {
    setCertError(`Could not sign: ${errorText(e)}`);
    return false;
  } finally {
    setCertState('busy', false);
  }
}

/** Tests only. */
export function resetCertStore(): void {
  window.removeEventListener('keydown', onPlacingKey, true);
  setCertState({
    open: false,
    placing: false,
    target: null,
    identities: [],
    loaded: false,
    busy: false,
    error: null,
  });
}
