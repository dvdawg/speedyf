/** Interactive AcroForm widgets for one page.
 *
 * Checkboxes and radio buttons are operated where they are, on the page,
 * rather than through a list off to one side: a form asks its questions in
 * place and answering it anywhere else means holding the mapping between a
 * field's name and the sentence it belongs to in your head.
 *
 * Nothing here rewrites the page raster. A toggled widget is drawn as an
 * overlay and recorded as a model edit, exactly like an annotation, and the
 * engine applies it to the real field at save time.
 */
import { createResource, For, Show, useContext } from 'solid-js';
import type { FormFieldInfo } from '../../types/engine';
import { formButtonKey } from '../document/documentStore';
import { formFields, isClickableWidget } from './formStore';
import { TabContext } from '../../app/TabContext';

interface Props {
  docId: number;
  src: number;
  pageHeightPt: number;
  zoom: number;
}

export default function FormLayer(props: Props) {
  const { documentStore } = useContext(TabContext)!;
  const [fields] = createResource(
    () => props.docId,
    (docId) => formFields(docId)
  );

  const widgets = (): FormFieldInfo[] =>
    (fields() ?? []).filter(
      (f) => f.page === props.src && isClickableWidget(f) && f.rect.w > 0 && f.rect.h > 0
    );

  /** A widget's state: the user's edit if they made one, else the file's. */
  const isChecked = (f: FormFieldInfo) =>
    documentStore.state.formButtonEdits[formButtonKey(f.page, f.index)] ?? f.checked;

  const toggle = (f: FormFieldInfo) => {
    const next = !isChecked(f);
    documentStore.apply({
      type: 'setFormButton',
      page: f.page,
      index: f.index,
      checked: next,
    });
    // Selecting a radio clears the rest of its group. The engine enforces
    // that when it writes the field — the group's value lives on the shared
    // parent — but the overlay has to show it immediately, or two buttons
    // would appear selected until the file was saved and reopened.
    if (next && f.kind === 'radio') {
      for (const other of fields() ?? []) {
        if (
          other.kind === 'radio' &&
          other.name === f.name &&
          !(other.page === f.page && other.index === f.index) &&
          isChecked(other)
        ) {
          documentStore.apply({
            type: 'setFormButton',
            page: other.page,
            index: other.index,
            checked: false,
          });
        }
      }
    }
  };

  return (
    <Show when={widgets().length > 0}>
      <div class="form-layer">
        <For each={widgets()}>
          {(f) => (
            <button
              type="button"
              class="form-widget"
              classList={{ 'is-checked': isChecked(f), 'is-radio': f.kind === 'radio' }}
              role={f.kind === 'radio' ? 'radio' : 'checkbox'}
              aria-checked={isChecked(f)}
              aria-label={f.name || (f.kind === 'radio' ? 'Radio button' : 'Checkbox')}
              style={{
                left: `${f.rect.x * props.zoom}px`,
                top: `${(props.pageHeightPt - f.rect.y - f.rect.h) * props.zoom}px`,
                width: `${f.rect.w * props.zoom}px`,
                height: `${f.rect.h * props.zoom}px`,
              }}
              onClick={() => toggle(f)}
            />
          )}
        </For>
      </div>
    </Show>
  );
}
