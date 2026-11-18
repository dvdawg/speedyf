/** Moving between form fields: Tab on the page and clicks in the panel both
 * end here, so a field on a page that is not mounted yet is scrolled to and
 * focused the moment it appears. */
import type { FormFieldInfo } from '../../types/engine';
import { formButtonKey, type DocumentStore, type FormValues } from '../document/documentStore';
import type { ViewportStore } from '../../stores/viewportStore';
import { layoutIndexOf } from '../outline/jumpToAnchor';
import { recordJumpForViewport } from '../viewer/navHistory';
import { focusTarget, focusWidget, isFillable } from './formStore';

interface TabLike {
  documentStore: DocumentStore;
  viewport: ViewportStore;
}

/** How far down the viewport a field scrolled into view lands. */
const LAND_AT = 0.3;

/** Focus `stop` (a radio group lands on its selected button), scrolling its
 * page into view first if it is not on screen. A jump from the panel is
 * recorded for Back; Tab between fields is not. */
export function goToField(
  tab: TabLike,
  fields: FormFieldInfo[],
  stop: FormFieldInfo,
  opts: { record?: boolean } = {}
): void {
  const doc = tab.documentStore.state;
  const vp = tab.viewport;
  const target = focusTarget(doc, fields, stop);
  const page = layoutIndexOf(doc, target.page);
  if (page === null) return; // the page was deleted in this session
  const geom = doc.pages[page]!;
  const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
  const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
  const top = target.rect.y + target.rect.h;
  const offsetCss = Math.max(0, (heightPt - top) * vp.state.zoom - vp.state.containerH * LAND_AT);
  if (opts.record) recordJumpForViewport(vp, { page, offsetCss });
  if (focusWidget(doc.docId, target)) {
    // Mounted: focusing scrolled it into view if it needed to be. A panel
    // jump still frames it the way every other jump does.
    if (opts.record) vp.requestScrollToPage(page, offsetCss);
    return;
  }
  vp.requestScrollToPage(page, offsetCss);
}

/** Put every fillable field back to its default (/DV), as one undo step.
 *
 * Read-only fields are left alone, and so is a fixed combo with no default:
 * the engine has no way to empty one, so showing it empty would be a lie
 * the saved file did not keep. */
export function resetForm(tab: TabLike, fields: FormFieldInfo[]): void {
  const values: FormValues = { texts: {}, buttons: {}, choices: {} };
  for (const f of fields.filter(isFillable)) {
    switch (f.kind) {
      case 'text':
        values.texts[f.name] = f.defaultValue;
        break;
      case 'checkbox':
      case 'radio':
        values.buttons[formButtonKey(f.page, f.index)] = f.defaultChecked;
        break;
      case 'combo':
      case 'list':
        if (f.defaultSelected.length > 0 || f.kind === 'list') {
          values.choices[f.name] = { selected: [...f.defaultSelected] };
        } else if (f.editable) {
          values.choices[f.name] = { selected: [], text: f.defaultValue };
        }
        break;
    }
  }
  tab.documentStore.apply({ type: 'setFormValues', values });
}
