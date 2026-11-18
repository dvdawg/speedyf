import { describe, expect, it } from 'vitest';
import type { FieldBehaviour, FieldFormat } from '../../types/engine';
import {
  acceptKeystroke,
  appearanceEntries,
  calculationsAfter,
  commitValue,
  displayValue,
  fieldProblem,
  fitMask,
  formatNumber,
  formatPercent,
  formatSpecial,
  formatValue,
  makeNumber,
  parseDate,
  parseTime,
  printNumber,
  printd,
  printx,
  recalculate,
  simpleCalculate,
  validateRange,
} from './formScripts';

type NumberSpec = Extract<FieldFormat, { kind: 'number' }>;
const num = (over: Partial<NumberSpec> = {}): NumberSpec => ({
  kind: 'number',
  decimals: 2,
  sepStyle: 0,
  negStyle: 0,
  currency: '',
  prepend: true,
  ...over,
});

const NOW = new Date(2026, 10, 17, 9, 30, 0);

describe('makeNumber (AFMakeNumber)', () => {
  it('reads plain, signed and comma-decimal numbers', () => {
    expect(makeNumber('42')).toBe(42);
    expect(makeNumber(' -3.5 ')).toBe(-3.5);
    expect(makeNumber('+7')).toBe(7);
    expect(makeNumber('1,5')).toBe(1.5);
    expect(makeNumber('.25')).toBe(0.25);
    expect(makeNumber('12.')).toBe(12);
  });
  it('reads a value grouped in thousands', () => {
    expect(makeNumber('1,234,567.89')).toBe(1234567.89);
  });
  it('is null for empty and non-numeric values', () => {
    expect(makeNumber('')).toBeNull();
    expect(makeNumber('abc')).toBeNull();
    expect(makeNumber('Off')).toBeNull();
    expect(makeNumber('1.2.3')).toBeNull();
  });
});

describe('printNumber (util.printf %,s.df)', () => {
  const v = 1234567.891;
  it.each([
    [0, '1,234,567.89'],
    [1, '1234567.89'],
    [2, '1.234.567,89'],
    [3, '1234567,89'],
    [4, "1'234'567.89"],
  ])('separator style %i', (sep, out) => {
    expect(printNumber(v, 2, sep)).toBe(out);
  });
  it('rounds to the requested decimals and drops the point at zero', () => {
    expect(printNumber(2.5, 0, 0)).toBe('3');
    expect(printNumber(1234.5, 0, 0)).toBe('1,235');
    expect(printNumber(0.125, 1, 0)).toBe('0.1');
    expect(printNumber(999.999, 2, 0)).toBe('1,000.00');
    expect(printNumber(-1234.5, 3, 2)).toBe('-1.234,500');
  });
  it('groups only the integer part', () => {
    expect(printNumber(123, 4, 0)).toBe('123.0000');
    expect(printNumber(1000, 2, 4)).toBe("1'000.00");
  });
});

describe('formatNumber (AFNumber_Format)', () => {
  it('plain positive values', () => {
    expect(formatNumber(num(), '1234.5').text).toBe('1,234.50');
    expect(formatNumber(num({ decimals: 0, sepStyle: 1 }), '1234.5').text).toBe('1235');
  });
  it('empty or non-numeric values show nothing', () => {
    expect(formatNumber(num(), '').text).toBe('');
    expect(formatNumber(num(), 'n/a').text).toBe('');
  });
  it('negative style 0: minus sign, before a prepended currency', () => {
    expect(formatNumber(num(), '-1234.5')).toEqual({ text: '-1,234.50', red: false });
    expect(formatNumber(num({ currency: '$' }), '-1234.5').text).toBe('-$1,234.50');
    expect(formatNumber(num({ currency: ' EUR', prepend: false }), '-5').text).toBe('-5.00 EUR');
  });
  it('negative style 1: red, no sign', () => {
    expect(formatNumber(num({ negStyle: 1 }), '-1234.5')).toEqual({
      text: '1,234.50',
      red: true,
    });
    expect(formatNumber(num({ negStyle: 1 }), '5').red).toBe(false);
  });
  it('negative style 2: parentheses round currency and number', () => {
    expect(formatNumber(num({ negStyle: 2, currency: '$' }), '-1234.5')).toEqual({
      text: '($1,234.50)',
      red: false,
    });
    expect(formatNumber(num({ negStyle: 2, currency: '$' }), '1234.5').text).toBe('$1,234.50');
  });
  it('negative style 3: red parentheses', () => {
    expect(formatNumber(num({ negStyle: 3 }), '-0.5')).toEqual({ text: '(0.50)', red: true });
  });
  it('currency after the number', () => {
    expect(formatNumber(num({ currency: ' kr', prepend: false, sepStyle: 2 }), '1234.5').text).toBe(
      '1.234,50 kr'
    );
  });
  it('accepts comma-decimal raw values', () => {
    expect(formatNumber(num({ sepStyle: 2 }), '1234,5').text).toBe('1.234,50');
  });
});

describe('formatPercent (AFPercent_Format)', () => {
  const pct = (decimals: number, sepStyle = 0, prepend = false) =>
    ({ kind: 'percent', decimals, sepStyle, prepend }) as const;
  it('multiplies by 100', () => {
    expect(formatPercent(pct(2), '0.125').text).toBe('12.50%');
    expect(formatPercent(pct(0), '12.34').text).toBe('1,234%');
    expect(formatPercent(pct(1, 2), '0.5').text).toBe('50,0%');
    expect(formatPercent(pct(0, 0, true), '0.07').text).toBe('%7');
  });
  it('empty stays empty', () => {
    expect(formatPercent(pct(2), '').text).toBe('');
  });
});

describe('number keystrokes (AFNumber_Keystroke)', () => {
  const dot = num();
  const comma = num({ sepStyle: 2 });
  it.each(['1', '-', '+', '12.', '.5', '-1.25', '', '007'])('dot style accepts %j', (v) => {
    expect(acceptKeystroke(dot, v).ok).toBe(true);
  });
  it.each(['a', '1,0', '1.2.3', '1-', '--1', '1e5', '$5'])('dot style rejects %j', (v) => {
    expect(acceptKeystroke(dot, v).ok).toBe(false);
  });
  it('comma style takes a comma decimal and not a point', () => {
    expect(acceptKeystroke(comma, '12,5').ok).toBe(true);
    expect(acceptKeystroke(comma, '12.5').ok).toBe(false);
  });
  it('commit rejects a lone sign or point, and normalises a comma decimal', () => {
    expect(commitValue(dot, '-').error).not.toBeNull();
    expect(commitValue(dot, '.').error).not.toBeNull();
    expect(commitValue(dot, '12.')).toEqual({ value: '12.', error: null });
    expect(commitValue(comma, '12,5')).toEqual({ value: '12.5', error: null });
    expect(commitValue(dot, '')).toEqual({ value: '', error: null });
  });
  it('percent keystrokes follow the number rules', () => {
    const pct = { kind: 'percent', decimals: 1, sepStyle: 0, prepend: false } as const;
    expect(acceptKeystroke(pct, '12.5').ok).toBe(true);
    expect(acceptKeystroke(pct, '12%').ok).toBe(false);
  });
});

describe('printx (util.printx)', () => {
  it('lays digits into a picture, skipping other characters', () => {
    expect(printx('999-99-9999', '123456789')).toBe('123-45-6789');
    expect(printx('999-99-9999', '123-45-6789')).toBe('123-45-6789');
    expect(printx('(999) 999-9999', '555 123 4567')).toBe('(555) 123-4567');
  });
  it('stops when the source runs out', () => {
    expect(printx('999-99-9999', '1234')).toBe('123-4');
  });
  it('handles letters, any-character, rest, case and escapes', () => {
    expect(printx('AAA-999', 'ab1c23')).toBe('abc-23');
    expect(printx('>AAA', 'abc')).toBe('ABC');
    expect(printx('??-*', 'abcdef')).toBe('ab-cdef');
    expect(printx('\\9-9', '5')).toBe('9-5');
    expect(printx('XX', '-a-1')).toBe('a1');
  });
});

describe('AFSpecial', () => {
  it('formats zip, zip+4, phone and SSN', () => {
    expect(formatSpecial(0, '94720')).toBe('94720');
    expect(formatSpecial(1, '947201234')).toBe('94720-1234');
    expect(formatSpecial(2, '5551234567')).toBe('(555) 123-4567');
    expect(formatSpecial(2, '1234567')).toBe('123-4567');
    expect(formatSpecial(3, '123456789')).toBe('123-45-6789');
    expect(formatSpecial(3, '')).toBe('');
  });
  const sp = (style: number) => ({ kind: 'special', style }) as const;
  it('zip keystrokes: five digits at most', () => {
    expect(acceptKeystroke(sp(0), '9472').ok).toBe(true);
    expect(acceptKeystroke(sp(0), '947201').ok).toBe(false);
    expect(acceptKeystroke(sp(0), '94a').ok).toBe(false);
    expect(commitValue(sp(0), '9472').error).not.toBeNull();
    expect(commitValue(sp(0), '94720').error).toBeNull();
  });
  it('zip+4 takes an optional separator', () => {
    expect(acceptKeystroke(sp(1), '94720-12').ok).toBe(true);
    expect(commitValue(sp(1), '947201234').error).toBeNull();
    expect(commitValue(sp(1), '94720 1234').error).toBeNull();
    expect(commitValue(sp(1), '94720-123').error).not.toBeNull();
  });
  it('phone: local, with area code, in brackets, international', () => {
    for (const v of ['555', '555-1', '(555', '(555) 12', '555.123.4567', '011 44 20']) {
      expect(acceptKeystroke(sp(2), v).ok, v).toBe(true);
    }
    expect(acceptKeystroke(sp(2), '555-123-45678').ok).toBe(false);
    expect(commitValue(sp(2), '1234567').error).toBeNull();
    expect(commitValue(sp(2), '(555) 123-4567').error).toBeNull();
    expect(commitValue(sp(2), '555123456').error).not.toBeNull();
  });
  it('SSN', () => {
    expect(acceptKeystroke(sp(3), '123-45').ok).toBe(true);
    expect(acceptKeystroke(sp(3), '1234567890').ok).toBe(false);
    expect(commitValue(sp(3), '123456789').error).toBeNull();
    expect(commitValue(sp(3), '123-45-678').error).not.toBeNull();
    expect(displayValue({ format: sp(3) }, '123456789').text).toBe('123-45-6789');
  });
});

describe('arbitrary masks (AFSpecial_KeystrokeEx)', () => {
  const mask = { kind: 'mask', mask: '999-9999-9' } as const;
  it('puts the mask characters in as the user types', () => {
    expect(fitMask(mask.mask, '1234')).toBe('123-4');
    expect(acceptKeystroke(mask, '123')).toEqual({ ok: true, value: '123' });
    expect(acceptKeystroke(mask, '1234')).toEqual({ ok: true, value: '123-4' });
    expect(acceptKeystroke(mask, '123-4567-8')).toEqual({ ok: true, value: '123-4567-8' });
  });
  it('re-lays the value after a deletion inside it', () => {
    expect(acceptKeystroke(mask, '123-567-8').value).toBe('123-5678');
  });
  it('rejects characters that do not fit and values that are too long', () => {
    expect(acceptKeystroke(mask, '12a').ok).toBe(false);
    expect(acceptKeystroke(mask, '12345678901').ok).toBe(false);
  });
  it('requires the whole mask on commit', () => {
    expect(commitValue(mask, '123-4567-8').error).toBeNull();
    expect(commitValue(mask, '123-45').error).toContain('999-9999-9');
  });
  it('letter and alphanumeric cells', () => {
    expect(fitMask('AA-OO', 'ab1c')).toBe('ab-1c');
    expect(fitMask('AA-OO', 'a1')).toBeNull();
    expect(fitMask('XX', '-')).toBe('-');
  });
});

describe('dates (AFDate_*)', () => {
  const d = (y: number, m: number, day: number, h = 0, min = 0) => new Date(y, m - 1, day, h, min);
  it('printd pictures', () => {
    const date = d(2026, 3, 5, 14, 7);
    expect(printd('mm/dd/yyyy', date)).toBe('03/05/2026');
    expect(printd('m/d/yy', date)).toBe('3/5/26');
    expect(printd('d-mmm-yy', date)).toBe('5-Mar-26');
    expect(printd('dd-mmm-yy', date)).toBe('05-Mar-26');
    expect(printd('mmmm d, yyyy', date)).toBe('March 5, 2026');
    expect(printd('mmm d, yyyy', date)).toBe('Mar 5, 2026');
    expect(printd('yy-mm-dd', date)).toBe('26-03-05');
    expect(printd('dddd', date)).toBe('Thursday');
    expect(printd('m/d/yy h:MM tt', date)).toBe('3/5/26 2:07 pm');
    expect(printd('m/d/yy HH:MM', date)).toBe('3/5/26 14:07');
    expect(printd('\\d\\a\\y d', date)).toBe('day 5');
  });
  it('parses values in the picture and loosely around it', () => {
    expect(parseDate('mm/dd/yyyy', '03/05/2026')).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/dd/yyyy', '3/5/2026')).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/dd/yyyy', '3-5-26')).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/dd/yyyy', '03052026')).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/dd/yyyy', '030526')).toEqual(d(2026, 3, 5));
    expect(parseDate('dd/mm/yyyy', '5/3/2026')).toEqual(d(2026, 3, 5));
    expect(parseDate('yy-mm-dd', '26-03-05')).toEqual(d(2026, 3, 5));
    expect(parseDate('d-mmm-yy', '5-Mar-26')).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/dd/yyyy', 'March 5, 2026')).toEqual(d(2026, 3, 5));
    expect(parseDate('mmmm d, yyyy', '5 march 2026')).toEqual(d(2026, 3, 5));
  });
  it('reads a two-digit year below 50 as this century', () => {
    expect(parseDate('m/d/yy', '1/2/49')!.getFullYear()).toBe(2049);
    expect(parseDate('m/d/yy', '1/2/50')!.getFullYear()).toBe(1950);
  });
  it('fills a missing year from today', () => {
    expect(parseDate('mm/dd/yyyy', '3/5', NOW)).toEqual(d(2026, 3, 5));
    expect(parseDate('mm/yy', '3/26')).toEqual(d(2026, 3, 1));
  });
  it('rejects dates that do not exist', () => {
    expect(parseDate('mm/dd/yyyy', '02/30/2026')).toBeNull();
    expect(parseDate('mm/dd/yyyy', '13/01/2026')).toBeNull();
    expect(parseDate('mm/dd/yyyy', '02/29/2025')).toBeNull();
    expect(parseDate('mm/dd/yyyy', '02/29/2024')).toEqual(d(2024, 2, 29));
    expect(parseDate('mm/dd/yyyy', 'tomorrow')).toBeNull();
    expect(parseDate('mm/dd/yyyy', '1/2/3/4')).toBeNull();
  });
  it('date and time pictures', () => {
    expect(parseDate('m/d/yy h:MM tt', '3/5/26 2:07 pm')).toEqual(d(2026, 3, 5, 14, 7));
    expect(parseDate('m/d/yy HH:MM', '3/5/26 14:07')).toEqual(d(2026, 3, 5, 14, 7));
  });
  it('formats a typed date in the field picture and keeps an unreadable one', () => {
    const spec = { kind: 'date', format: 'mmm d, yyyy' } as const;
    expect(formatValue(spec, '3/5/2026').text).toBe('Mar 5, 2026');
    expect(formatValue(spec, 'soon').text).toBe('soon');
    expect(formatValue(spec, '').text).toBe('');
  });
  it('an empty picture uses the reader date format', () => {
    const spec = { kind: 'date', format: '' } as const;
    expect(formatValue(spec, '3/5/2026').text).toBe('03/05/2026');
    expect(formatValue(spec, '5/3/2026', () => 'dd.mm.yyyy').text).toBe('05.03.2026');
  });
  it('anything goes while typing; commit checks the date exists', () => {
    const spec = { kind: 'date', format: 'mm/dd/yyyy' } as const;
    expect(acceptKeystroke(spec, '3/').ok).toBe(true);
    expect(commitValue(spec, '3/5/2026').error).toBeNull();
    expect(commitValue(spec, '2/31/2026').error).toContain('mm/dd/yyyy');
  });
});

describe('times (AFTime_*)', () => {
  it('parses 24- and 12-hour times', () => {
    expect(parseTime('13:30', NOW)!.getHours()).toBe(13);
    expect(parseTime('1:30 pm', NOW)!.getHours()).toBe(13);
    expect(parseTime('12:15 am', NOW)!.getHours()).toBe(0);
    expect(parseTime('12 pm', NOW)!.getHours()).toBe(12);
    expect(parseTime('0930', NOW)!.getMinutes()).toBe(30);
    expect(parseTime('25:00', NOW)).toBeNull();
    expect(parseTime('13:00 pm', NOW)).toBeNull();
    expect(parseTime('noon', NOW)).toBeNull();
  });
  it('formats in each AFTime_Format style', () => {
    const f = (format: string, v: string) => formatValue({ kind: 'time', format }, v).text;
    expect(f('HH:MM', '1:05 pm')).toBe('13:05');
    expect(f('h:MM tt', '13:05')).toBe('1:05 pm');
    expect(f('HH:MM:ss', '9:05:07')).toBe('09:05:07');
    expect(f('h:MM:ss tt', '21:00')).toBe('9:00:00 pm');
  });
  it('commit flags an impossible time', () => {
    expect(commitValue({ kind: 'time', format: 'HH:MM' }, '99:99').error).not.toBeNull();
    expect(commitValue({ kind: 'time', format: 'HH:MM' }, '9:15').error).toBeNull();
  });
});

describe('AFRange_Validate', () => {
  it('both bounds', () => {
    const r = { min: 0, max: 1 };
    expect(validateRange(r, '0.5')).toBeNull();
    expect(validateRange(r, '0')).toBeNull();
    expect(validateRange(r, '1')).toBeNull();
    expect(validateRange(r, '2')).toBe(
      'Invalid value: must be greater than or equal to 0 and less than or equal to 1.'
    );
    expect(validateRange(r, '-1')).not.toBeNull();
  });
  it('one bound', () => {
    expect(validateRange({ min: 10, max: null }, '5')).toBe(
      'Invalid value: must be greater than or equal to 10.'
    );
    expect(validateRange({ min: null, max: 10 }, '11')).toBe(
      'Invalid value: must be less than or equal to 10.'
    );
    expect(validateRange({ min: null, max: 10 }, '9')).toBeNull();
  });
  it('skips empty and non-numeric values', () => {
    expect(validateRange({ min: 0, max: 1 }, '')).toBeNull();
    expect(validateRange({ min: 0, max: 1 }, 'x')).toBeNull();
  });
});

describe('fieldProblem', () => {
  const b: FieldBehaviour = {
    keystroke: num(),
    format: num(),
    validate: { min: 0, max: 100 },
  };
  it('reports a format problem before a range one', () => {
    expect(fieldProblem(b, '-')).toContain('does not match');
    expect(fieldProblem(b, '150')).toContain('less than or equal to 100');
    expect(fieldProblem(b, '50')).toBeNull();
    expect(fieldProblem(b, '')).toBeNull();
    expect(fieldProblem(undefined, 'x')).toBeNull();
  });
});

describe('displayValue', () => {
  it('raw without a format, formatted with one', () => {
    expect(displayValue(undefined, '1234')).toEqual({ text: '1234', red: false });
    expect(displayValue({ keystroke: num() }, '1234').text).toBe('1234');
    expect(displayValue({ format: num({ currency: '$' }) }, '1234').text).toBe('$1,234.00');
  });
});

describe('AFSimple_Calculate', () => {
  const values: Record<string, string> = {
    a: '1',
    b: '2.5',
    c: '',
    'g.0': '4',
    'g.1': '6',
    box: 'Off',
  };
  const read = (name: string) =>
    Object.keys(values)
      .filter((k) => k === name || k.startsWith(`${name}.`))
      .map((k) => values[k]!);
  it('sum, product, average, min, max', () => {
    expect(simpleCalculate({ op: 'SUM', fields: ['a', 'b'] }, read)).toBe('3.5');
    expect(simpleCalculate({ op: 'PRD', fields: ['a', 'b', 'g'] }, read)).toBe('60');
    expect(simpleCalculate({ op: 'AVG', fields: ['g'] }, read)).toBe('5');
    expect(simpleCalculate({ op: 'MIN', fields: ['a', 'b'] }, read)).toBe('1');
    expect(simpleCalculate({ op: 'MAX', fields: ['g', 'b'] }, read)).toBe('6');
  });
  it('an empty or non-numeric field counts as 0, as in Acrobat', () => {
    expect(simpleCalculate({ op: 'SUM', fields: ['a', 'c', 'box'] }, read)).toBe('1');
    expect(simpleCalculate({ op: 'AVG', fields: ['a', 'c'] }, read)).toBe('0.5');
    expect(simpleCalculate({ op: 'MIN', fields: ['a', 'c'] }, read)).toBe('0');
  });
  it('a parent name covers its children', () => {
    expect(simpleCalculate({ op: 'SUM', fields: ['g'] }, read)).toBe('10');
  });
  it('drops floating-point noise', () => {
    const r = (n: string) => (n === 'x' ? ['0.1'] : ['0.2']);
    expect(simpleCalculate({ op: 'SUM', fields: ['x', 'y'] }, r)).toBe('0.3');
  });
  it('no fields at all gives 0', () => {
    expect(simpleCalculate({ op: 'SUM', fields: ['missing'] }, read)).toBe('0');
  });
});

describe('recalculate', () => {
  const behaviours: Record<string, FieldBehaviour> = {
    subtotal: { calculate: { op: 'SUM', fields: ['l1', 'l2'] } },
    tax: { calculate: { op: 'PRD', fields: ['subtotal', 'rate'] } },
    total: { calculate: { op: 'SUM', fields: ['subtotal', 'tax'] } },
    other: { calculate: { op: 'SUM', fields: ['z'] } },
  };
  const names = ['l1', 'l2', 'rate', 'z', 'subtotal', 'tax', 'total', 'other'];
  const order = ['subtotal', 'tax', 'total', 'other'];

  it('runs only what depends on the change, in /CO order, each seeing the last', () => {
    const values: Record<string, string> = { l1: '100', l2: '50', rate: '0.1' };
    const out = recalculate(['l1'], behaviours, order, names, (n) => values[n] ?? '');
    expect(out).toEqual({ subtotal: '150', tax: '15', total: '165' });
  });
  it('leaves fields whose value did not move', () => {
    const values: Record<string, string> = {
      l1: '1',
      l2: '2',
      rate: '0',
      subtotal: '3',
      tax: '0',
      total: '3',
    };
    expect(recalculate(['l2'], behaviours, order, names, (n) => values[n] ?? '')).toEqual({});
  });
  it('a change to a rate reaches tax and total but not subtotal', () => {
    expect(calculationsAfter(['rate'], behaviours, order, names)).toEqual(['tax', 'total']);
  });
  it('without /CO, every calculated field in field order', () => {
    expect(calculationsAfter(['l1'], behaviours, [], names)).toEqual(['subtotal', 'tax', 'total']);
  });
  it('an order that lists a consumer first still settles', () => {
    const values: Record<string, string> = { l1: '1', l2: '1', rate: '1' };
    const out = recalculate(
      ['l1'],
      behaviours,
      ['total', 'tax', 'subtotal'],
      names,
      (n) => values[n] ?? ''
    );
    // The form's order is followed as Acrobat follows it: total and tax run
    // first and see the subtotal from before this change.
    expect(out).toEqual({ total: '0', tax: '0', subtotal: '2' });
  });
  it('ignores /CO names with no recognised calculation', () => {
    expect(calculationsAfter(['l1'], behaviours, ['custom', 'subtotal'], names)).toEqual([
      'subtotal',
    ]);
  });
});

/** The same form as the engine's invoice fixture (behaviour_tests.rs): what
 * the engine reads its scripts as, and the values and appearances the save
 * test expects. The two must agree. */
describe('invoice', () => {
  const money = (): FieldFormat => num({ currency: '$' });
  const behaviours: Record<string, FieldBehaviour> = {
    l1: { keystroke: money(), format: money() },
    l2: { keystroke: money(), format: money() },
    l3: { keystroke: money(), format: money() },
    rate: {
      keystroke: { kind: 'percent', decimals: 1, sepStyle: 0, prepend: false },
      format: { kind: 'percent', decimals: 1, sepStyle: 0, prepend: false },
      validate: { min: 0, max: 1 },
    },
    subtotal: { format: money(), calculate: { op: 'SUM', fields: ['l1', 'l2', 'l3'] } },
    tax: {
      format: num({ negStyle: 2 }),
      calculate: { op: 'PRD', fields: ['subtotal', 'rate'] },
    },
    total: { format: money(), calculate: { op: 'SUM', fields: ['subtotal', 'tax'] } },
    avg: {
      format: num({ decimals: 1, sepStyle: 2, currency: ' EUR', prepend: false }),
      calculate: { op: 'AVG', fields: ['l1', 'l2', 'l3'] },
    },
    maxv: {
      format: num({ decimals: 0, sepStyle: 1 }),
      calculate: { op: 'MAX', fields: ['l1', 'l2', 'l3'] },
    },
    net: { keystroke: num({ negStyle: 1 }), format: num({ negStyle: 1 }) },
    when: {
      keystroke: { kind: 'date', format: 'mmm d, yyyy' },
      format: { kind: 'date', format: 'mmm d, yyyy' },
    },
    ssn: { keystroke: { kind: 'special', style: 3 }, format: { kind: 'special', style: 3 } },
    phone: { keystroke: { kind: 'special', style: 2 }, format: { kind: 'special', style: 2 } },
    custom: { format: money() },
  };
  const calcOrder = ['subtotal', 'tax', 'total', 'avg', 'maxv', 'custom'];
  const names = [
    'l1',
    'l2',
    'l3',
    'rate',
    'subtotal',
    'tax',
    'total',
    'avg',
    'maxv',
    'net',
    'when',
    'ssn',
    'phone',
    'custom',
    'note',
  ];
  const typed: Record<string, string> = {
    l1: '100',
    l2: '250.7',
    l3: '1000',
    rate: '0.08',
    net: '-42.5',
    when: '3/5/2026',
    ssn: '123456789',
    phone: '5551234567',
    note: 'Paid',
  };

  it('calculates the hand-computed totals in /CO order', () => {
    const results = recalculate(
      Object.keys(typed),
      behaviours,
      calcOrder,
      names,
      (n) => typed[n] ?? ''
    );
    expect(results).toEqual({
      subtotal: '1350.7',
      tax: '108.056',
      total: '1458.756',
      avg: '450.233333333333',
      maxv: '1000',
    });
  });

  it('formats every field the way the saved appearance draws it', () => {
    const values = {
      ...typed,
      subtotal: '1350.7',
      tax: '108.056',
      total: '1458.756',
      avg: '450.233333333333',
      maxv: '1000',
    };
    const fields = names.map((name) => ({ name, kind: 'text', comb: false, maxLen: 0 }));
    expect(appearanceEntries(behaviours, fields, Object.entries(values))).toEqual([
      ['l1', '$100.00', null],
      ['l2', '$250.70', null],
      ['l3', '$1,000.00', null],
      ['rate', '8.0%', null],
      ['net', '42.50', true],
      ['when', 'Mar 5, 2026', null],
      ['ssn', '123-45-6789', null],
      ['phone', '(555) 123-4567', null],
      ['subtotal', '$1,350.70', null],
      ['tax', '108.06', null],
      ['total', '$1,458.76', null],
      ['avg', '450,2 EUR', null],
    ]);
  });

  it('leaves a comb field, or one too short for the formatting, raw', () => {
    const fields = [
      { name: 'ssn', kind: 'text', comb: true, maxLen: 9 },
      { name: 'l1', kind: 'text', comb: false, maxLen: 5 },
    ];
    expect(
      appearanceEntries(behaviours, fields, [
        ['ssn', '123456789'],
        ['l1', '100'],
      ])
    ).toEqual([]);
  });

  it('a red style sends black for a positive value', () => {
    const fields = [{ name: 'net', kind: 'text', comb: false, maxLen: 0 }];
    expect(appearanceEntries(behaviours, fields, [['net', '5']])).toEqual([['net', '5.00', false]]);
  });
});
