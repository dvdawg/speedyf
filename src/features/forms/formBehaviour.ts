/** Where the form helpers (formScripts.ts) meet the document model: the
 * values calculations read, the edit a change becomes once the fields that
 * depend on it are recalculated, and what the save writes on the page.
 *
 * A change and the recalculation it causes are one model edit, so one undo
 * takes both back. */
import type { FieldBehaviour, FormFieldInfo, FormInfo } from '../../types/engine';
import {
  formButtonKey,
  type DocState,
  type EditOp,
  type EditPlan,
  type FormValues,
} from '../document/documentStore';
import { settings } from '../../stores/settings';
import {
  appearanceEntries,
  calculationsAfter,
  displayValue,
  recalculate,
  type Display,
} from './formScripts';
import { choiceValue, formInfo, isOn, textValue } from './formStore';

/** The date picture for a date field that names none: the reader's date
 * format setting when there is one, else US month/day/year. Kept as one
 * small accessor so a settings key can be wired in without touching the
 * helpers. A setting written the everyday way (MM/DD/YYYY) is lowered to
 * Acrobat's picture letters, where capital M means minutes. */
export function userDatePicture(): string {
  const value = (settings as unknown as Record<string, unknown>)['dateFormat'];
  return typeof value === 'string' && value.trim() ? value.trim().toLowerCase() : 'mm/dd/yyyy';
}

export function behaviourOf(info: FormInfo | undefined, name: string): FieldBehaviour | undefined {
  return info?.behaviours?.[name];
}

/** What a text field shows when it is not being edited. */
export function shownValue(info: FormInfo | undefined, name: string, raw: string): Display {
  return displayValue(behaviourOf(info, name), raw, userDatePicture);
}

/** Field names in Tab order, once each. */
function fieldNames(fields: FormFieldInfo[]): string[] {
  const names: string[] = [];
  for (const f of [...fields].sort((a, b) => a.tabOrder - b.tabOrder)) {
    if (!names.includes(f.name)) names.push(f.name);
  }
  return names;
}

/** A field's value as a script would read it (`field.value`): the text, a
 * choice's export value, a button's on state or Off. */
function scriptValue(doc: DocState, widgets: FormFieldInfo[]): string {
  const first = widgets[0]!;
  switch (first.kind) {
    case 'text':
      return textValue(doc, first);
    case 'combo':
    case 'list': {
      const choice = choiceValue(doc, first);
      if (choice.text !== undefined) return choice.text;
      const at = choice.selected[0];
      return at === undefined ? '' : (first.options[at]?.export ?? '');
    }
    case 'checkbox':
    case 'radio':
      return widgets.find((w) => isOn(doc, w))?.exportValue ?? 'Off';
    default:
      return first.value;
  }
}

/** The document as it will be once `change` is applied, for reading only. */
function withChange(doc: DocState, change: FormValues): DocState {
  return {
    ...doc,
    formEdits: { ...doc.formEdits, ...change.texts },
    formButtonEdits: { ...doc.formButtonEdits, ...change.buttons },
    formChoiceEdits: { ...doc.formChoiceEdits, ...change.choices },
  } as DocState;
}

/** Whether any calculation reads one of `names`. */
export function hasDependents(info: FormInfo | undefined, names: string[]): boolean {
  const behaviours = info?.behaviours;
  if (!info || !behaviours) return false;
  return (
    calculationsAfter(names, behaviours, info.calcOrder ?? [], fieldNames(info.fields)).length > 0
  );
}

/** Raw values of the calculated fields that move once `change` is in. */
export function calculatedValues(
  doc: DocState,
  info: FormInfo | undefined,
  changed: string[],
  change: FormValues
): Record<string, string> {
  const behaviours = info?.behaviours;
  if (!info || !behaviours) return {};
  const next = withChange(doc, change);
  const byName = new Map<string, FormFieldInfo[]>();
  for (const f of info.fields) {
    const list = byName.get(f.name);
    if (list) list.push(f);
    else byName.set(f.name, [f]);
  }
  return recalculate(changed, behaviours, info.calcOrder ?? [], [...byName.keys()], (name) => {
    const widgets = byName.get(name);
    return widgets ? scriptValue(next, widgets) : '';
  });
}

/** The edit for `change` to the fields `changed`: `plain` (the op the
 * widget would apply on its own) when no calculation reads them, else one
 * setFormValues carrying the change and every result it causes. A field
 * that has dependents always takes the second form, so typing in it keeps
 * coalescing into one undo step. */
export function formEdit(
  doc: DocState,
  info: FormInfo | undefined,
  changed: string[],
  change: FormValues,
  plain: EditOp
): EditOp {
  if (!hasDependents(info, changed)) return plain;
  const results = calculatedValues(doc, info, changed, change);
  return {
    type: 'setFormValues',
    values: {
      texts: { ...change.texts, ...results },
      buttons: { ...change.buttons },
      choices: { ...change.choices },
    },
  };
}

/** A single text value as a change. */
export function textChange(field: string, value: string): FormValues {
  return { texts: { [field]: value }, buttons: {}, choices: {} };
}

/** Button states as a change. */
export function buttonChange(states: [FormFieldInfo, boolean][]): FormValues {
  const buttons: Record<string, boolean> = {};
  for (const [f, on] of states) buttons[formButtonKey(f.page, f.index)] = on;
  return { texts: {}, buttons, choices: {} };
}

/** Text fields whose page appearance should show something other than the
 * raw value the save writes to /V. See `appearanceEntries`. */
export function formDisplay(
  info: FormInfo,
  texts: [string, string][]
): [string, string, boolean | null][] {
  return appearanceEntries(info.behaviours ?? {}, info.fields, texts, userDatePicture);
}

/** `plan` with the formatted appearance of every edited text field, so the
 * saved page shows what the overlay showed while /V keeps the raw value. */
export async function withFormDisplay(docId: number, plan: EditPlan): Promise<EditPlan> {
  if (plan.form.length === 0) return plan;
  const info = await formInfo(docId);
  const display = formDisplay(info, plan.form);
  return display.length > 0 ? { ...plan, formDisplay: display } : plan;
}
