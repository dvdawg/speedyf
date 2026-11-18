/** The format and calculation choices the properties panel offers, and
 * the field formats (the standard Acrobat helpers, engine/forms/scripts.rs)
 * each one is written as. */
import type { FieldCalc, FieldFormat } from '../../types/engine';

export type FormatChoice =
  | 'none'
  | 'number'
  | 'currency'
  | 'percent'
  | 'date'
  | 'time'
  | 'zip'
  | 'zip4'
  | 'phone'
  | 'ssn'
  | 'mask';

export const FORMAT_CHOICES: { value: FormatChoice; label: string }[] = [
  { value: 'none', label: 'None' },
  { value: 'number', label: 'Number' },
  { value: 'currency', label: 'Currency' },
  { value: 'percent', label: 'Percent' },
  { value: 'date', label: 'Date' },
  { value: 'time', label: 'Time' },
  { value: 'zip', label: 'ZIP code' },
  { value: 'zip4', label: 'ZIP + 4' },
  { value: 'phone', label: 'Phone number' },
  { value: 'ssn', label: 'Social security number' },
  { value: 'mask', label: 'Custom mask' },
];

export const DATE_PICTURES = ['mm/dd/yyyy', 'dd/mm/yyyy', 'yyyy-mm-dd', 'm/d/yy', 'mmm d, yyyy'];
export const TIME_PICTURES = ['HH:MM', 'h:MM tt', 'HH:MM:ss'];

export const CALC_OPS: { value: string; label: string }[] = [
  { value: 'SUM', label: 'Sum' },
  { value: 'PRD', label: 'Product' },
  { value: 'AVG', label: 'Average' },
  { value: 'MIN', label: 'Minimum' },
  { value: 'MAX', label: 'Maximum' },
];

/** Which choice a field's format is. */
export function choiceOf(format: FieldFormat | null | undefined): FormatChoice {
  if (!format) return 'none';
  switch (format.kind) {
    case 'number':
      return format.currency ? 'currency' : 'number';
    case 'percent':
      return 'percent';
    case 'date':
      return 'date';
    case 'time':
      return 'time';
    case 'special':
      return (['zip', 'zip4', 'phone', 'ssn'] as const)[format.style] ?? 'zip';
    case 'mask':
      return 'mask';
  }
}

/** The format a choice starts as; `datePicture` is the reader's. */
export function formatFor(choice: FormatChoice, datePicture = 'mm/dd/yyyy'): FieldFormat | null {
  switch (choice) {
    case 'none':
      return null;
    case 'number':
      return { kind: 'number', decimals: 2, sepStyle: 0, negStyle: 0, currency: '', prepend: true };
    case 'currency':
      return {
        kind: 'number',
        decimals: 2,
        sepStyle: 0,
        negStyle: 0,
        currency: '$',
        prepend: true,
      };
    case 'percent':
      return { kind: 'percent', decimals: 2, sepStyle: 0, prepend: false };
    case 'date':
      return { kind: 'date', format: datePicture };
    case 'time':
      return { kind: 'time', format: 'HH:MM' };
    case 'zip':
      return { kind: 'special', style: 0 };
    case 'zip4':
      return { kind: 'special', style: 1 };
    case 'phone':
      return { kind: 'special', style: 2 };
    case 'ssn':
      return { kind: 'special', style: 3 };
    case 'mask':
      return { kind: 'mask', mask: '999-999' };
  }
}

/** A calculation with `op` over `fields`, or none when nothing is chosen. */
export function calcFor(op: string, fields: string[]): FieldCalc | null {
  if (!op || op === 'none') return null;
  return { op, fields: [...fields] };
}
