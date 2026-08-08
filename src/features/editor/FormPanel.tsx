/** AcroForm text-field editor. Values are stored as model edits (undoable)
 * and applied by the engine at save time. Non-text fields are listed but
 * read-only in this version. */
import { createResource, For, Show, useContext } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';

export default function FormPanel() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp } = tab;
  const doc = documentStore.state;
  const [fields] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => engine.getFormFields(docId).catch(() => [])
  );

  const valueOf = (name: string, original: string) => doc.formEdits[name] ?? original;

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
              {(f) => (
                <label class="form-field">
                  <span class="form-name">
                    {f.name} <em>p.{f.page + 1}</em>
                  </span>
                  <Show
                    when={f.kind === 'text' && !f.readOnly}
                    fallback={<span class="form-ro">{f.value || '—'} (read-only here)</span>}
                  >
                    <input
                      type="text"
                      value={valueOf(f.name, f.value)}
                      onChange={(e) =>
                        documentStore.apply({
                          type: 'setForm',
                          field: f.name,
                          value: e.currentTarget.value,
                        })
                      }
                    />
                  </Show>
                </label>
              )}
            </For>
          </div>
          <div class="panel-note">Values are written into the PDF when you save.</div>
        </Show>
      </Show>
    </div>
  );
}
