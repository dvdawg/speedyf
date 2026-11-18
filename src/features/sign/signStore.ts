/** Visual signatures: the user's saved signatures and initials, the create
 * dialog, and placing one on a page.
 *
 * Saved signatures are personal. They live in one file in the app data
 * directory (src-tauri/src/commands/signatures.rs), never in the library and
 * never in anything synced or exported. A placed signature is an ordinary
 * annotation of kind 'signature', so moving, resizing, deleting and undo all
 * go through the document store like every other edit.
 *
 * These are pictures of a signature. Nothing here signs a file with a
 * certificate, and the UI says so wherever a signature field is involved. */
import { createStore, produce } from 'solid-js/store';
import type { DocumentStore } from '../document/documentStore';
import type { PageEntry, PageId, PdfPoint, PdfRect, SignatureArt } from '../../types/model';
import { signaturesApi } from '../../lib/transport/signatures';
import { newAnnotId } from '../editor/editorActions';
import { setActiveTool } from '../annotations/toolStore';
import { activeTab } from '../../stores/tabsStore';
import { fitIntoField, placementRect, type SignatureRole } from './art';
import { formFields } from '../forms/formStore';
import type { FormFieldInfo } from '../../types/engine';

export interface SavedSignature {
  id: string;
  role: SignatureRole;
  art: SignatureArt;
  /** ink colour for drawn and typed artwork; ignored by images */
  color: string;
  createdAt: number;
}

/** Where a signature goes once the dialog has made one: a form field the
 * user clicked before any signature existed. */
export interface FieldTarget {
  store: DocumentStore;
  pageId: PageId;
  rect: PdfRect;
}

interface SignState {
  loaded: boolean;
  items: SavedSignature[];
  /** Armed for placement: the next click on a page puts it there. */
  pending: SavedSignature | null;
  dialog: { role: SignatureRole } | null;
  menuOpen: boolean;
}

const [signState, setSignState] = createStore<SignState>({
  loaded: false,
  items: [],
  pending: null,
  dialog: null,
  menuOpen: false,
});
export { signState };

/** Pending until the dialog saves or is cancelled. */
let dialogTarget: FieldTarget | null = null;
let loading: Promise<void> | null = null;

interface StoredFile {
  v: 1;
  items: SavedSignature[];
}

function isSaved(value: unknown): value is SavedSignature {
  const v = value as Partial<SavedSignature> | null;
  return (
    !!v &&
    typeof v.id === 'string' &&
    (v.role === 'signature' || v.role === 'initials') &&
    typeof v.color === 'string' &&
    !!v.art &&
    typeof v.art.w === 'number' &&
    typeof v.art.h === 'number' &&
    ['drawn', 'typed', 'image'].includes(v.art.type)
  );
}

/** Read the saved signatures once. A file that cannot be read leaves the list
 * empty rather than failing anything: signing still works, it simply starts
 * from nothing. */
export function loadSignatures(): Promise<void> {
  if (signState.loaded) return Promise.resolve();
  loading ??= signaturesApi
    .load()
    .then((json) => {
      const parsed = json ? (JSON.parse(json) as Partial<StoredFile>) : null;
      const items = Array.isArray(parsed?.items) ? parsed.items.filter(isSaved) : [];
      setSignState({ loaded: true, items });
    })
    .catch(() => {
      setSignState({ loaded: true, items: [] });
    })
    .finally(() => {
      loading = null;
    });
  return loading;
}

async function persist(): Promise<void> {
  const file: StoredFile = { v: 1, items: signState.items };
  await signaturesApi.save(JSON.stringify(file));
}

export async function addSignature(
  saved: Omit<SavedSignature, 'id' | 'createdAt'>
): Promise<SavedSignature> {
  await loadSignatures();
  const item: SavedSignature = {
    ...saved,
    id: `sig-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
    createdAt: Date.now(),
  };
  setSignState(
    produce((s) => {
      s.items.unshift(item);
    })
  );
  await persist();
  return item;
}

export async function removeSignature(id: string): Promise<void> {
  if (signState.pending?.id === id) cancelPlacement();
  setSignState(
    produce((s) => {
      s.items = s.items.filter((item) => item.id !== id);
    })
  );
  await persist();
}

export const signaturesOf = (role: SignatureRole) =>
  signState.items.filter((item) => item.role === role);

/** The one to use when nothing says which: the newest of its kind. */
export const defaultSignature = (role: SignatureRole = 'signature') =>
  signaturesOf(role)[0] ?? null;

// ---------- placement ----------

/** Escape puts an armed signature back down without placing it. */
function onPlacementKey(e: KeyboardEvent) {
  if (e.key !== 'Escape') return;
  e.preventDefault();
  e.stopPropagation();
  cancelPlacement();
}

export function armPlacement(saved: SavedSignature): void {
  setActiveTool('select');
  setSignState({ pending: saved, menuOpen: false });
  window.addEventListener('keydown', onPlacementKey, true);
}

export function cancelPlacement(): void {
  window.removeEventListener('keydown', onPlacementKey, true);
  setSignState('pending', null);
}

/** Put a saved signature on a page as a new, selected annotation. */
export function placeSignature(
  store: DocumentStore,
  pageId: PageId,
  rect: PdfRect,
  saved: SavedSignature
): string {
  const id = newAnnotId();
  store.apply({
    type: 'addAnnot',
    annot: {
      id,
      pageId,
      kind: 'signature',
      rect,
      color: saved.color,
      opacity: 1,
      // The store copies what it is given; the saved list keeps its own.
      signature: saved.art,
    },
  });
  store.setSelected({ pageId, annotId: id });
  return id;
}

/** Whether a form field is one a visual signature can go into: an empty,
 * editable signature field. A signed one is left alone - a picture over a
 * certificate signature would only hide what it says. */
export function isOpenSignatureField(f: FormFieldInfo): boolean {
  return f.kind === 'signature' && !f.readOnly && !f.signed && f.rect.w > 0 && f.rect.h > 0;
}

/** Place the armed signature for a click at `at` on `page`: fitted into the
 * empty signature field under the click if there is one, else centred on
 * the click at its starting size. */
export async function placePendingSignature(
  store: DocumentStore,
  page: PageEntry,
  at: PdfPoint
): Promise<void> {
  const saved = signState.pending;
  if (!saved) return;
  cancelPlacement();
  const fields = page.srcIndex === null ? [] : await formFields(store.state.docId);
  const field = fields.find(
    (f) =>
      f.page === page.srcIndex &&
      isOpenSignatureField(f) &&
      at.x >= f.rect.x &&
      at.x <= f.rect.x + f.rect.w &&
      at.y >= f.rect.y &&
      at.y <= f.rect.y + f.rect.h
  );
  const rect = field
    ? fitIntoField(saved.art, field.rect)
    : placementRect(saved.art, saved.role, at, page);
  placeSignature(store, page.id, rect, saved);
}

// ---------- the create dialog and the menu ----------

export function openSignDialog(role: SignatureRole, target: FieldTarget | null = null): void {
  dialogTarget = target;
  void loadSignatures();
  setSignState({ dialog: { role }, menuOpen: false });
}

export function closeSignDialog(): void {
  dialogTarget = null;
  setSignState('dialog', null);
}

/** Store what the dialog made, then put it where it was asked for: into the
 * field that was clicked, or into the user's hand to place with a click. */
export async function finishSignDialog(
  made: Omit<SavedSignature, 'id' | 'createdAt'>
): Promise<void> {
  const target = dialogTarget;
  const saved = await addSignature(made);
  closeSignDialog();
  if (target) {
    placeSignature(target.store, target.pageId, fitIntoField(saved.art, target.rect), saved);
  } else {
    armPlacement(saved);
  }
}

export function setSignMenuOpen(open: boolean): void {
  setSignState('menuOpen', open);
  if (open) void loadSignatures();
}

/** Flip whether this tab's signatures are written into the page at save.
 * An undoable edit, so it dirties the document like any other. */
export function toggleFlattenSignatures(
  store: DocumentStore | undefined = activeTab()?.documentStore
): void {
  if (!store?.state.loaded) return;
  store.apply({ type: 'setFlattenSignatures', value: !store.state.flattenSignatures });
}

/** Tests only. */
export function resetSignStore(): void {
  window.removeEventListener('keydown', onPlacementKey, true);
  dialogTarget = null;
  loading = null;
  setSignState({ loaded: false, items: [], pending: null, dialog: null, menuOpen: false });
}
