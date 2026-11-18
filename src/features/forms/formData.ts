/** Form data in and out: FDF and XFDF (Acrobat's formats), JSON and CSV.
 *
 * Export writes each field's current value - the user's edit if there is
 * one, else the file's - by fully qualified name: text as typed, a checkbox
 * or radio group as its export value ("Off" when off), a choice as the
 * export values of what is selected. Import is the reverse, applied as one
 * `setFormValues` edit, so a single undo takes it back. The engine reads and
 * writes the files (src-tauri/src/engine/forms/data.rs). */
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import type { FormDataField, FormFieldInfo, FormInfo } from '../../types/engine';
import {
  formButtonKey,
  type DocState,
  type DocumentStore,
  type FormValues,
} from '../document/documentStore';
import { engine } from '../../lib/transport/engine';
import { flashStatus } from '../../stores/statusMessage';
import { choiceValue, fieldsByName, formInfo, isFillable, isOn, textValue } from './formStore';
import { calculatedValues } from './formBehaviour';

/** Every field's value, once per field, in Tab order. */
export function collectData(doc: DocState, fields: FormFieldInfo[]): FormDataField[] {
  const out: FormDataField[] = [];
  for (const widgets of fieldsByName(fields)) {
    const first = widgets[0]!;
    switch (first.kind) {
      case 'text':
        out.push({ name: first.name, kind: 'text', values: [textValue(doc, first)] });
        break;
      case 'checkbox':
      case 'radio': {
        const on = widgets.find((w) => isOn(doc, w));
        out.push({
          name: first.name,
          kind: first.kind,
          values: [on ? on.exportValue || 'Yes' : 'Off'],
        });
        break;
      }
      case 'combo':
      case 'list': {
        const choice = choiceValue(doc, first);
        const values =
          choice.text !== undefined
            ? [choice.text]
            : choice.selected.map((i) => first.options[i]?.export ?? '').filter((v) => v !== '');
        out.push({ name: first.name, kind: first.kind, values: values.length ? values : [''] });
        break;
      }
      default:
        break;
    }
  }
  return out;
}

/** Imported values as form edits: what matched a field, and the names that
 * matched nothing. Read-only fields are left as they are. */
export function valuesFromData(
  fields: FormFieldInfo[],
  data: FormDataField[]
): { values: FormValues; matched: number; unknown: string[] } {
  const values: FormValues = { texts: {}, buttons: {}, choices: {} };
  const byName = new Map(fieldsByName(fields).map((w) => [w[0]!.name, w]));
  let matched = 0;
  const unknown: string[] = [];
  for (const entry of data) {
    const widgets = byName.get(entry.name);
    if (!widgets) {
      unknown.push(entry.name);
      continue;
    }
    const first = widgets[0]!;
    // Read-only fields keep what the form says.
    if (!isFillable(first)) continue;
    matched += 1;
    const value = entry.values[0] ?? '';
    switch (first.kind) {
      case 'text':
        values.texts[first.name] = entry.values.join('\n');
        break;
      case 'checkbox':
      case 'radio': {
        const off = value === '' || value === 'Off' || value.toLowerCase() === 'false';
        for (const w of widgets) {
          const on =
            !off &&
            (w.exportValue === value ||
              // A checkbox named on in a data file made elsewhere.
              (first.kind === 'checkbox' &&
                ['yes', 'on', 'true', '1'].includes(value.toLowerCase())));
          values.buttons[formButtonKey(w.page, w.index)] = on;
        }
        break;
      }
      case 'combo':
      case 'list': {
        // A list may arrive with its selections joined (CSV).
        const wanted =
          first.kind === 'list' && entry.values.length === 1 && first.multiSelect
            ? value.split(';').map((v) => v.trim())
            : entry.values;
        const selected = first.options
          .map((o, i) => (wanted.includes(o.export) || wanted.includes(o.label) ? i : -1))
          .filter((i) => i >= 0);
        if (selected.length > 0 || value === '') {
          values.choices[first.name] = {
            selected: first.multiSelect ? selected : selected.slice(0, 1),
          };
        } else if (first.editable) {
          values.choices[first.name] = { selected: [], text: value };
        }
        break;
      }
      default:
        break;
    }
  }
  return { values, matched, unknown };
}

const FILTERS = [
  { name: 'Acrobat form data (FDF)', extensions: ['fdf'] },
  { name: 'XML form data (XFDF)', extensions: ['xfdf'] },
  { name: 'JSON', extensions: ['json'] },
  { name: 'CSV', extensions: ['csv'] },
];

interface TabLike {
  documentStore: DocumentStore;
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export async function exportFormData(tab: TabLike | null | undefined): Promise<void> {
  const doc = tab?.documentStore.state;
  if (!doc?.loaded) return;
  const info = await formInfo(doc.docId);
  const data = collectData(doc, info.fields);
  if (data.length === 0) {
    flashStatus('This document has no form fields to export');
    return;
  }
  const stem = (doc.name || 'form').replace(/\.pdf$/i, '');
  const picked = await saveDialog({
    title: 'Export form data',
    defaultPath: `${stem}.fdf`,
    filters: FILTERS,
  }).catch(() => null);
  if (typeof picked !== 'string') return;
  const dest = /\.(fdf|xfdf|json|csv)$/i.test(picked) ? picked : `${picked}.fdf`;
  try {
    await engine.exportFormData(dest, doc.path ? baseName(doc.path) : doc.name, data);
    flashStatus(
      `Exported ${data.length} field${data.length === 1 ? '' : 's'} to ${baseName(dest)}`
    );
  } catch (err) {
    flashStatus(
      `Could not export the form data: ${err instanceof Error ? err.message : String(err)}`
    );
  }
}

export async function importFormData(tab: TabLike | null | undefined): Promise<void> {
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) return;
  const picked = await openDialog({
    title: 'Import form data',
    multiple: false,
    filters: [{ name: 'Form data', extensions: ['fdf', 'xfdf', 'json', 'csv'] }, ...FILTERS],
  }).catch(() => null);
  if (typeof picked !== 'string') return;
  try {
    const data = await engine.importFormData(picked);
    const info: FormInfo = await formInfo(doc.docId);
    applyImported(tab, info, data);
  } catch (err) {
    flashStatus(
      `Could not import the form data: ${err instanceof Error ? err.message : String(err)}`
    );
  }
}

/** Apply imported values as one edit, calculations following. */
export function applyImported(tab: TabLike, info: FormInfo, data: FormDataField[]): number {
  const { values, matched, unknown } = valuesFromData(info.fields, data);
  if (matched === 0) {
    flashStatus('None of the imported fields are in this form');
    return 0;
  }
  const names = [
    ...Object.keys(values.texts),
    ...Object.keys(values.choices),
    ...info.fields
      .filter((f) => values.buttons[formButtonKey(f.page, f.index)] !== undefined)
      .map((f) => f.name),
  ];
  Object.assign(
    values.texts,
    calculatedValues(tab.documentStore.state, info, [...new Set(names)], values)
  );
  tab.documentStore.apply({ type: 'setFormValues', values });
  flashStatus(
    `Imported ${matched} field${matched === 1 ? '' : 's'}` +
      (unknown.length ? `; ${unknown.length} not in this form` : '')
  );
  return matched;
}
