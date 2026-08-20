/** Validating the page range a person types into the print dialog.
 *
 * The engine checks this again before it builds an argument list — this copy
 * exists so the dialog can say "3-99 is past the end" while they type, rather
 * than failing after they press Print. */

export interface RangeProblem {
  message: string;
}

/** Parse a CUPS page-range list, or explain what is wrong with it.
 *
 * Accepts `3`, `2-5`, `1,4,7-9`. Rejects open ends, reversed pairs, page 0,
 * and anything past the document — all of which CUPS would either refuse or,
 * worse, quietly print nothing for. */
export function checkPageRange(value: string, pageCount: number): RangeProblem | null {
  const text = value.trim();
  if (text === '') return { message: 'Enter a range like 1-4, or choose All.' };
  if (/\s/.test(text)) return { message: 'Ranges cannot contain spaces.' };

  for (const part of text.split(',')) {
    const bounds = part.split('-');
    if (bounds.length > 2 || bounds.some((bound) => !/^\d+$/.test(bound))) {
      return { message: `"${part}" is not a page or a range.` };
    }
    const numbers = bounds.map(Number);
    if (numbers.some((n) => n < 1)) return { message: 'Pages start at 1.' };
    if (numbers.length === 2 && numbers[1]! < numbers[0]!) {
      return { message: `"${part}" runs backwards.` };
    }
    const highest = numbers[numbers.length - 1]!;
    if (highest > pageCount) {
      return { message: `This document has ${pageCount} pages.` };
    }
  }
  return null;
}
