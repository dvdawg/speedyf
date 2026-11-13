/** AcroForm text-field editor. Values are stored as model edits (undoable)
 * and applied by the engine at save time.
 *
 * Checkboxes and radio buttons are not edited here — they are clicked on the
 * page itself, where the question they answer is written. This panel shows
 * their state so a long form can be checked over at a glance. */
import { createResource, For, Show, useContext } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import type { FormFieldInfo } from '../../types/engine';
import { formButtonKey } from '../document/documentStore';
import { formFields } from '../forms/formStore';

export default function FormPanel() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp } = tab;
  const doc = documentStore.state;
  const [fields] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => formFields(docId)
  );

  const valueOf = (name: string, original: string) => doc.formEdits[name] ?? original;

  /** A button's state, as the page currently shows it: the reader's click if
   * they made one, otherwise what the file says. */
  const buttonState = (f: FormFieldInfo) => {
    const checked = doc.formButtonEdits[formButtonKey(f.page, f.index)] ?? f.checked;
    const label = checked ? 'checked' : 'unchecked';
    return f.exportValue && checked ? `${label} (${f.exportValue})` : label;
  };

  return (
    <div class="form-panel" aria-label="Form fields">
      <div class="panel-head">
        <h2>Form fields</h2>
        <IconButton label="Close form panel" onClick={() => vp.setState('formPanelOpen', false)}>
          <IconClose />
        </IconButton>
      </div>
      <Show when={!fields.loading} fallback={<div class="panel-note">Reading form fields…</div>}>
        <Show
          when={(fields() ?? []).length > 0}
          fallback={<div class="panel-note">This document has no AcroForm fields.</div>}
        >
          <div class="form-list">
            <For each={fields()}>
              {(f) => {
                let typingGroup = Symbol('form text edit');
                return (
                  <label class="form-field">
                    <span class="form-name">
                      {f.name} <em>p.{f.page + 1}</em>
                    </span>
                    <Show
                      when={f.kind === 'text' && !f.readOnly}
                      fallback={
                        <span class="form-ro">
                          {f.kind === 'checkbox' || f.kind === 'radio'
                            ? buttonState(f)
                            : f.value || '—'}
                        </span>
                      }
                    >
                      <input
                        type="text"
                        value={valueOf(f.name, f.value)}
                        onFocus={() => {
                          typingGroup = Symbol('form text edit');
                        }}
                        onInput={(e) =>
                          documentStore.apply(
                            {
                              type: 'setForm',
                              field: f.name,
                              value: e.currentTarget.value,
                            },
                            typingGroup
                          )
                        }
                      />
                    </Show>
                  </label>
                );
              }}
            </For>
          </div>
          <div class="panel-note">
            Values are written into the PDF when you save. Tick checkboxes and radio buttons on the
            page itself.
          </div>
        </Show>
      </Show>
    </div>
  );
}
