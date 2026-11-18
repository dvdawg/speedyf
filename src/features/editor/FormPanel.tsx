/** The form navigator: every fillable field, grouped by page and labelled the
 * way the form labels it, with what it holds and whether it is still owed an
 * answer. Clicking a row goes to the field on the page and focuses it there;
 * the page is where fields are filled.
 *
 * Values are model edits (undoable) that the engine writes at save time. */
import { createMemo, createResource, For, Show, useContext } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import type { FormFieldInfo, FormInfo } from '../../types/engine';
import type { DocState } from '../document/documentStore';
import {
  choiceText,
  choiceValue,
  fieldLabel,
  fieldsByName,
  formInfo,
  isFillable,
  isOn,
  requiredLeft,
  textValue,
} from '../forms/formStore';
import { goToField, resetForm } from '../forms/formNav';
import { behaviourOf, shownValue, userDatePicture } from '../forms/formBehaviour';
import { fieldProblem } from '../forms/formScripts';
import { layoutIndexOf } from '../outline/jumpToAnchor';
import { settings, updateSettings } from '../../stores/settings';

/** What a field holds, as one short line for the list: a text field as
 * its format shows it. */
export function fieldSummary(doc: DocState, widgets: FormFieldInfo[], info?: FormInfo): string {
  const first = widgets[0]!;
  switch (first.kind) {
    case 'text': {
      const raw = textValue(doc, first);
      if (first.password && raw) return '(hidden)';
      return shownValue(info, first.name, raw).text;
    }
    case 'combo':
    case 'list': {
      const choice = choiceValue(doc, first);
      if (first.kind === 'list' && choice.selected.length > 1) {
        return choice.selected.map((i) => first.options[i]?.label ?? '').join(', ');
      }
      return choiceText(first, choice);
    }
    case 'checkbox':
      return widgets.some((w) => isOn(doc, w)) ? 'Checked' : '';
    case 'radio': {
      const on = widgets.find((w) => isOn(doc, w));
      return on ? on.exportValue || 'Selected' : '';
    }
    default:
      return '';
  }
}

/** Scripts the form carries that SpeedyF did not do itself. */
export function scriptsNotRun(info: FormInfo | undefined): number {
  const s = info?.scripts;
  return s ? Math.max(0, s.total - s.recognised) + s.document : 0;
}

/** What a field's value breaks, if anything: a format it does not fit or
 * a range it falls outside. */
function problemOf(doc: DocState, info: FormInfo | undefined, field: FormFieldInfo) {
  if (field.kind !== 'text') return null;
  return fieldProblem(behaviourOf(info, field.name), textValue(doc, field), userDatePicture);
}

const SCRIPTS_NOTE =
  'This form carries JavaScript, which SpeedyF never runs. Standard number, date and ' +
  'special formats, range checks and simple sums are done by SpeedyF itself; anything ' +
  'else, such as a custom calculation or a pop-up check, is left out, so check those ' +
  'values yourself.';

export default function FormPanel() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp } = tab;
  const doc = documentStore.state;
  const [info] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => formInfo(docId)
  );
  const fields = () => info()?.fields ?? [];
  const fillable = createMemo(() => fields().filter(isFillable));

  /** Fields grouped by the page their first widget is on, in reading order
   * of the pages as they now stand. */
  const pages = createMemo(() => {
    const byPage = new Map<number, FormFieldInfo[][]>();
    for (const widgets of fieldsByName(fillable())) {
      const page = layoutIndexOf(doc, widgets[0]!.page);
      if (page === null) continue;
      const list = byPage.get(page);
      if (list) list.push(widgets);
      else byPage.set(page, [widgets]);
    }
    return [...byPage.entries()].sort((a, b) => a[0] - b[0]);
  });
  const left = () => requiredLeft(doc, fields());
  const unanswered = (widgets: FormFieldInfo[]) =>
    widgets[0]!.required && fieldSummary(doc, widgets, info()).trim() === '';
  const notRun = () => scriptsNotRun(info());

  return (
    <div class="form-panel" aria-label="Form fields">
      <div class="panel-head">
        <h2>Form</h2>
        <IconButton label="Close form panel" onClick={() => vp.setState('formPanelOpen', false)}>
          <IconClose />
        </IconButton>
      </div>
      <Show when={!info.loading} fallback={<div class="panel-note">Reading form fields...</div>}>
        <Show
          when={fillable().length > 0}
          fallback={
            <div class="panel-note">
              {info()?.xfa
                ? "This form uses XFA, which SpeedyF can't fill."
                : 'This document has no fillable form fields.'}
            </div>
          }
        >
          <div class="form-nav-bar">
            <span class="form-nav-count" classList={{ 'is-owed': left() > 0 }}>
              {left() > 0 ? `${left()} required left` : `${fieldsByName(fillable()).length} fields`}
            </span>
            <label class="form-nav-toggle">
              <input
                type="checkbox"
                checked={settings.highlightFields}
                onInput={(e) => updateSettings({ highlightFields: e.currentTarget.checked })}
              />
              Highlight fields
            </label>
          </div>
          <div class="form-list" role="list">
            <For each={pages()}>
              {([page, groups]) => (
                <section class="form-page" aria-label={`Page ${page + 1}`}>
                  <h3 class="form-page-head">Page {page + 1}</h3>
                  <For each={groups}>
                    {(widgets) => {
                      const problem = () => problemOf(doc, info(), widgets[0]!);
                      return (
                        <>
                          <button
                            type="button"
                            role="listitem"
                            class="form-nav-row"
                            classList={{
                              'is-owed': unanswered(widgets),
                              'has-problem': problem() !== null,
                            }}
                            title={widgets[0]!.name}
                            onClick={() => goToField(tab, fields(), widgets[0]!, { record: true })}
                          >
                            <span class="form-nav-label">{fieldLabel(widgets[0]!)}</span>
                            <span class="form-nav-value">
                              {unanswered(widgets)
                                ? 'Required'
                                : fieldSummary(doc, widgets, info())}
                            </span>
                          </button>
                          <Show when={problem()}>
                            {(message) => <div class="form-nav-problem">{message()}</div>}
                          </Show>
                        </>
                      );
                    }}
                  </For>
                </section>
              )}
            </For>
          </div>
          <Show when={info()?.xfa}>
            <div class="panel-note">
              This form also carries an XFA copy of its fields. Saving removes it, so other readers
              show the values you entered.
            </div>
          </Show>
          <Show when={notRun() > 0}>
            <div class="form-nav-scripts" title={SCRIPTS_NOTE}>
              {notRun() === 1 ? '1 script not run' : `${notRun()} scripts not run`}
            </div>
          </Show>
          <div class="form-nav-foot">
            <span>Values are written into the PDF when you save.</span>
            <button
              type="button"
              class="form-nav-reset"
              onClick={() => resetForm(tab, fields(), info())}
            >
              Reset form
            </button>
          </div>
        </Show>
      </Show>
    </div>
  );
}
