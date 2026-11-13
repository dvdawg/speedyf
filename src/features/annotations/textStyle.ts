import type { Annotation, PageEntry, PdfRect, TextFontFamily, TextFormat } from '../../types/model';

export const DEFAULT_TEXT_STYLE: TextFormat & { fontSizePt: number } = {
  fontFamily: 'Helvetica',
  fontSizePt: 12,
  bold: false,
  italic: false,
  underline: false,
};

export const textFontCss: Record<TextFontFamily, string> = {
  Helvetica: 'Helvetica, Arial, sans-serif',
  'Times-Roman': '"Times New Roman", Times, serif',
  Courier: '"Courier New", Courier, monospace',
};

export function textStyleOf(a: Partial<Annotation>) {
  return {
    fontSizePt: a.fontSizePt ?? DEFAULT_TEXT_STYLE.fontSizePt,
    fontFamily: a.fontFamily ?? DEFAULT_TEXT_STYLE.fontFamily,
    bold: a.bold ?? false,
    italic: a.italic ?? false,
    underline: a.underline ?? false,
  };
}

let measureContext: CanvasRenderingContext2D | null | undefined;

/** Use the same browser font as the overlay; no per-keystroke DOM layout. */
function textMeasure(a: Partial<Annotation>): (text: string) => number {
  const style = textStyleOf(a);
  measureContext ??= document.createElement('canvas').getContext('2d');
  if (!measureContext) return (text) => text.length * style.fontSizePt * 0.6;
  const context = measureContext;
  context.font = `${style.italic ? 'italic ' : ''}${style.bold ? 'bold ' : ''}${style.fontSizePt}px ${textFontCss[style.fontFamily]}`;
  return (text) => context.measureText(text).width;
}

/** Keep the top edge anchored as text grows. Manual resizing fixes the width,
 * but typing or enlarging the font can still add enough height for new lines. */
export function fitTextBox(
  a: Annotation,
  page: Pick<PageEntry, 'widthPt' | 'heightPt'>,
  measure = textMeasure(a)
): PdfRect {
  const size = textStyleOf(a).fontSizePt;
  const paragraphs = (a.text ?? '').replace(/\r\n?/g, '\n').split('\n');
  const available = Math.max(1, page.widthPt - a.rect.x);
  const width = a.autoSize
    ? Math.min(
        available,
        Math.max(
          size * 8,
          Math.min(300, paragraphs.reduce((w, line) => Math.max(w, measure(line)), 0) + 2)
        )
      )
    : a.rect.w;
  let lines = 0;
  for (const paragraph of paragraphs) {
    let line = '';
    lines++;
    for (const word of paragraph.match(/\S+\s*|\s+/g) ?? []) {
      if (line && measure(line + word.trimEnd()) > width) {
        lines++;
        line = '';
      }
      for (const char of word) {
        if (line && !/\s/.test(char) && measure(line + char) > width) {
          lines++;
          line = '';
        }
        line += char;
      }
    }
  }
  const height = Math.min(
    page.heightPt,
    Math.max(a.autoSize ? 0 : a.rect.h, lines * size * 1.3 + 2)
  );
  const top = Math.max(height, Math.min(page.heightPt, a.rect.y + a.rect.h));
  return { x: a.rect.x, y: top - height, w: width, h: height };
}
