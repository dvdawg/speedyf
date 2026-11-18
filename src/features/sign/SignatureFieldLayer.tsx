/** Empty signature form fields on one page, made clickable: a click fits the
 * user's signature into the field, or opens the create dialog first when
 * there is none yet.
 *
 * What lands in the field is a visual signature - a picture - and every
 * label here says so. Certificate signing is a separate thing this does not
 * do; a field that already holds a signature is left alone. */
import { createResource, For, Show, useContext } from 'solid-js';
import type { FormFieldInfo } from '../../types/engine';
import type { PageEntry, PdfRect } from '../../types/model';
import { formFields } from '../forms/formStore';
import { TabContext } from '../../app/TabContext';
import { fitIntoField } from './art';
import {
  defaultSignature,
  isOpenSignatureField,
  loadSignatures,
  openSignDialog,
  placeSignature,
} from './signStore';
import './sign.css';

interface Props {
  docId: number;
  src: number;
  page: PageEntry;
  zoom: number;
}

export const SIGN_FIELD_LABEL =
  'Place your visual signature here (a picture of your signature, not a certificate signature)';

const covers = (outer: PdfRect, inner: PdfRect) => {
  const cx = inner.x + inner.w / 2;
  const cy = inner.y + inner.h / 2;
  return cx >= outer.x && cx <= outer.x + outer.w && cy >= outer.y && cy <= outer.y + outer.h;
};

export default function SignatureFieldLayer(props: Props) {
  const { documentStore } = useContext(TabContext)!;
  const [fields] = createResource(
    () => props.docId,
    (docId) => formFields(docId)
  );

  /** Open fields with no signature placed in them yet this session. */
  const open = () => {
    const placed = (documentStore.state.annotations[props.page.id] ?? []).filter(
      (a) => a.kind === 'signature'
    );
    return (fields() ?? []).filter(
      (f) =>
        f.page === props.src &&
        isOpenSignatureField(f) &&
        !placed.some((a) => covers(f.rect, a.rect))
    );
  };

  const sign = async (f: FormFieldInfo) => {
    await loadSignatures();
    const saved = defaultSignature('signature');
    if (saved) {
      placeSignature(documentStore, props.page.id, fitIntoField(saved.art, f.rect), saved);
    } else {
      openSignDialog('signature', { store: documentStore, pageId: props.page.id, rect: f.rect });
    }
  };

  return (
    <Show when={open().length > 0}>
      <div class="sign-field-layer">
        <For each={open()}>
          {(f) => (
            <button
              type="button"
              class="sign-field"
              title={SIGN_FIELD_LABEL}
              aria-label={`${f.name ? `${f.name}: ` : ''}${SIGN_FIELD_LABEL}`}
              style={{
                left: `${f.rect.x * props.zoom}px`,
                top: `${(props.page.heightPt - f.rect.y - f.rect.h) * props.zoom}px`,
                width: `${f.rect.w * props.zoom}px`,
                height: `${f.rect.h * props.zoom}px`,
              }}
              onClick={() => void sign(f)}
            />
          )}
        </For>
      </div>
    </Show>
  );
}
