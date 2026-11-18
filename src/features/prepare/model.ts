/** Prepare form mode's model: the form as it is being designed, and the
 * plan the engine writes it from (src-tauri/src/engine/forms/design.rs).
 *
 * A design holds every field of the document in one list - the ones already
 * in the file (each with an `origin`: where it was and what it said) and the
 * ones made here. It is one value in the document store, replaced whole by
 * each edit, so every change is one undo step and the plan is a diff of the
 * design against the origins. Nothing here talks to the engine. */
import type {
  FieldCalc,
  FieldFormat,
  FormFieldInfo,
  FormInfo,
  FormOption,
} from '../../types/engine';
import type { PageEntry, PageId, PdfRect } from '../../types/model';

export type DesignKind = 'text' | 'checkbox' | 'radio' | 'combo' | 'list' | 'signature';

/** What Prepare form's tool row draws. */
export type PrepareTool =
  'select' | 'text' | 'multiline' | 'checkbox' | 'radio' | 'combo' | 'list' | 'signature' | 'date';

/** A field's editable properties. */
export interface DesignProps {
  kind: DesignKind;
  rect: PdfRect;
  name: string;
  tooltip: string;
  required: boolean;
  readOnly: boolean;
  /** Text, a choice's export value, or for a checkbox or radio the export
   * value it starts checked with ("" for off). */
  defaultValue: string;
  /** 0 is auto. */
  fontSize: number;
  align: 0 | 1 | 2;
  maxLen: number;
  comb: boolean;
  multiline: boolean;
  options: FormOption[];
  /** A checkbox's or radio button's on-state. */
  exportValue: string;
  border: boolean;
  format: FieldFormat | null;
  calc: FieldCalc | null;
}

/** A widget already in the file, as read: its source page, field name and
 * rect there. */
export interface WidgetRef {
  src: number;
  name: string;
  rect: PdfRect;
}

export interface DesignField extends DesignProps {
  id: string;
  pageId: PageId;
  /** Set for a widget already in the file: where it was, and what it said. */
  origin?: WidgetRef & { base: DesignProps; pageId: PageId };
  /** Its format or calculation is a script SpeedyF did not recognise, which
   * is kept unless the user sets one. */
  opaqueScripts?: boolean;
}

export type TabMode = 'R' | 'C' | 'S';

export interface FormDesign {
  fields: DesignField[];
  /** Widgets from the file that were deleted. */
  deleted: WidgetRef[];
  /** Pages whose Tab order was set here: the mode and field ids in order. */
  tabs: Record<PageId, { mode: TabMode; order: string[] }>;
  /** Merge the fields into the pages at save and remove the form. */
  flatten: boolean;
}

export const EMPTY_DESIGN: FormDesign = { fields: [], deleted: [], tabs: {}, flatten: false };

const KINDS = new Set<DesignKind>(['text', 'checkbox', 'radio', 'combo', 'list', 'signature']);

let seq = 0;
/** A fresh field id. */
export function newFieldId(): string {
  seq += 1;
  return `pf-${Date.now().toString(36)}-${seq}`;
}

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

export function propsOf(f: DesignProps): DesignProps {
  return {
    kind: f.kind,
    rect: { ...f.rect },
    name: f.name,
    tooltip: f.tooltip,
    required: f.required,
    readOnly: f.readOnly,
    defaultValue: f.defaultValue,
    fontSize: f.fontSize,
    align: f.align,
    maxLen: f.maxLen,
    comb: f.comb,
    multiline: f.multiline,
    options: f.options.map((o) => ({ ...o })),
    exportValue: f.exportValue,
    border: f.border,
    format: f.format ? clone(f.format) : null,
    calc: f.calc ? clone(f.calc) : null,
  };
}

/** The design of the form as the file has it: every widget a field. */
export function designFromForm(
  info: FormInfo | undefined,
  pages: readonly PageEntry[]
): FormDesign {
  const fields: DesignField[] = [];
  const pageOf = new Map<number, PageId>();
  for (const p of pages) {
    if (p.srcIndex !== null && !pageOf.has(p.srcIndex)) pageOf.set(p.srcIndex, p.id);
  }
  const widgets = [...(info?.fields ?? [])].sort((a, b) => a.tabOrder - b.tabOrder);
  for (const w of widgets) {
    if (!KINDS.has(w.kind as DesignKind) || w.rect.w <= 0 || w.rect.h <= 0) continue;
    const pageId = pageOf.get(w.page);
    if (!pageId) continue;
    const behaviour = info?.behaviours?.[w.name];
    const format = behaviour?.format ?? behaviour?.keystroke ?? null;
    const props: DesignProps = {
      kind: w.kind as DesignKind,
      rect: { ...w.rect },
      name: w.name,
      tooltip: w.tooltip,
      required: w.required,
      readOnly: w.readOnly,
      defaultValue: defaultOf(w),
      fontSize: w.fontSize,
      align: (Math.min(2, Math.max(0, w.align)) as 0 | 1 | 2) ?? 0,
      maxLen: w.maxLen,
      comb: w.comb,
      multiline: w.multiline,
      options: w.options.map((o) => ({ ...o })),
      exportValue: w.exportValue,
      border: false,
      format: format ? clone(format) : null,
      calc: behaviour?.calculate ? clone(behaviour.calculate) : null,
    };
    fields.push({
      ...propsOf(props),
      id: `src-${w.page}-${w.index}`,
      pageId,
      origin: { src: w.page, name: w.name, rect: { ...w.rect }, base: props, pageId },
      opaqueScripts: w.scriptCount > 0 && !behaviour,
    });
  }
  return { fields, deleted: [], tabs: {}, flatten: false };
}

function defaultOf(w: FormFieldInfo): string {
  switch (w.kind) {
    case 'text':
      return w.defaultValue;
    case 'combo':
    case 'list': {
      const i = w.defaultSelected[0];
      return i === undefined ? w.defaultValue : (w.options[i]?.export ?? '');
    }
    case 'checkbox':
    case 'radio':
      return w.defaultChecked ? w.exportValue : '';
    default:
      return '';
  }
}

/** `base`, or `base 2`, `base 3`... - whichever `taken` does not hold. */
export function uniqueName(base: string, taken: ReadonlySet<string>): string {
  const clean = base.replace(/\./g, ' ').trim() || 'Field';
  if (!taken.has(clean)) return clean;
  for (let n = 2; ; n += 1) {
    const candidate = `${clean} ${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

export function namesOf(design: FormDesign): Set<string> {
  return new Set(design.fields.map((f) => f.name));
}

const BASE_NAME: Record<PrepareTool, string> = {
  select: 'Field',
  text: 'Text',
  multiline: 'Text',
  checkbox: 'Check Box',
  radio: 'Choice',
  combo: 'Dropdown',
  list: 'List Box',
  signature: 'Signature',
  date: 'Date',
};

/** The size a click (rather than a drag) makes, in points. */
export function defaultSize(tool: PrepareTool): { w: number; h: number } {
  switch (tool) {
    case 'checkbox':
    case 'radio':
      return { w: 12, h: 12 };
    case 'multiline':
    case 'list':
      return { w: 180, h: 54 };
    case 'signature':
      return { w: 180, h: 28 };
    case 'date':
      return { w: 90, h: 18 };
    default:
      return { w: 150, h: 18 };
  }
}

/** A new field of `tool`'s kind at `rect`. */
export function newField(
  tool: Exclude<PrepareTool, 'select'>,
  pageId: PageId,
  rect: PdfRect,
  design: FormDesign,
  datePicture = 'mm/dd/yyyy'
): DesignField {
  const kind: DesignKind = tool === 'multiline' || tool === 'date' ? 'text' : (tool as DesignKind);
  const taken = namesOf(design);
  // A radio button joins the group just made, as Acrobat's tool does,
  // unless the user names it otherwise.
  const name =
    tool === 'radio'
      ? `${BASE_NAME.radio} ${radioGroupNumber(design)}`
      : uniqueName(`${BASE_NAME[tool]} ${countOf(design, tool) + 1}`, taken);
  const radioIndex = design.fields.filter((f) => f.kind === 'radio' && f.name === name).length;
  return {
    id: newFieldId(),
    pageId,
    kind,
    rect: { ...rect },
    name,
    tooltip: '',
    required: false,
    readOnly: false,
    defaultValue: '',
    fontSize: 0,
    align: 0,
    maxLen: 0,
    comb: false,
    multiline: tool === 'multiline',
    options:
      kind === 'combo' || kind === 'list'
        ? [
            { label: 'Option 1', export: 'Option 1' },
            { label: 'Option 2', export: 'Option 2' },
          ]
        : [],
    exportValue: kind === 'checkbox' ? 'Yes' : kind === 'radio' ? `Choice${radioIndex + 1}` : '',
    border: kind === 'checkbox' || kind === 'radio' || kind === 'signature',
    format: tool === 'date' ? { kind: 'date', format: datePicture } : null,
    calc: null,
  };
}

function countOf(design: FormDesign, tool: PrepareTool): number {
  return design.fields.filter((f) => f.name.startsWith(BASE_NAME[tool])).length;
}

/** The radio group a new button joins: the newest made here, else a new one. */
function radioGroupNumber(design: FormDesign): number {
  const made = design.fields.filter((f) => f.kind === 'radio' && !f.origin);
  const last = made[made.length - 1];
  const m = last?.name.match(/^Choice (\d+)$/);
  if (m) return Number(m[1]);
  let n = 1;
  const taken = namesOf(design);
  while (taken.has(`Choice ${n}`)) n += 1;
  return n;
}

// ---------------------------------------------------------------------------
// Edits
// ---------------------------------------------------------------------------

export function addFields(design: FormDesign, fields: DesignField[]): FormDesign {
  return { ...clone(design), fields: [...clone(design.fields), ...clone(fields)] };
}

export function patchFields(
  design: FormDesign,
  ids: readonly string[],
  patch: (f: DesignField) => Partial<DesignProps>
): FormDesign {
  const next = clone(design);
  const wanted = new Set(ids);
  next.fields = next.fields.map((f) => (wanted.has(f.id) ? { ...f, ...clone(patch(f)) } : f));
  return next;
}

export function moveFields(design: FormDesign, ids: readonly string[], dx: number, dy: number) {
  return patchFields(design, ids, (f) => ({
    rect: { ...f.rect, x: f.rect.x + dx, y: f.rect.y + dy },
  }));
}

export function deleteFields(design: FormDesign, ids: readonly string[]): FormDesign {
  const next = clone(design);
  const gone = new Set(ids);
  for (const f of next.fields) {
    if (gone.has(f.id) && f.origin) {
      next.deleted.push({ src: f.origin.src, name: f.origin.name, rect: { ...f.origin.rect } });
    }
  }
  next.fields = next.fields.filter((f) => !gone.has(f.id));
  for (const tab of Object.values(next.tabs)) tab.order = tab.order.filter((id) => !gone.has(id));
  return next;
}

/** Copies of `ids`, a little down and right, named anew (radio buttons stay
 * in their group with a new export value). Returns the design and the new
 * ids. */
export function duplicateFields(
  design: FormDesign,
  ids: readonly string[],
  offset = 10
): { design: FormDesign; ids: string[] } {
  const taken = namesOf(design);
  const copies: DesignField[] = [];
  for (const f of design.fields.filter((x) => ids.includes(x.id))) {
    const copy: DesignField = {
      ...propsOf(f),
      id: newFieldId(),
      pageId: f.pageId,
      rect: { ...f.rect, x: f.rect.x + offset, y: f.rect.y - offset },
      name: f.kind === 'radio' ? f.name : uniqueName(f.name, taken),
      exportValue:
        f.kind === 'radio'
          ? uniqueName(f.exportValue || 'Choice', exportsOf(design, f.name))
          : f.exportValue,
    };
    taken.add(copy.name);
    copies.push(copy);
  }
  return { design: addFields(design, copies), ids: copies.map((c) => c.id) };
}

function exportsOf(design: FormDesign, group: string): Set<string> {
  return new Set(design.fields.filter((f) => f.name === group).map((f) => f.exportValue));
}

/** What `name` may be for field `id`: an error, or null when it is fine.
 * Fields other than radio buttons share a value when they share a name, so
 * a new name must be free. */
export function nameProblem(design: FormDesign, id: string, name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return 'A field needs a name';
  if (trimmed.includes('.')) return 'A name cannot contain a full stop';
  const self = design.fields.find((f) => f.id === id);
  const clash = design.fields.find((f) => f.id !== id && f.name === trimmed);
  if (!clash) return null;
  if (self?.kind === 'radio' && clash.kind === 'radio') return null;
  return 'Another field has this name';
}

// ---------------------------------------------------------------------------
// Tab order
// ---------------------------------------------------------------------------

/** Field ids of a page in Tab order: as set here, else as read (the order
 * fields came out of the file), new fields last. */
export function tabOrderOf(design: FormDesign, pageId: PageId): string[] {
  const onPage = design.fields.filter((f) => f.pageId === pageId);
  const set = design.tabs[pageId];
  if (!set) return onPage.map((f) => f.id);
  const ids = new Set(onPage.map((f) => f.id));
  const listed = set.order.filter((id) => ids.has(id));
  const rest = onPage.map((f) => f.id).filter((id) => !listed.includes(id));
  return [...listed, ...rest];
}

/** Rows top to bottom, each left to right (or columns), grouping fields
 * whose edges are within half the smaller height (width) - the rule the
 * engine reads /Tabs /R and /C by. */
export function geometricOrder(fields: readonly DesignField[], mode: 'R' | 'C'): string[] {
  const top = (f: DesignField) => f.rect.y + f.rect.h;
  const sorted = [...fields].sort((a, b) => (mode === 'R' ? top(b) - top(a) : a.rect.x - b.rect.x));
  const out: string[] = [];
  let band: DesignField[] = [];
  const flush = () => {
    band.sort((a, b) => (mode === 'R' ? a.rect.x - b.rect.x : top(b) - top(a)));
    out.push(...band.map((f) => f.id));
    band = [];
  };
  for (const f of sorted) {
    const first = band[0];
    if (first) {
      const gap = mode === 'R' ? top(first) - top(f) : f.rect.x - first.rect.x;
      const extent =
        mode === 'R' ? Math.min(first.rect.h, f.rect.h) : Math.min(first.rect.w, f.rect.w);
      if (gap > 0.5 * extent) flush();
    }
    band.push(f);
  }
  flush();
  return out;
}

export function setTabOrder(
  design: FormDesign,
  pageId: PageId,
  mode: TabMode,
  order?: string[]
): FormDesign {
  const next = clone(design);
  const onPage = next.fields.filter((f) => f.pageId === pageId);
  const ids = order ?? (mode === 'S' ? tabOrderOf(design, pageId) : geometricOrder(onPage, mode));
  next.tabs[pageId] = { mode, order: ids };
  return next;
}

/** Move one field to `to` in its page's Tab order. */
export function moveInTabOrder(design: FormDesign, pageId: PageId, id: string, to: number) {
  const order = tabOrderOf(design, pageId).filter((x) => x !== id);
  order.splice(Math.max(0, Math.min(order.length, to)), 0, id);
  return setTabOrder(design, pageId, 'S', order);
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

export interface PlanWidgetRef {
  page: number;
  name: string;
  rect: PdfRect;
}

export interface PlanDesignField {
  key: string;
  origin?: PlanWidgetRef;
  page: number;
  rect: PdfRect;
  kind: DesignKind;
  name: string;
  tooltip: string;
  required: boolean;
  readOnly: boolean;
  defaultValue: string;
  fontSize: number;
  align: number;
  maxLen: number;
  comb: boolean;
  multiline: boolean;
  options: FormOption[];
  exportValue: string;
  border: boolean;
  format?: FieldFormat | null;
  calc?: FieldCalc | null;
  formatSet: boolean;
  calcSet: boolean;
  restyle: boolean;
}

export interface PlanFormDesign {
  fields: PlanDesignField[];
  deleted: PlanWidgetRef[];
  tabOrders: {
    page: number;
    mode: TabMode;
    order: { key?: string; widget?: PlanWidgetRef }[];
  }[];
  flatten: boolean;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const near = (a: number, b: number) => Math.abs(a - b) < 0.01;
const sameRect = (a: PdfRect, b: PdfRect) =>
  near(a.x, b.x) && near(a.y, b.y) && near(a.w, b.w) && near(a.h, b.h);

/** How an existing field differs from what the file has. */
export function changeOf(f: DesignField): {
  changed: boolean;
  restyle: boolean;
  formatSet: boolean;
  calcSet: boolean;
} {
  if (!f.origin) return { changed: true, restyle: true, formatSet: true, calcSet: true };
  const b = f.origin.base;
  const formatSet = !same(f.format, b.format);
  const calcSet = !same(f.calc, b.calc);
  const restyle =
    !near(f.rect.w, b.rect.w) ||
    !near(f.rect.h, b.rect.h) ||
    f.fontSize !== b.fontSize ||
    f.align !== b.align ||
    f.comb !== b.comb ||
    f.multiline !== b.multiline ||
    (f.comb && f.maxLen !== b.maxLen) ||
    !same(f.options, b.options) ||
    f.border !== b.border ||
    f.exportValue !== b.exportValue;
  const changed =
    restyle ||
    formatSet ||
    calcSet ||
    f.pageId !== f.origin.pageId ||
    !sameRect(f.rect, b.rect) ||
    f.name !== b.name ||
    f.tooltip !== b.tooltip ||
    f.required !== b.required ||
    f.readOnly !== b.readOnly ||
    f.defaultValue !== b.defaultValue ||
    f.maxLen !== b.maxLen;
  return { changed, restyle, formatSet, calcSet };
}

/** Whether the design changes the file at all. */
export function designIsEdited(design: FormDesign | null | undefined): boolean {
  if (!design) return false;
  return (
    design.flatten ||
    design.deleted.length > 0 ||
    Object.keys(design.tabs).length > 0 ||
    design.fields.some((f) => changeOf(f).changed)
  );
}

/** The engine's plan for `design`, against the document's pages as they
 * now stand; null when it changes nothing. */
export function designPlan(
  design: FormDesign | null | undefined,
  pages: readonly PageEntry[]
): PlanFormDesign | null {
  if (!designIsEdited(design)) return null;
  const d = design!;
  const indexOf = new Map(pages.map((p, i) => [p.id, i]));
  const firstOfSrc = new Map<number, number>();
  pages.forEach((p, i) => {
    if (p.srcIndex !== null && !firstOfSrc.has(p.srcIndex)) firstOfSrc.set(p.srcIndex, i);
  });
  const ref = (r: WidgetRef): PlanWidgetRef | null => {
    const page = firstOfSrc.get(r.src);
    return page === undefined ? null : { page, name: r.name, rect: { ...r.rect } };
  };
  const fields: PlanDesignField[] = [];
  for (const f of d.fields) {
    const page = indexOf.get(f.pageId);
    if (page === undefined) continue;
    const change = changeOf(f);
    if (!change.changed) continue;
    const origin = f.origin ? ref(f.origin) : null;
    if (f.origin && !origin) continue;
    fields.push({
      key: f.id,
      ...(origin ? { origin } : {}),
      page,
      rect: { ...f.rect },
      kind: f.kind,
      name: f.name.trim(),
      tooltip: f.tooltip,
      required: f.required,
      readOnly: f.readOnly,
      defaultValue: f.defaultValue,
      fontSize: f.fontSize,
      align: f.align,
      maxLen: f.maxLen,
      comb: f.comb,
      multiline: f.multiline,
      options: f.options.map((o) => ({ ...o })),
      exportValue: f.exportValue,
      border: f.border,
      format: f.format,
      calc: f.calc,
      formatSet: change.formatSet,
      calcSet: change.calcSet,
      restyle: change.restyle,
    });
  }
  const deleted = d.deleted.map(ref).filter((r): r is PlanWidgetRef => r !== null);
  const byId = new Map(d.fields.map((f) => [f.id, f]));
  const tabOrders: PlanFormDesign['tabOrders'] = [];
  for (const [pageId, tab] of Object.entries(d.tabs)) {
    const page = indexOf.get(pageId);
    if (page === undefined) continue;
    const order = tabOrderOf(d, pageId).flatMap(
      (id): { key?: string; widget?: PlanWidgetRef }[] => {
        const f = byId.get(id);
        if (!f) return [];
        if (f.origin && !changeOf(f).changed) {
          const widget = ref(f.origin);
          return widget ? [{ widget }] : [];
        }
        return [{ key: id }];
      }
    );
    tabOrders.push({ page, mode: tab.mode, order });
  }
  return { fields, deleted, tabOrders, flatten: d.flatten };
}

// ---------------------------------------------------------------------------
// Snapping
// ---------------------------------------------------------------------------

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

function overlap(a: Box, b: Box): number {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

/** The blank a drawn (or clicked) rect means: one it mostly covers or that
 * mostly covers it. */
export function blankUnder<T extends { rect: Box }>(rect: Box, blanks: readonly T[]): T | null {
  let best: T | null = null;
  let bestScore = 0;
  for (const b of blanks) {
    const i = overlap(rect, b.rect);
    if (i <= 0) continue;
    const score = i / Math.min(rect.w * rect.h || 1, b.rect.w * b.rect.h || 1);
    if (score > 0.6 && score > bestScore) {
      best = b;
      bestScore = score;
    }
  }
  return best;
}

/** The blank a click at (x, y) lands in, smallest first. */
export function blankAtPoint<T extends { rect: Box }>(x: number, y: number, blanks: readonly T[]) {
  return (
    [...blanks]
      .filter(
        (b) =>
          x >= b.rect.x && x <= b.rect.x + b.rect.w && y >= b.rect.y && y <= b.rect.y + b.rect.h
      )
      .sort((a, b) => a.rect.w * a.rect.h - b.rect.w * b.rect.h)[0] ?? null
  );
}

/** `rect` moved so an edge within `tol` points of a blank's edge sits on
 * it. */
export function snapToEdges(rect: Box, blanks: readonly { rect: Box }[], tol = 3): Box {
  let dx = 0;
  let dy = 0;
  let bestX = tol;
  let bestY = tol;
  for (const { rect: b } of blanks) {
    for (const [mine, theirs] of [
      [rect.x, b.x],
      [rect.x + rect.w, b.x + b.w],
      [rect.x, b.x + b.w],
      [rect.x + rect.w, b.x],
    ] as const) {
      const d = theirs - mine;
      if (Math.abs(d) < bestX) {
        bestX = Math.abs(d);
        dx = d;
      }
    }
    for (const [mine, theirs] of [
      [rect.y, b.y],
      [rect.y + rect.h, b.y + b.h],
    ] as const) {
      const d = theirs - mine;
      if (Math.abs(d) < bestY) {
        bestY = Math.abs(d);
        dy = d;
      }
    }
  }
  return { ...rect, x: rect.x + dx, y: rect.y + dy };
}

/** A field from an auto-detect proposal. */
export function fieldFromProposal(
  p: {
    rect: Box;
    kind: 'text' | 'checkbox';
    name: string;
    label: string;
    fontSize: number;
    maxLen: number;
    comb: boolean;
    multiline: boolean;
  },
  pageId: PageId,
  taken: Set<string>
): DesignField {
  const name = uniqueName(p.name, taken);
  taken.add(name);
  const label = p.label.replace(/\s+/g, ' ').trim();
  return {
    id: newFieldId(),
    pageId,
    kind: p.kind,
    rect: { ...p.rect },
    name,
    tooltip: label.length > name.length ? label : '',
    required: false,
    readOnly: false,
    defaultValue: '',
    fontSize: p.fontSize,
    align: 0,
    maxLen: p.maxLen,
    comb: p.comb,
    multiline: p.multiline,
    options: [],
    exportValue: p.kind === 'checkbox' ? 'Yes' : '',
    border: false,
    format: null,
    calc: null,
  };
}

/** A field's kind as the panel names it. */
export function kindLabel(f: Pick<DesignProps, 'kind' | 'multiline' | 'format'>): string {
  switch (f.kind) {
    case 'text':
      if (f.format?.kind === 'date') return 'Date';
      return f.multiline ? 'Multiline text' : 'Text';
    case 'checkbox':
      return 'Checkbox';
    case 'radio':
      return 'Radio button';
    case 'combo':
      return 'Dropdown';
    case 'list':
      return 'List box';
    case 'signature':
      return 'Signature';
  }
}
