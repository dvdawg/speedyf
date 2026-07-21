/**
 * Document model store — the single source of truth for document mutations.
 * Holds ONLY lightweight metadata (ids, order, rotations, annotation vectors,
 * form edits). Never PDF bytes or bitmaps. UI components subscribe to this
 * store; they are not allowed to own document state.
 *
 * Every mutation flows through `apply(op)`, which records an inverse op for
 * undo/redo and maintains dirty/savepoint bookkeeping.
 */
import { createStore, produce } from 'solid-js/store';
import type { Annotation, DocMeta, PageEntry, PageId, PdfRect, Rotation } from '../../types/model';

export type EditOp =
  | { type: 'reorder'; pageId: PageId; toIndex: number }
  | { type: 'delete'; pageId: PageId }
  | { type: 'duplicate'; pageId: PageId; newId?: PageId; annotIds?: string[] }
  | { type: 'rotate'; pageId: PageId; delta: 90 | 180 | 270 }
  | { type: 'addBlank'; index: number; widthPt: number; heightPt: number; newId?: PageId }
  | { type: 'addAnnot'; annot: Annotation }
  | { type: 'patchAnnot'; pageId: PageId; id: string; patch: Partial<Annotation> }
  | { type: 'deleteAnnot'; pageId: PageId; id: string }
  | { type: 'setForm'; field: string; value: string }
  // internal inverse-only ops:
  | { type: 'restorePage'; entry: PageEntry; index: number; annots: Annotation[] }
  | { type: 'restoreAnnot'; pageId: PageId; annot: Annotation; index: number }
  | { type: 'restoreForm'; field: string; prev: string | undefined };

export interface PlanAnnot {
  kind: 'highlight' | 'ink' | 'rect' | 'note';
  rect: PdfRect;
  color: string;
  opacity: number;
  strokeWidth?: number;
  quads?: Annotation['quads'];
  strokes?: Annotation['strokes'];
  text?: string;
}

export interface PlanText {
  rect: PdfRect;
  text: string;
  fontSizePt: number;
  color: string;
  opacity: number;
}

export interface PlanImage {
  rect: PdfRect;
  sourcePath: string;
}

export interface PlanPage {
  srcIndex: number | null;
  widthPt: number;
  heightPt: number;
  cropX: number;
  cropY: number;
  /** absolute target rotation = (base + user) % 360 */
  rotation: Rotation;
  annots: PlanAnnot[];
  texts: PlanText[];
  images: PlanImage[];
}

export interface EditPlan {
  pages: PlanPage[];
  form: [string, string][];
}

export interface DocState {
  loaded: boolean;
  docId: number;
  path: string | null;
  name: string;
  pages: PageEntry[];
  annotations: Record<PageId, Annotation[]>;
  formEdits: Record<string, string>;
  dirty: boolean;
  saving: boolean;
  /** bumped whenever engine-derived visuals must be re-fetched (reload) */
  generation: number;
  selected: { pageId: PageId; annotId: string } | null;
  searchIndex: { indexed: number; total: number; done: boolean };
  historyDepth: number;
  redoDepth: number;
}

const emptyState = (): DocState => ({
  loaded: false,
  docId: -1,
  path: null,
  name: '',
  pages: [],
  annotations: {},
  formEdits: {},
  dirty: false,
  saving: false,
  generation: 0,
  selected: null,
  searchIndex: { indexed: 0, total: 0, done: false },
  historyDepth: 0,
  redoDepth: 0,
});

interface HistoryEntry {
  op: EditOp;
  inverse: EditOp;
}

export interface DocumentStore {
  state: DocState;
  initFromMeta: (meta: DocMeta) => void;
  updateSizes: (from: number, sizes: [number, number, number, number, number][]) => void;
  apply: (op: EditOp) => void;
  undo: () => boolean;
  redo: () => boolean;
  canUndo: () => boolean;
  canRedo: () => boolean;
  markSaved: () => void;
  setSaving: (v: boolean) => void;
  setSelected: (sel: DocState['selected']) => void;
  setSearchIndex: (s: Partial<DocState['searchIndex']>) => void;
  reset: () => void;
  pageIndexById: (id: PageId) => number;
  buildEditPlan: () => EditPlan;
}

export function createDocumentStore(opts?: { historyLimit?: number }): DocumentStore {
  const historyLimit = opts?.historyLimit ?? 200;
  const [state, setState] = createStore<DocState>(emptyState());

  let undoStack: HistoryEntry[] = [];
  let redoStack: HistoryEntry[] = [];
  /** undoStack.length at the last save; null once history diverged past it. */
  let savedLen: number | null = 0;
  let idSeq = 0;

  const genId = (prefix: string) => `${prefix}-${++idSeq}`;

  const syncHistoryMeta = () => {
    setState(
      produce((s) => {
        s.dirty = savedLen === null ? true : undoStack.length !== savedLen;
        s.historyDepth = undoStack.length;
        s.redoDepth = redoStack.length;
      })
    );
  };

  const pageIndexById = (id: PageId) => state.pages.findIndex((p) => p.id === id);

  /** Fill in generated ids so redo replays deterministically, then build the inverse. */
  const materialize = (op: EditOp): EditOp => {
    if (op.type === 'duplicate') {
      const annots = state.annotations[op.pageId] ?? [];
      return {
        ...op,
        newId: op.newId ?? genId('pg'),
        annotIds: op.annotIds ?? annots.map(() => genId('an')),
      };
    }
    if (op.type === 'addBlank') {
      return { ...op, newId: op.newId ?? genId('pg') };
    }
    return op;
  };

  const computeInverse = (op: EditOp): EditOp => {
    switch (op.type) {
      case 'reorder':
        return { type: 'reorder', pageId: op.pageId, toIndex: pageIndexById(op.pageId) };
      case 'delete': {
        const index = pageIndexById(op.pageId);
        const entry = JSON.parse(JSON.stringify(state.pages[index])) as PageEntry;
        const annots = JSON.parse(
          JSON.stringify(state.annotations[op.pageId] ?? [])
        ) as Annotation[];
        return { type: 'restorePage', entry, index, annots };
      }
      case 'duplicate':
        return { type: 'delete', pageId: op.newId! };
      case 'rotate':
        return { type: 'rotate', pageId: op.pageId, delta: ((360 - op.delta) % 360) as 90 | 180 | 270 };
      case 'addBlank':
        return { type: 'delete', pageId: op.newId! };
      case 'addAnnot':
        return { type: 'deleteAnnot', pageId: op.annot.pageId, id: op.annot.id };
      case 'patchAnnot': {
        const prev = (state.annotations[op.pageId] ?? []).find((a) => a.id === op.id);
        const patch: Partial<Annotation> = {};
        if (prev) {
          for (const key of Object.keys(op.patch) as (keyof Annotation)[]) {
            (patch as Record<string, unknown>)[key] = JSON.parse(
              JSON.stringify(prev[key] ?? null)
            ) as unknown;
          }
        }
        return { type: 'patchAnnot', pageId: op.pageId, id: op.id, patch };
      }
      case 'deleteAnnot': {
        const list = state.annotations[op.pageId] ?? [];
        const index = list.findIndex((a) => a.id === op.id);
        const annot = JSON.parse(JSON.stringify(list[index])) as Annotation;
        return { type: 'restoreAnnot', pageId: op.pageId, annot, index };
      }
      case 'setForm':
        return { type: 'restoreForm', field: op.field, prev: state.formEdits[op.field] };
      case 'restorePage':
      case 'restoreAnnot':
      case 'restoreForm':
        throw new Error(`inverse-only op cannot be applied directly: ${op.type}`);
    }
  };

  const execute = (op: EditOp) => {
    setState(
      produce((s) => {
        switch (op.type) {
          case 'reorder': {
            const from = s.pages.findIndex((p) => p.id === op.pageId);
            if (from < 0) return;
            const [entry] = s.pages.splice(from, 1);
            s.pages.splice(op.toIndex, 0, entry!);
            break;
          }
          case 'delete': {
            const idx = s.pages.findIndex((p) => p.id === op.pageId);
            if (idx < 0) return;
            s.pages.splice(idx, 1);
            delete s.annotations[op.pageId];
            if (s.selected?.pageId === op.pageId) s.selected = null;
            break;
          }
          case 'duplicate': {
            const idx = s.pages.findIndex((p) => p.id === op.pageId);
            if (idx < 0) return;
            const src = s.pages[idx]!;
            const copy: PageEntry = { ...src, id: op.newId! };
            s.pages.splice(idx + 1, 0, copy);
            const annots = s.annotations[op.pageId] ?? [];
            s.annotations[copy.id] = annots.map((a, i) => ({
              ...(JSON.parse(JSON.stringify(a)) as Annotation),
              id: op.annotIds![i]!,
              pageId: copy.id,
            }));
            break;
          }
          case 'rotate': {
            const p = s.pages.find((pg) => pg.id === op.pageId);
            if (!p) return;
            p.userRotation = ((p.userRotation + op.delta) % 360) as Rotation;
            break;
          }
          case 'addBlank': {
            const entry: PageEntry = {
              id: op.newId!,
              srcIndex: null,
              baseRotation: 0,
              userRotation: 0,
              widthPt: op.widthPt,
              heightPt: op.heightPt,
              cropX: 0,
              cropY: 0,
              sizeKnown: true,
            };
            s.pages.splice(op.index, 0, entry);
            break;
          }
          case 'addAnnot': {
            const list = (s.annotations[op.annot.pageId] ??= []);
            list.push(JSON.parse(JSON.stringify(op.annot)) as Annotation);
            break;
          }
          case 'patchAnnot': {
            const list = s.annotations[op.pageId] ?? [];
            const a = list.find((x) => x.id === op.id);
            if (!a) return;
            Object.assign(a, JSON.parse(JSON.stringify(op.patch)) as Partial<Annotation>);
            break;
          }
          case 'deleteAnnot': {
            const list = s.annotations[op.pageId];
            if (!list) return;
            const idx = list.findIndex((x) => x.id === op.id);
            if (idx >= 0) list.splice(idx, 1);
            if (s.selected?.annotId === op.id) s.selected = null;
            break;
          }
          case 'setForm': {
            s.formEdits[op.field] = op.value;
            break;
          }
          case 'restorePage': {
            s.pages.splice(op.index, 0, JSON.parse(JSON.stringify(op.entry)) as PageEntry);
            s.annotations[op.entry.id] = JSON.parse(JSON.stringify(op.annots)) as Annotation[];
            break;
          }
          case 'restoreAnnot': {
            const list = (s.annotations[op.pageId] ??= []);
            list.splice(op.index, 0, JSON.parse(JSON.stringify(op.annot)) as Annotation);
            break;
          }
          case 'restoreForm': {
            if (op.prev === undefined) delete s.formEdits[op.field];
            else s.formEdits[op.field] = op.prev;
            break;
          }
        }
      })
    );
  };

  const apply = (op: EditOp) => {
    const mat = materialize(op);
    const inverse = computeInverse(mat);
    execute(mat);
    if (redoStack.length > 0) {
      redoStack = [];
      if (savedLen !== null && savedLen > undoStack.length) savedLen = null;
    }
    undoStack.push({ op: mat, inverse });
    if (undoStack.length > historyLimit) {
      undoStack.shift();
      if (savedLen !== null) {
        savedLen -= 1;
        if (savedLen < 0) savedLen = null;
      }
    }
    syncHistoryMeta();
  };

  const undo = () => {
    const entry = undoStack.pop();
    if (!entry) return false;
    execute(entry.inverse);
    redoStack.push(entry);
    syncHistoryMeta();
    return true;
  };

  const redo = () => {
    const entry = redoStack.pop();
    if (!entry) return false;
    execute(entry.op);
    undoStack.push(entry);
    syncHistoryMeta();
    return true;
  };

  const initFromMeta = (meta: DocMeta) => {
    undoStack = [];
    redoStack = [];
    savedLen = 0;
    const pages: PageEntry[] = [];
    for (let i = 0; i < meta.pageCount; i++) {
      const size = meta.sizes[i];
      pages.push({
        id: genId('pg'),
        srcIndex: i,
        baseRotation: (size ? size[4] : 0) as Rotation,
        userRotation: 0,
        widthPt: size ? size[0] : meta.estimatedSize[0],
        heightPt: size ? size[1] : meta.estimatedSize[1],
        cropX: size ? size[2] : 0,
        cropY: size ? size[3] : 0,
        sizeKnown: Boolean(size),
      });
    }
    setState({
      ...emptyState(),
      loaded: true,
      docId: meta.docId,
      path: meta.path,
      name: meta.name,
      pages,
      searchIndex: { indexed: 0, total: meta.pageCount, done: false },
    });
  };

  const updateSizes = (from: number, sizes: [number, number, number, number, number][]) => {
    setState(
      produce((s) => {
        sizes.forEach(([w, h, cx, cy, rot], i) => {
          // sizes arrive keyed by ORIGINAL source index; update matching pages
          for (const p of s.pages) {
            if (p.srcIndex === from + i && !p.sizeKnown) {
              p.widthPt = w;
              p.heightPt = h;
              p.cropX = cx;
              p.cropY = cy;
              p.baseRotation = rot as Rotation;
              p.sizeKnown = true;
            }
          }
        });
      })
    );
  };

  const buildEditPlan = (): EditPlan => {
    const pages: PlanPage[] = state.pages.map((p) => {
      const anns = state.annotations[p.id] ?? [];
      return {
        srcIndex: p.srcIndex,
        widthPt: p.widthPt,
        heightPt: p.heightPt,
        cropX: p.cropX,
        cropY: p.cropY,
        rotation: (((p.baseRotation + p.userRotation) % 360) + 0) as Rotation,
        annots: anns
          .filter((a): a is Annotation & { kind: PlanAnnot['kind'] } =>
            ['highlight', 'ink', 'rect', 'note'].includes(a.kind)
          )
          .map((a) => ({
            kind: a.kind,
            rect: a.rect,
            color: a.color,
            opacity: a.opacity,
            ...(a.strokeWidth !== undefined ? { strokeWidth: a.strokeWidth } : {}),
            ...(a.quads !== undefined ? { quads: a.quads } : {}),
            ...(a.strokes !== undefined ? { strokes: a.strokes } : {}),
            ...(a.text !== undefined ? { text: a.text } : {}),
          })),
        texts: anns
          .filter((a) => a.kind === 'textbox')
          .map((a) => ({
            rect: a.rect,
            text: a.text ?? '',
            fontSizePt: a.fontSizePt ?? 12,
            color: a.color,
            opacity: a.opacity,
          })),
        images: anns
          .filter((a) => a.kind === 'image' && a.sourcePath)
          .map((a) => ({ rect: a.rect, sourcePath: a.sourcePath! })),
      };
    });
    return { pages, form: Object.entries(state.formEdits) };
  };

  return {
    state,
    initFromMeta,
    updateSizes,
    apply,
    undo,
    redo,
    canUndo: () => undoStack.length > 0,
    canRedo: () => redoStack.length > 0,
    markSaved: () => {
      savedLen = undoStack.length;
      syncHistoryMeta();
    },
    setSaving: (v) => setState('saving', v),
    setSelected: (sel) => setState('selected', sel),
    setSearchIndex: (s) =>
      setState(
        produce((st) => {
          Object.assign(st.searchIndex, s);
        })
      ),
    reset: () => {
      undoStack = [];
      redoStack = [];
      savedLen = 0;
      setState(emptyState());
    },
    pageIndexById,
    buildEditPlan,
  };
}

/** App-wide singleton store (components import this; tests build their own). */
export const documentStore = createDocumentStore();
