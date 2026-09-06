/** One fetch of a document's AcroForm fields, shared by every mounted page.
 *
 * Form fields come back per *document*, not per page — one call describes the
 * whole file. Left to each page to fetch for itself, opening a long form would
 * fire the same request once per visible page and get the same answer every
 * time, so the promise is cached here and the pages filter what they need out
 * of it.
 */
import type { FormFieldInfo } from '../../types/engine';
import { engine } from '../../lib/transport/engine';

const inFlight = new Map<number, Promise<FormFieldInfo[]>>();

export function formFields(docId: number): Promise<FormFieldInfo[]> {
  const hit = inFlight.get(docId);
  if (hit) return hit;
  // A document with no form is the common case and answers with an empty
  // list; a failure is treated the same way, since a viewer that cannot read
  // the fields should still show the page.
  const request = engine.getFormFields(docId).catch(() => [] as FormFieldInfo[]);
  inFlight.set(docId, request);
  return request;
}

/** Drop the cached fields for a document, so the next read sees the file as
 * it is now. Called when a document is closed or reloaded. */
export function clearFormFields(docId?: number) {
  if (docId === undefined) inFlight.clear();
  else inFlight.delete(docId);
}

/** Widgets a reader can operate directly on the page. Text fields are edited
 * in the form panel, where there is room for a caret and a keyboard. */
export function isClickableWidget(field: FormFieldInfo): boolean {
  return (field.kind === 'checkbox' || field.kind === 'radio') && !field.readOnly;
}
