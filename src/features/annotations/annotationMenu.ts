/** The right-click menu on an annotation: edit its text, change its colour,
 * delete it - what selecting it and using the toolbar or Delete already
 * does, from where the pointer is. */
import { SEPARATOR, type MenuEntry } from '../../lib/nativeMenu';
import type { Annotation } from '../../types/model';

/** The colours offered, the annotation tools' own defaults (toolStore). */
export const ANNOTATION_COLOURS: readonly { name: string; value: string }[] = [
  { name: 'Yellow', value: '#ffd54a' },
  { name: 'Orange', value: '#fb8c00' },
  { name: 'Red', value: '#e53935' },
  { name: 'Blue', value: '#1e88e5' },
  { name: 'Black', value: '#1a1a1a' },
];

/** Kinds whose colour is theirs to change. A signature, an image or a
 * redaction mark looks the way it does for a reason. */
const RECOLOURABLE = new Set(['highlight', 'ink', 'rect', 'textbox', 'note', 'mark']);

export interface AnnotationMenuActions {
  /** Whether the document lets annotations be changed. */
  editable: boolean;
  editText: () => void;
  recolour: (color: string) => void;
  remove: () => void;
}

export function annotationMenuEntries(
  annot: Annotation,
  actions: AnnotationMenuActions
): MenuEntry[] {
  const entries: MenuEntry[] = [];
  if (annot.kind === 'textbox' || annot.kind === 'note') {
    entries.push({
      text: annot.kind === 'note' ? 'Edit Note' : 'Edit Text',
      enabled: actions.editable,
      run: actions.editText,
    });
  }
  if (RECOLOURABLE.has(annot.kind)) {
    const current = annot.color.toLowerCase();
    entries.push({
      kind: 'submenu',
      text: 'Colour',
      enabled: actions.editable,
      items: ANNOTATION_COLOURS.map((c) => ({
        text: c.name,
        checked: c.value === current,
        run: () => actions.recolour(c.value),
      })),
    });
  }
  entries.push(SEPARATOR, { text: 'Delete', enabled: actions.editable, run: actions.remove });
  return entries;
}
