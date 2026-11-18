/** A form field for tests, with every attribute at the engine's default. */
import type { FormFieldInfo } from '../../types/engine';

export function testField(over: Partial<FormFieldInfo> = {}): FormFieldInfo {
  return {
    name: 'agree',
    kind: 'checkbox',
    value: '',
    page: 0,
    readOnly: false,
    index: 0,
    rect: { x: 100, y: 345, w: 14, h: 14 },
    exportValue: 'Yes',
    checked: false,
    tooltip: '',
    required: false,
    multiline: false,
    password: false,
    comb: false,
    noScroll: false,
    multiSelect: false,
    editable: false,
    maxLen: 0,
    fontSize: 10,
    align: 0,
    color: '#000000',
    options: [],
    selected: [],
    defaultValue: '',
    defaultSelected: [],
    defaultChecked: false,
    scriptCount: 0,
    tabOrder: over.index ?? 0,
    ...over,
  };
}
