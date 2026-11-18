/** A document's AcroForm, fetched once and shared by every mounted page, and
 * the small amount of logic the page layer, the panel and the status bar all
 * need to agree on: what a field's value is right now, what Tab visits next,
 * and which required fields are still empty.
 *
 * Form fields come back per *document*, not per page — one call describes the
 * whole file. Left to each page to fetch for itself, opening a long form would
 * fire the same request once per visible page and get the same answer every
 * time, so the promise is cached here and the pages filter what they need out
 * of it.
 */
import { createSignal } from 'solid-js';
import type { FormFieldInfo, FormInfo } from '../../types/engine';
import { engine } from '../../lib/transport/engine';
import { formButtonKey, type DocState, type FormChoice } from '../document/documentStore';

const inFlight = new Map<number, Promise<FormInfo>>();

const EMPTY: FormInfo = { fields: [], xfa: false };

/** The whole form: fields plus whether the document carries XFA. */
export function formInfo(docId: number): Promise<FormInfo> {
  const hit = inFlight.get(docId);
  if (hit) return hit;
  // A document with no form is the common case and answers with no fields;
  // a failure is treated the same way, since a viewer that cannot read the
  // fields should still show the page.
  const request = engine
    .getFormFields(docId)
    .then((info) => info ?? EMPTY)
    .catch(() => EMPTY);
  inFlight.set(docId, request);
  return request;
}

export function formFields(docId: number): Promise<FormFieldInfo[]> {
  return formInfo(docId).then((info) => info.fields);
}

/** Drop the cached fields for a document, so the next read sees the file as
 * it is now. Called when a document is closed or reloaded. */
export function clearFormFields(docId?: number) {
  if (docId === undefined) inFlight.clear();
  else inFlight.delete(docId);
}

const FILLABLE = new Set(['text', 'combo', 'list', 'checkbox', 'radio']);

/** Widgets a reader can operate on the page: every kind SpeedyF can write,
 * unless the form marks it read-only, and only where it has an area. */
export function isFillable(field: FormFieldInfo): boolean {
  return FILLABLE.has(field.kind) && !field.readOnly && field.rect.w > 0 && field.rect.h > 0;
}

/** A text field's value as the page should show it. */
export function textValue(doc: DocState, field: FormFieldInfo): string {
  return doc.formEdits[field.name] ?? field.value;
}

/** A combo or list box's selection as the page should show it. */
export function choiceValue(doc: DocState, field: FormFieldInfo): FormChoice {
  return doc.formChoiceEdits[field.name] ?? { selected: field.selected };
}

/** What a combo shows: typed text if there is any, else the selected label. */
export function choiceText(field: FormFieldInfo, choice: FormChoice): string {
  if (choice.text !== undefined) return choice.text;
  const first = choice.selected[0];
  return first === undefined ? '' : (field.options[first]?.label ?? '');
}

/** A checkbox or radio widget's state: the user's click, else the file's. */
export function isOn(doc: DocState, field: FormFieldInfo): boolean {
  return doc.formButtonEdits[formButtonKey(field.page, field.index)] ?? field.checked;
}

/** Whether a field (all its widgets, by name) holds an answer. */
function answered(doc: DocState, widgets: FormFieldInfo[]): boolean {
  const first = widgets[0]!;
  switch (first.kind) {
    case 'text':
      return textValue(doc, first).trim() !== '';
    case 'combo':
    case 'list':
      return choiceText(first, choiceValue(doc, first)).trim() !== '';
    case 'checkbox':
    case 'radio':
      return widgets.some((w) => isOn(doc, w));
    default:
      return true;
  }
}

/** Fields grouped by name, in Tab order of their first widget. */
export function fieldsByName(fields: FormFieldInfo[]): FormFieldInfo[][] {
  const groups = new Map<string, FormFieldInfo[]>();
  for (const f of [...fields].sort((a, b) => a.tabOrder - b.tabOrder)) {
    const list = groups.get(f.name);
    if (list) list.push(f);
    else groups.set(f.name, [f]);
  }
  return [...groups.values()];
}

/** Required fields (counted once each, however many widgets they have) that
 * are still empty. Read-only fields are left out: nobody can fill them. */
export function requiredLeft(doc: DocState, fields: FormFieldInfo[]): number {
  let left = 0;
  for (const widgets of fieldsByName(fields.filter(isFillable))) {
    if (widgets[0]!.required && !answered(doc, widgets)) left += 1;
  }
  return left;
}

/** How many fields are marked required at all. */
export function requiredCount(fields: FormFieldInfo[]): number {
  return fieldsByName(fields.filter(isFillable)).filter((w) => w[0]!.required).length;
}

/** The stops Tab visits, in form order. A radio group is one stop, as in
 * every other form filler: arrow keys move within it. */
export function tabStops(fields: FormFieldInfo[]): FormFieldInfo[] {
  const seenGroups = new Set<string>();
  const out: FormFieldInfo[] = [];
  for (const f of [...fields].filter(isFillable).sort((a, b) => a.tabOrder - b.tabOrder)) {
    if (f.kind === 'radio') {
      if (seenGroups.has(f.name)) continue;
      seenGroups.add(f.name);
    }
    out.push(f);
  }
  return out;
}

/** The stop after (or before) `from`, wrapping round the document. */
export function nextStop(
  fields: FormFieldInfo[],
  from: FormFieldInfo,
  direction: 1 | -1
): FormFieldInfo | null {
  const stops = tabStops(fields);
  if (stops.length === 0) return null;
  let at = stops.findIndex(
    (s) =>
      (s.page === from.page && s.index === from.index) ||
      (from.kind === 'radio' && s.kind === 'radio' && s.name === from.name)
  );
  if (at < 0) {
    // Not a stop itself (should not happen): continue from its order.
    at = stops.findIndex((s) => s.tabOrder > from.tabOrder) - (direction === 1 ? 1 : 0);
  }
  return stops[(at + direction + stops.length) % stops.length] ?? null;
}

/** Where focus lands for a stop: in a radio group, the selected button. */
export function focusTarget(
  doc: DocState,
  fields: FormFieldInfo[],
  stop: FormFieldInfo
): FormFieldInfo {
  if (stop.kind !== 'radio') return stop;
  return (
    fields.find(
      (f) => f.kind === 'radio' && f.name === stop.name && isOn(doc, f) && isFillable(f)
    ) ?? stop
  );
}

/** A field's label for a list: its tooltip, else the last part of its name. */
export function fieldLabel(field: FormFieldInfo): string {
  const tip = field.tooltip.trim();
  if (tip) return tip;
  const parts = field.name.split('.');
  return (parts[parts.length - 1] ?? field.name).replace(/\[\d+\]$/, '') || field.name;
}

// ---------------------------------------------------------------------------
// Focus across pages
// ---------------------------------------------------------------------------

/** Mounted widget elements, so a focus request can reach one directly. */
const widgets = new Map<string, HTMLElement>();

const widgetKey = (docId: number, page: number, index: number) => `${docId}:${page}:${index}`;

/** A widget waiting to be focused once its page mounts. */
const [pendingFocus, setPendingFocus] = createSignal<string | null>(null);

/** Record a mounted widget; returns the matching unregister. Focuses it at
 * once if a request was waiting for it. */
export function registerWidget(
  docId: number,
  page: number,
  index: number,
  el: HTMLElement
): () => void {
  const key = widgetKey(docId, page, index);
  widgets.set(key, el);
  if (pendingFocus() === key) {
    setPendingFocus(null);
    // After the page has laid out, or focus would scroll to a stale place.
    queueMicrotask(() => el.focus());
  }
  return () => {
    if (widgets.get(key) === el) widgets.delete(key);
  };
}

/** Focus a widget, wherever it is. Returns true when it was mounted and took
 * focus now; false when it will be focused once its page is shown, which the
 * caller arranges by scrolling there. */
export function focusWidget(docId: number, field: FormFieldInfo): boolean {
  const key = widgetKey(docId, field.page, field.index);
  const el = widgets.get(key);
  if (el?.isConnected) {
    setPendingFocus(null);
    el.focus();
    return true;
  }
  setPendingFocus(key);
  return false;
}

/** Test hook: forget every registration. */
export function resetFormFocus(): void {
  widgets.clear();
  setPendingFocus(null);
}
