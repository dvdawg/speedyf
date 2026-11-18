/** How a field's value is laid out on the page: the font size an auto-sized
 * field settles on, and the spacing that puts one character in each cell of
 * a comb.
 *
 * The overlay draws an edited field itself until the file is saved, at which
 * point PDFium writes the real appearance. The two should look alike, so the
 * auto-size here follows PDFium's (`CPVT_VariableText::GetAutoFontSize`): the
 * largest of a fixed ladder of sizes at which the text still fits, with
 * multiline fields capped at 12pt. Everything is in PDF points; the caller
 * multiplies by the zoom.
 */
import type { FormFieldInfo } from '../../types/engine';

/** PDFium's ladder of auto font sizes. Multiline fields use the first six. */
const SIZE_STEPS = [
  4, 6, 8, 9, 10, 12, 14, 18, 20, 25, 30, 35, 40, 45, 50, 55, 60, 70, 80, 90, 100, 110, 120, 130,
  144,
];
const MULTILINE_STEPS = 6;

/** Line height as a multiple of the font size, for Helvetica. */
export const LINE_HEIGHT = 1.15;

/** Inset of the text from the widget's edge: a 1pt border plus 1pt. */
export const FIELD_PAD_PT = 2;

/** Average Helvetica advance per em, for when no canvas can measure. */
const FALLBACK_ADVANCE = 0.52;

/** Courier's advance per em: the comb overlay's font is monospaced so that
 * spacing can put each character in the middle of its own cell. */
export const MONO_ADVANCE = 0.6;

export const FIELD_FONT = "Helvetica, Arial, 'Liberation Sans', sans-serif";
export const COMB_FONT = "Courier, 'Courier New', 'Liberation Mono', monospace";

let context: CanvasRenderingContext2D | null | undefined;

/** Width of `text` set at `sizePt` in the field font, in points. */
export function measureWidth(text: string, sizePt: number): number {
  if (context === undefined) {
    try {
      context =
        typeof document !== 'undefined' ? document.createElement('canvas').getContext('2d') : null;
    } catch {
      context = null;
    }
  }
  if (context) {
    context.font = `${sizePt}px ${FIELD_FONT}`;
    const width = context.measureText(text).width;
    if (Number.isFinite(width)) return width;
  }
  return text.length * sizePt * FALLBACK_ADVANCE;
}

type Measure = (text: string, sizePt: number) => number;

/** Lines `text` wraps to at `sizePt` in a box `width` points wide. */
export function wrappedLines(
  text: string,
  sizePt: number,
  width: number,
  measure: Measure = measureWidth
): number {
  let lines = 0;
  for (const paragraph of text.split('\n')) {
    let line = '';
    let count = 1;
    for (const word of paragraph.split(' ')) {
      const next = line ? `${line} ${word}` : word;
      if (line && measure(next, sizePt) > width) {
        count += 1;
        line = word;
      } else {
        line = next;
      }
      // A single word wider than the box breaks inside itself.
      while (measure(line, sizePt) > width && line.length > 1) {
        const fit = Math.max(1, Math.floor((line.length * width) / measure(line, sizePt)));
        line = line.slice(fit);
        count += 1;
      }
    }
    lines += count;
  }
  return Math.max(1, lines);
}

/** The font size, in points, an auto-sized (0 Tf) field shows `text` at. */
export function autoFontSize(
  text: string,
  field: Pick<FormFieldInfo, 'rect' | 'multiline'>,
  measure: Measure = measureWidth
): number {
  const width = Math.max(1, field.rect.w - 2 * FIELD_PAD_PT);
  const height = Math.max(1, field.rect.h - 2 * FIELD_PAD_PT);
  const steps = field.multiline ? SIZE_STEPS.slice(0, MULTILINE_STEPS) : SIZE_STEPS;
  let best = steps[0]!;
  for (const size of steps) {
    const fits = field.multiline
      ? wrappedLines(text, size, width, measure) * size * LINE_HEIGHT <= height
      : size * LINE_HEIGHT <= height && measure(text, size) <= width;
    if (!fits) break;
    best = size;
  }
  return best;
}

/** The font size a field draws its value at, in points. */
export function fieldFontSize(
  text: string,
  field: Pick<FormFieldInfo, 'rect' | 'multiline' | 'fontSize' | 'comb' | 'maxLen'>,
  measure: Measure = measureWidth
): number {
  if (field.comb && field.maxLen > 0) {
    const cell = field.rect.w / field.maxLen;
    const byCell = (cell * 0.9) / MONO_ADVANCE;
    const byHeight = (field.rect.h - 2 * FIELD_PAD_PT) / LINE_HEIGHT;
    const fitted = Math.max(4, Math.min(byCell, byHeight, 144));
    return field.fontSize > 0 ? Math.min(field.fontSize, fitted) : fitted;
  }
  return field.fontSize > 0 ? field.fontSize : autoFontSize(text, field, measure);
}

/** Letter spacing and left inset, in points, that centre each character of
 * a comb field in its own cell. */
export function combSpacing(
  field: Pick<FormFieldInfo, 'rect' | 'maxLen'>,
  sizePt: number
): { letterSpacing: number; inset: number } {
  const cell = field.rect.w / Math.max(1, field.maxLen);
  const advance = sizePt * MONO_ADVANCE;
  return { letterSpacing: cell - advance, inset: (cell - advance) / 2 };
}
