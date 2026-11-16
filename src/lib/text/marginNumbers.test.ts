import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import { marginNumbers } from './marginNumbers';

const run = (text: string, x: number, y: number, w: number, h = 8): TextRunDto => ({
  text,
  start: 0,
  x,
  y,
  w,
  h,
});

/** A two-column lineno page: the left column's numbers flush right in the
 * left margin (746, 747, ...), the right column's in the gutter; each
 * number in the stream just before its line, as lineno emits them. Then a
 * page number, a footnote mark, and a small table of integers. */
function linenoPage(): { runs: TextRunDto[]; numbers: Set<number> } {
  const runs: TextRunDto[] = [];
  const numbers = new Set<number>();
  for (let i = 0; i < 6; i += 1) {
    const y = 700 - i * 11;
    const label = String(746 + i);
    numbers.add(runs.length);
    runs.push(run(label, 60 - label.length * 4, y, label.length * 4));
    runs.push(run(`left column words of line ${i}`, 72, y, 218));
  }
  for (let i = 0; i < 6; i += 1) {
    const y = 700 - i * 11;
    const label = String(752 + i);
    numbers.add(runs.length);
    runs.push(run(label, 312 - label.length * 4, y, label.length * 4));
    runs.push(run(`right column words of line ${i}`, 320, y, 220));
  }
  // a footnote mark right after a word, and the page number
  runs.push(run('see the note', 72, 600, 50));
  runs.push(run('1', 123, 603, 3, 5));
  runs.push(run('12', 300, 40, 8));
  // a table: integers in an aligned column, not counting up
  for (const [i, value] of ['100', '20', '350', '7'].entries()) {
    runs.push(run(`Model ${i}`, 120, 500 - i * 11, 40));
    runs.push(run(value, 200 - value.length * 4, 500 - i * 11, value.length * 4));
  }
  return { runs, numbers };
}

describe('marginNumbers', () => {
  it('finds the line numbers in both margins and nothing else', () => {
    const { runs, numbers } = linenoPage();
    const found = marginNumbers(runs);
    runs.forEach((r, i) => expect([r.text, found[i]]).toEqual([r.text, numbers.has(i)]));
  });

  it('keeps numbers that sit among the words', () => {
    // numbered lines, but the column crosses a line of text
    const runs = [
      run('1', 100, 700, 4),
      run('2', 100, 689, 4),
      run('3', 100, 678, 4),
      run('a wide line through them', 60, 676, 200),
    ];
    expect(marginNumbers(runs)).toEqual([true, true, false, false]);
  });

  it('needs a column of them', () => {
    expect(marginNumbers([run('746', 40, 700, 12), run('text', 72, 700, 100)])).toEqual([
      false,
      false,
    ]);
  });
});
