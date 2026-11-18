/** The standard Acrobat form helpers, done here instead of in JavaScript.
 *
 * A form's field scripts are never run. The engine recognises the calls
 * Acrobat itself writes when an author picks a format, a range or a simple
 * calculation in the field properties dialog (AFNumber_Format and friends,
 * from Acrobat's AForm.js) and hands them over as data; this module does
 * what each of those helpers does. It follows AForm.js closely - the same
 * entry and commit patterns, separator and negative styles, date pictures
 * and quirks (an empty field counts as 0 in a calculation) - so a value
 * typed here looks the way it would in Acrobat.
 *
 * Where Acrobat would refuse a committed value with an alert, nothing is
 * refused: the value is kept and `fieldProblem` says what is wrong with it,
 * for the page outline and the panel.
 *
 * Everything here is pure: no stores, no DOM.
 */
import type {
  FieldBehaviour,
  FieldCalc,
  FieldFormat,
  FieldRange,
  FormFieldInfo,
} from '../../types/engine';

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

/** Thousands and decimal separators for sepStyle 0-4. */
const SEPARATORS: readonly [string, string][] = [
  [',', '.'],
  ['', '.'],
  ['.', ','],
  ['', ','],
  ["'", '.'],
];

const NUMBER = /^[+-]?(\d+([.,]\d*)?|[.,]\d+)$/;
const GROUPED = /^[+-]?\d{1,3}(,\d{3})+(\.\d*)?$/;

/** AFMakeNumber: the number a field value stands for, or null. Accepts a
 * comma as the decimal separator, as Acrobat does, and a value already
 * grouped in thousands. */
export function makeNumber(value: string): number | null {
  const s = value.trim();
  let n: number;
  if (NUMBER.test(s)) n = Number(s.replace(',', '.'));
  else if (GROUPED.test(s)) n = Number(s.replace(/,/g, ''));
  else return null;
  return Number.isFinite(n) ? n : null;
}

/** util.printf("%,<sepStyle>.<decimals>f", value). */
export function printNumber(value: number, decimals: number, sepStyle: number): string {
  const [group, point] = SEPARATORS[Math.min(4, Math.max(0, Math.floor(sepStyle)))]!;
  const negative = value < 0;
  const fixed = Math.abs(value).toFixed(Math.min(20, Math.max(0, Math.floor(decimals))));
  const [int = '0', frac] = fixed.split('.');
  const grouped = group ? int.replace(/\B(?=(\d{3})+(?!\d))/g, group) : int;
  return `${negative ? '-' : ''}${grouped}${frac !== undefined ? point + frac : ''}`;
}

/** What a field shows: its text, and whether Acrobat would draw it red. */
export interface Display {
  text: string;
  red: boolean;
}

/** AFNumber_Format. */
export function formatNumber(spec: Extract<FieldFormat, { kind: 'number' }>, raw: string): Display {
  const value = makeNumber(raw);
  if (value === null) return { text: '', red: false };
  const negative = value < 0;
  const parens = (spec.negStyle === 2 || spec.negStyle === 3) && negative;
  let shown = value;
  if (spec.negStyle !== 0 || spec.prepend) shown = Math.abs(value);
  let text = printNumber(shown, spec.decimals, spec.sepStyle);
  if (spec.prepend) text = spec.currency + text;
  else text += spec.currency;
  if (parens) text = `(${text})`;
  if (negative && spec.prepend && spec.negStyle === 0) text = `-${text}`;
  return { text, red: negative && (spec.negStyle === 1 || spec.negStyle === 3) };
}

/** AFPercent_Format. */
export function formatPercent(
  spec: Extract<FieldFormat, { kind: 'percent' }>,
  raw: string
): Display {
  const value = makeNumber(raw);
  if (value === null) return { text: '', red: false };
  const text = printNumber(value * 100, spec.decimals, spec.sepStyle);
  return { text: spec.prepend ? `%${text}` : `${text}%`, red: false };
}

const DOT_ENTRY = /^[+-]?\d*\.?\d*$/;
const DOT_COMMIT = [/^[+-]?\d+(\.\d+)?$/, /^[+-]?\.\d+$/, /^[+-]?\d+\.$/];
const COMMA_ENTRY = /^[+-]?\d*,?\d*$/;
const COMMA_COMMIT = [/^[+-]?\d+([.,]\d+)?$/, /^[+-]?[.,]\d+$/, /^[+-]?\d+[.,]$/];

// ---------------------------------------------------------------------------
// Special formats and masks
// ---------------------------------------------------------------------------

const SEP = '(\\.|[- ])?';
const ZIP_ENTRY = [/^\d{0,5}$/];
const ZIP_COMMIT = [/^\d{5}$/];
const ZIP4_ENTRY = [new RegExp(`^\\d{0,5}${SEP}\\d{0,4}$`)];
const ZIP4_COMMIT = [new RegExp(`^\\d{5}${SEP}\\d{4}$`)];
const PHONE_ENTRY = [
  new RegExp(`^\\d{0,3}${SEP}\\d{0,3}${SEP}\\d{0,4}$`),
  /^\(\d{0,3}\)?$/,
  new RegExp(`^\\(\\d{0,3}\\)?\\s?\\d{0,3}${SEP}\\d{0,4}$`),
  new RegExp(`^\\d{0,3}${SEP}\\d{0,4}$`),
  /^011(\.|[- \d])*$/,
];
const PHONE_COMMIT = [
  new RegExp(`^\\d{3}${SEP}\\d{4}$`),
  new RegExp(`^\\d{3}${SEP}\\d{3}${SEP}\\d{4}$`),
  new RegExp(`^\\(\\d{3}\\)\\s?\\d{3}${SEP}\\d{4}$`),
  /^011(\.|[- \d])*$/,
];
const SSN_ENTRY = [new RegExp(`^\\d{0,3}${SEP}\\d{0,2}${SEP}\\d{0,4}$`)];
const SSN_COMMIT = [new RegExp(`^\\d{3}${SEP}\\d{2}${SEP}\\d{4}$`)];

const SPECIAL: readonly [RegExp[], RegExp[]][] = [
  [ZIP_ENTRY, ZIP_COMMIT],
  [ZIP4_ENTRY, ZIP4_COMMIT],
  [PHONE_ENTRY, PHONE_COMMIT],
  [SSN_ENTRY, SSN_COMMIT],
];

const isDigit = (c: string) => c >= '0' && c <= '9';
const isLetter = (c: string) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');

/** util.printx: lay `source` into the picture `format`. 9 takes the next
 * digit, A the next letter, X the next letter or digit (skipping anything
 * else), ? any character, * the rest; > < = switch case; \\ escapes. */
export function printx(format: string, source: string): string {
  const out: string[] = [];
  let i = 0;
  let mode: 'keep' | 'upper' | 'lower' = 'keep';
  const cased = (c: string) =>
    mode === 'upper' ? c.toUpperCase() : mode === 'lower' ? c.toLowerCase() : c;
  const take = (ok: (c: string) => boolean) => {
    while (i < source.length) {
      const c = source.charAt(i++);
      if (ok(c)) {
        out.push(cased(c));
        return;
      }
    }
  };
  let escaped = false;
  for (const command of format) {
    if (escaped) {
      out.push(command);
      escaped = false;
      continue;
    }
    if (i >= source.length) break;
    switch (command) {
      case '?':
        out.push(cased(source.charAt(i++)));
        break;
      case 'X':
        take((c) => isDigit(c) || isLetter(c));
        break;
      case 'A':
        take(isLetter);
        break;
      case '9':
        take(isDigit);
        break;
      case '*':
        while (i < source.length) out.push(cased(source.charAt(i++)));
        break;
      case '\\':
        escaped = true;
        break;
      case '>':
        mode = 'upper';
        break;
      case '<':
        mode = 'lower';
        break;
      case '=':
        mode = 'keep';
        break;
      default:
        out.push(command);
    }
  }
  return out.join('');
}

/** AFSpecial_Format. */
export function formatSpecial(style: number, raw: string): string {
  if (!raw) return raw;
  switch (style) {
    case 0:
      return printx('99999', raw);
    case 1:
      return printx('99999-9999', raw);
    case 2:
      return printx(printx('9999999999', raw).length >= 10 ? '(999) 999-9999' : '999-9999', raw);
    case 3:
      return printx('999-99-9999', raw);
    default:
      return raw;
  }
}

const MASK_TESTS: Record<string, (c: string) => boolean> = {
  '9': isDigit,
  A: isLetter,
  O: (c) => isDigit(c) || isLetter(c),
  X: () => true,
};

/** Lay what the user typed into an arbitrary mask (AFSpecial_KeystrokeEx),
 * putting the mask's own characters - the dashes and brackets - where they
 * go, so they need not be typed. Null when a character does not fit. */
export function fitMask(mask: string, value: string): string | null {
  const literals = new Set([...mask].filter((c) => !MASK_TESTS[c]));
  // Typed or inserted literals are dropped and laid out again, so deleting
  // inside the value never strands one in the wrong cell.
  const chars = [...value].filter((c) => !literals.has(c));
  let out = '';
  let at = 0;
  for (const c of chars) {
    while (at < mask.length && !MASK_TESTS[mask[at]!]) out += mask[at++];
    const test = MASK_TESTS[mask[at] ?? ''];
    if (!test || !test(c)) return null;
    out += c;
    at += 1;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

const MONTHS = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];
const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

/** The tokens of a util.printd picture, longest first. */
const DATE_TOKENS = [
  'mmmm',
  'mmm',
  'mm',
  'm',
  'dddd',
  'ddd',
  'dd',
  'd',
  'yyyy',
  'yy',
  'HH',
  'H',
  'hh',
  'h',
  'MM',
  'M',
  'ss',
  's',
  'tt',
  't',
];

type Piece = { token: string } | { text: string };

function pieces(format: string): Piece[] {
  const out: Piece[] = [];
  let i = 0;
  while (i < format.length) {
    if (format[i] === '\\' && i + 1 < format.length) {
      out.push({ text: format[i + 1]! });
      i += 2;
      continue;
    }
    const token = DATE_TOKENS.find((t) => format.startsWith(t, i));
    if (token) {
      out.push({ token });
      i += token.length;
    } else {
      out.push({ text: format[i]! });
      i += 1;
    }
  }
  return out;
}

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

/** util.printd: `date` in the picture `format`. */
export function printd(format: string, date: Date): string {
  const hours = date.getHours();
  const h12 = hours % 12 === 0 ? 12 : hours % 12;
  return pieces(format)
    .map((p) => {
      if ('text' in p) return p.text;
      switch (p.token) {
        case 'mmmm':
          return MONTHS[date.getMonth()]!;
        case 'mmm':
          return MONTHS[date.getMonth()]!.slice(0, 3);
        case 'mm':
          return pad(date.getMonth() + 1);
        case 'm':
          return String(date.getMonth() + 1);
        case 'dddd':
          return DAYS[date.getDay()]!;
        case 'ddd':
          return DAYS[date.getDay()]!.slice(0, 3);
        case 'dd':
          return pad(date.getDate());
        case 'd':
          return String(date.getDate());
        case 'yyyy':
          return pad(date.getFullYear(), 4);
        case 'yy':
          return pad(date.getFullYear() % 100);
        case 'HH':
          return pad(hours);
        case 'H':
          return String(hours);
        case 'hh':
          return pad(h12);
        case 'h':
          return String(h12);
        case 'MM':
          return pad(date.getMinutes());
        case 'M':
          return String(date.getMinutes());
        case 'ss':
          return pad(date.getSeconds());
        case 's':
          return String(date.getSeconds());
        case 'tt':
          return hours < 12 ? 'am' : 'pm';
        case 't':
          return hours < 12 ? 'a' : 'p';
        default:
          return '';
      }
    })
    .join('');
}

interface Parts {
  year?: number;
  month?: number;
  day?: number;
  hour?: number;
  minute?: number;
  second?: number;
  pm?: boolean;
}

/** A two-digit year, the way Acrobat reads one: below 50 is this century. */
function fullYear(n: number, digits: number): number {
  if (digits > 2) return n;
  return n < 50 ? 2000 + n : 1900 + n;
}

function monthFromWord(word: string): number | undefined {
  if (word.length < 3) return undefined;
  const at = MONTHS.findIndex((m) => m.toLowerCase().startsWith(word.toLowerCase().slice(0, 3)));
  return at >= 0 ? at + 1 : undefined;
}

/** Build a date from parts, or null when they do not name a real one. */
function build(parts: Parts, now: Date): Date | null {
  const year = parts.year ?? now.getFullYear();
  const month = parts.month ?? (parts.year === undefined ? now.getMonth() + 1 : 1);
  const day = parts.day ?? 1;
  let hour = parts.hour ?? 0;
  const minute = parts.minute ?? 0;
  const second = parts.second ?? 0;
  if (parts.pm !== undefined) {
    if (hour < 1 || hour > 12) return null;
    hour = (hour % 12) + (parts.pm ? 12 : 0);
  }
  if (month < 1 || month > 12 || day < 1 || hour > 23 || minute > 59 || second > 59) return null;
  const date = new Date(year, month - 1, day, hour, minute, second);
  // Rejects 31 April and 29 February outside leap years, which Date would
  // otherwise roll into the next month.
  if (date.getMonth() !== month - 1 || date.getDate() !== day) return null;
  date.setFullYear(year);
  return date;
}

/** Read a value exactly in the picture `format` (util.scand). */
function scanExact(format: string, value: string, now: Date): Date | null {
  const parts: Parts = {};
  let i = 0;
  const digits = (min: number, max: number): number | null => {
    let j = i;
    while (j < value.length && j - i < max && isDigit(value[j]!)) j += 1;
    if (j - i < min) return null;
    const n = Number(value.slice(i, j));
    i = j;
    return n;
  };
  for (const p of pieces(format)) {
    if ('text' in p) {
      if (value[i] === p.text) i += 1;
      else if (/\s/.test(p.text)) continue;
      else return null;
      continue;
    }
    const t = p.token;
    let n: number | null = null;
    if (t === 'mmmm' || t === 'mmm' || t === 'dddd' || t === 'ddd') {
      const word = /^[A-Za-z]+/.exec(value.slice(i))?.[0];
      if (!word) return null;
      i += word.length;
      if (t.startsWith('m')) {
        const month = monthFromWord(word);
        if (month === undefined) return null;
        parts.month = month;
      }
      continue;
    }
    if (t === 'tt' || t === 't') {
      const word = /^[AaPp][Mm]?/.exec(value.slice(i))?.[0];
      if (!word) return null;
      i += word.length;
      parts.pm = word[0]!.toLowerCase() === 'p';
      continue;
    }
    if (t === 'yyyy') {
      const start = i;
      n = digits(2, 4);
      if (n !== null) n = fullYear(n, i - start);
    } else if (t === 'yy') {
      n = digits(2, 2);
      if (n !== null) n = fullYear(n, 2);
    } else {
      n = digits(1, 2);
    }
    if (n === null) return null;
    if (t[0] === 'y') parts.year = n;
    else if (t[0] === 'm') parts.month = n;
    else if (t[0] === 'd') parts.day = n;
    else if (t[0] === 'H') parts.hour = n;
    else if (t[0] === 'h') parts.hour = n;
    else if (t[0] === 'M') parts.minute = n;
    else if (t[0] === 's') parts.second = n;
  }
  if (i !== value.length) return null;
  return build(parts, now);
}

/** Order of the year, month and day fields in a picture. */
function dateOrder(format: string): ('y' | 'm' | 'd')[] {
  const order: { key: 'y' | 'm' | 'd'; at: number }[] = [];
  const tokens = pieces(format);
  tokens.forEach((p, at) => {
    if (!('token' in p)) return;
    const k = p.token[0];
    if (k !== 'y' && k !== 'm' && !(k === 'd' && p.token.length <= 2)) return;
    if (!order.some((o) => o.key === k)) order.push({ key: k as 'y' | 'm' | 'd', at });
  });
  return order.map((o) => o.key);
}

function digitWidth(format: string, key: 'y' | 'm' | 'd'): number {
  const p = pieces(format).find((x) => 'token' in x && x.token[0] === key && x.token.length <= 4);
  if (!p || !('token' in p)) return 0;
  return key === 'y' ? p.token.length : 2;
}

/** AFParseDateEx: read a date (and time) the way Acrobat does when the
 * value does not match its picture exactly - numbers taken in the order the
 * picture puts them, a month name wherever it appears, am or pm, and a run
 * of digits split by the picture's field widths. */
export function parseDate(format: string, value: string, now: Date = new Date()): Date | null {
  const s = value.trim();
  if (!s) return null;
  const exact = scanExact(format, s, now);
  if (exact) return exact;

  const numbers = s.match(/\d+/g) ?? [];
  const words = s.match(/[A-Za-z]+/g) ?? [];
  if (numbers.length === 0) return null;
  const parts: Parts = {};
  for (const w of words) {
    const lower = w.toLowerCase();
    if (lower === 'am' || lower === 'a') parts.pm = false;
    else if (lower === 'pm' || lower === 'p') parts.pm = true;
    else if (parts.month === undefined) {
      const month = monthFromWord(w);
      if (month !== undefined) parts.month = month;
    }
  }
  let order = dateOrder(format);
  if (parts.month !== undefined) order = order.filter((k) => k !== 'm');

  let rest = [...numbers];
  // One run of digits, as in 01022020: split it by the picture's widths.
  if (rest.length === 1 && order.length > 1 && rest[0]!.length > 2) {
    const run = rest[0]!;
    const widths = order.map((k) => digitWidth(format, k));
    const yearAt = order.indexOf('y');
    if (yearAt >= 0 && widths.reduce((a, b) => a + b, 0) !== run.length) {
      widths[yearAt] = run.length - widths.reduce((a, b, i) => (i === yearAt ? a : a + b), 0);
    }
    if (widths.some((w) => w <= 0) || widths.reduce((a, b) => a + b, 0) !== run.length) return null;
    let at = 0;
    rest = widths.map((w) => {
      const piece = run.slice(at, at + w);
      at += w;
      return piece;
    });
  }

  for (const key of order) {
    const piece = rest.shift();
    if (piece === undefined) break;
    const n = Number(piece);
    if (key === 'y') parts.year = fullYear(n, piece.length);
    else if (key === 'm') parts.month = n;
    else parts.day = n;
  }
  const hasTime = /[HhMs]/.test(format.replace(/\\./g, ''));
  if (hasTime) {
    const [h, m, sec] = rest;
    if (h !== undefined) parts.hour = Number(h);
    if (m !== undefined) parts.minute = Number(m);
    if (sec !== undefined) parts.second = Number(sec);
  } else if (rest.length > 0) {
    return null;
  }
  if (parts.pm !== undefined && parts.hour === undefined) return null;
  return build(parts, now);
}

/** AFParseTime: a time of day, today. Takes 13:30, 1:30 pm, 1 pm, 0930. */
export function parseTime(value: string, now: Date = new Date()): Date | null {
  const s = value.trim();
  if (!s) return null;
  const numbers = s.match(/\d+/g) ?? [];
  if (numbers.length === 0 || numbers.length > 3) return null;
  const sec = numbers[2];
  let [h, m] = numbers;
  if (numbers.length === 1 && h!.length > 2) {
    // 0930 or 930
    m = h!.slice(-2);
    h = h!.slice(0, -2);
  }
  const ampm = /\b([ap])\.?m?\.?\s*$/i.exec(s) ?? /\d\s*([ap])m?\b/i.exec(s);
  const parts: Parts = {
    year: now.getFullYear(),
    month: now.getMonth() + 1,
    day: now.getDate(),
    hour: Number(h),
    minute: m === undefined ? 0 : Number(m),
    second: sec === undefined ? 0 : Number(sec),
  };
  if (ampm) parts.pm = ampm[1]!.toLowerCase() === 'p';
  if (/[^\d\s:.apmAPM]/.test(s)) return null;
  return build(parts, now);
}

// ---------------------------------------------------------------------------
// Keystroke, commit, format, validate
// ---------------------------------------------------------------------------

/** The value to show for a date picture left empty: the reader's own. */
export type DateFormatSource = () => string;

function datePicture(spec: { format: string }, fallback: DateFormatSource): string {
  return spec.format || fallback();
}

const INVALID = 'The value entered does not match the format of the field.';

/** Whether a field takes `proposed` while it is being typed, and what it
 * becomes (a mask puts its own characters in). Acrobat beeps and drops the
 * keystroke when this fails. */
export function acceptKeystroke(
  spec: FieldFormat,
  proposed: string
): { ok: boolean; value: string } {
  if (proposed === '') return { ok: true, value: '' };
  switch (spec.kind) {
    case 'number':
    case 'percent': {
      const entry = spec.sepStyle > 1 ? COMMA_ENTRY : DOT_ENTRY;
      return { ok: entry.test(proposed.trim()), value: proposed };
    }
    case 'special': {
      const entry = SPECIAL[spec.style]?.[0];
      return { ok: !entry || entry.some((r) => r.test(proposed)), value: proposed };
    }
    case 'mask': {
      if (proposed.length > spec.mask.length + 8) return { ok: false, value: proposed };
      const fitted = fitMask(spec.mask, proposed);
      return fitted === null || fitted.length > spec.mask.length
        ? { ok: false, value: proposed }
        : { ok: true, value: fitted };
    }
    // Dates and times take anything while typing and are checked on commit.
    case 'date':
    case 'time':
      return { ok: true, value: proposed };
  }
}

/** The committed value (Acrobat normalises a comma decimal to a point) and
 * what is wrong with it, if anything: Acrobat's alert, as a message. */
export function commitValue(
  spec: FieldFormat,
  value: string,
  dateFormat: DateFormatSource = defaultDatePicture
): { value: string; error: string | null } {
  if (value === '') return { value, error: null };
  switch (spec.kind) {
    case 'number':
    case 'percent': {
      const trimmed = value.trim();
      const commit = spec.sepStyle > 1 ? COMMA_COMMIT : DOT_COMMIT;
      if (!commit.some((r) => r.test(trimmed))) return { value, error: INVALID };
      if (spec.sepStyle > 1) {
        const n = Number(trimmed.replace(',', '.'));
        return { value: Number.isFinite(n) ? String(n) : value, error: null };
      }
      return { value, error: null };
    }
    case 'special': {
      const commit = SPECIAL[spec.style]?.[1];
      return { value, error: !commit || commit.some((r) => r.test(value)) ? null : INVALID };
    }
    case 'mask': {
      const fitted = fitMask(spec.mask, value);
      return fitted !== null && fitted.length === spec.mask.length
        ? { value: fitted, error: null }
        : { value, error: `Invalid value: must match format "${spec.mask}".` };
    }
    case 'date': {
      const picture = datePicture(spec, dateFormat);
      return parseDate(picture, value)
        ? { value, error: null }
        : {
            value,
            error: `Invalid date/time: please ensure that the date/time exists. The field should match format ${picture}.`,
          };
    }
    case 'time':
      return parseTime(value) || parseDate(spec.format, value)
        ? { value, error: null }
        : {
            value,
            error: `Invalid date/time: please ensure that the date/time exists. The field should match format ${spec.format}.`,
          };
  }
}

/** The text a field shows for `raw` when it is not being edited. */
export function formatValue(
  spec: FieldFormat,
  raw: string,
  dateFormat: DateFormatSource = defaultDatePicture,
  now: Date = new Date()
): Display {
  const plain = { text: raw, red: false };
  switch (spec.kind) {
    case 'number':
      return formatNumber(spec, raw);
    case 'percent':
      return formatPercent(spec, raw);
    case 'special':
      return { text: formatSpecial(spec.style, raw), red: false };
    case 'mask':
      return plain;
    case 'date': {
      if (!raw) return plain;
      const picture = datePicture(spec, dateFormat);
      const date = parseDate(picture, raw, now);
      // Acrobat empties a value it cannot read as a date; it is kept here,
      // and flagged, rather than hidden.
      return date ? { text: printd(picture, date), red: false } : plain;
    }
    case 'time': {
      if (!raw) return plain;
      const date = parseTime(raw, now) ?? parseDate(spec.format, raw, now);
      return date ? { text: printd(spec.format, date), red: false } : plain;
    }
  }
}

/** AFRange_Validate: Acrobat's message, or null when the value is in range.
 * An empty value, or one that is not a number, is not checked. */
export function validateRange(range: FieldRange, raw: string): string | null {
  if (raw.trim() === '') return null;
  const n = Number(raw.trim().replace(',', '.'));
  if (Number.isNaN(n)) return null;
  const { min, max } = range;
  if (min !== null && max !== null) {
    return n < min || n > max
      ? `Invalid value: must be greater than or equal to ${min} and less than or equal to ${max}.`
      : null;
  }
  if (min !== null && n < min) return `Invalid value: must be greater than or equal to ${min}.`;
  if (max !== null && n > max) return `Invalid value: must be less than or equal to ${max}.`;
  return null;
}

/** What is wrong with a field's committed value, or null. */
export function fieldProblem(
  behaviour: FieldBehaviour | undefined,
  raw: string,
  dateFormat: DateFormatSource = defaultDatePicture
): string | null {
  if (!behaviour || raw === '') return null;
  if (behaviour.keystroke) {
    const error = commitValue(behaviour.keystroke, raw, dateFormat).error;
    if (error) return error;
  }
  return behaviour.validate ? validateRange(behaviour.validate, raw) : null;
}

/** The text for a field when it is not focused. */
export function displayValue(
  behaviour: FieldBehaviour | undefined,
  raw: string,
  dateFormat: DateFormatSource = defaultDatePicture,
  now?: Date
): Display {
  if (!behaviour?.format) return { text: raw, red: false };
  return formatValue(behaviour.format, raw, dateFormat, now);
}

/** US month/day/year: what a date field with no picture of its own shows
 * until the reader picks another. */
export function defaultDatePicture(): string {
  return 'mm/dd/yyyy';
}

// ---------------------------------------------------------------------------
// Calculation
// ---------------------------------------------------------------------------

/** AFSimple_Calculate over the values of the fields it names. Each name
 * covers the field itself and every field below it (getArray), and an
 * empty or non-numeric value counts as 0, as in Acrobat. */
export function simpleCalculate(calc: FieldCalc, values: (name: string) => string[]): string {
  const numbers: number[] = [];
  for (const name of calc.fields) {
    for (const v of values(name)) numbers.push(makeNumber(v) ?? 0);
  }
  let result: number;
  if (numbers.length === 0) result = 0;
  else {
    switch (calc.op.toUpperCase()) {
      case 'SUM':
        result = numbers.reduce((a, b) => a + b, 0);
        break;
      case 'PRD':
        result = numbers.reduce((a, b) => a * b, 1);
        break;
      case 'AVG':
        result = numbers.reduce((a, b) => a + b, 0) / numbers.length;
        break;
      case 'MIN':
        result = Math.min(...numbers);
        break;
      case 'MAX':
        result = Math.max(...numbers);
        break;
      default:
        return '';
    }
  }
  // Drop binary noise (0.1 + 0.2) that no form means to show.
  const clean = Number(result.toPrecision(15));
  return String(Object.is(clean, -0) ? 0 : clean);
}

/** The fields `calc` reads, as names that cover them. */
function covers(name: string, field: string): boolean {
  return field === name || field.startsWith(`${name}.`);
}

/** The calculated fields to run, in order, after `changed` changed: those
 * whose inputs include it, directly or through another calculation. With
 * no /CO, every calculated field in `fallbackOrder`. */
export function calculationsAfter(
  changed: string[],
  behaviours: Record<string, FieldBehaviour>,
  calcOrder: string[],
  fallbackOrder: string[]
): string[] {
  const order = (calcOrder.length > 0 ? calcOrder : fallbackOrder).filter(
    (name, i, all) => behaviours[name]?.calculate && all.indexOf(name) === i
  );
  const dirty = new Set(changed);
  const out: string[] = [];
  // Settle: a calculation late in the order can feed one earlier. Acrobat
  // runs the order once per change; a second pass is what a second change
  // would have done, and the order list is short.
  for (let pass = 0; pass < 2; pass += 1) {
    for (const name of order) {
      if (out.includes(name)) continue;
      const calc = behaviours[name]!.calculate!;
      if ([...dirty].some((d) => calc.fields.some((f) => covers(f, d)))) {
        out.push(name);
        dirty.add(name);
      }
    }
  }
  return order.filter((name) => out.includes(name));
}

/** Run the calculations that follow from `changed`, in order, each seeing
 * the results before it. Returns the new raw values of the fields whose
 * value moved. */
export function recalculate(
  changed: string[],
  behaviours: Record<string, FieldBehaviour>,
  calcOrder: string[],
  fieldNames: string[],
  valueOf: (name: string) => string
): Record<string, string> {
  const current = new Map<string, string>();
  const read = (name: string) => current.get(name) ?? valueOf(name);
  const results: Record<string, string> = {};
  for (const name of calculationsAfter(changed, behaviours, calcOrder, fieldNames)) {
    const calc = behaviours[name]!.calculate!;
    const value = simpleCalculate(calc, (n) => fieldNames.filter((f) => covers(n, f)).map(read));
    if (value !== read(name)) results[name] = value;
    current.set(name, value);
  }
  return results;
}

// ---------------------------------------------------------------------------
// Saved appearance
// ---------------------------------------------------------------------------

/** For the save: the text fields among `texts` whose page appearance shows
 * something other than their raw value, as [name, formatted text, colour].
 * The colour is true for red and false for black where the format has a
 * red negative style (Acrobat sets the field's text colour either way), and
 * null where the format leaves colour alone.
 *
 * A comb field, or formatted text longer than the field's /MaxLen, is left
 * raw: the field could not hold it. */
export function appearanceEntries(
  behaviours: Record<string, FieldBehaviour>,
  fields: Pick<FormFieldInfo, 'name' | 'kind' | 'comb' | 'maxLen'>[],
  texts: [string, string][],
  dateFormat: DateFormatSource = defaultDatePicture
): [string, string, boolean | null][] {
  const out: [string, string, boolean | null][] = [];
  for (const [name, raw] of texts) {
    const format = behaviours[name]?.format;
    if (!format) continue;
    const field = fields.find((f) => f.name === name && f.kind === 'text');
    if (!field || field.comb) continue;
    const shown = displayValue(behaviours[name], raw, dateFormat);
    if (field.maxLen > 0 && [...shown.text].length > field.maxLen) continue;
    const colored = format.kind === 'number' && (format.negStyle === 1 || format.negStyle === 3);
    if (shown.text !== raw || colored) out.push([name, shown.text, colored ? shown.red : null]);
  }
  return out;
}
