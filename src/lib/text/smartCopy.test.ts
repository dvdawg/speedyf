import { describe, expect, it } from 'vitest';
import { smartCopyText, type CopyRun } from './smartCopy';

const H = 10;
let cursor = 0;

/** One run on a line whose baseline sits at `y`, starting at `x`. */
function run(text: string, x: number, y: number, options: Partial<CopyRun> = {}): CopyRun {
  cursor += text.length + 1;
  return {
    text,
    start: options.start ?? cursor,
    x,
    y,
    w: options.w ?? text.length * 5,
    h: options.h ?? H,
    page: options.page ?? 0,
  };
}

/** Consecutive body lines, 14pt apart, in one column at x=60. */
function lines(texts: string[], startY = 700, x = 60): CopyRun[] {
  return texts.map((text, i) => run(text, x, startY - i * 14, { w: 480 }));
}

describe('smartCopyText', () => {
  it('joins wrapped lines into one paragraph', () => {
    expect(smartCopyText(lines(['the quick brown', 'fox jumps over', 'the lazy dog']))).toBe(
      'the quick brown fox jumps over the lazy dog'
    );
  });

  it('rejoins a word broken across a line end', () => {
    expect(smartCopyText(lines(['we consider the under-', 'lying distribution']))).toBe(
      'we consider the underlying distribution'
    );
  });

  it('rejoins a word broken with a soft hyphen', () => {
    expect(smartCopyText(lines(['an approxi­', 'mation of it']))).toBe('an approximation of it');
  });

  it('keeps a hyphen when the next line starts a new token', () => {
    // "Transformer-" then "Based" is a compound, not a wrapped word.
    expect(smartCopyText(lines(['a Transformer-', 'Based approach']))).toBe(
      'a Transformer- Based approach'
    );
  });

  it('expands ligatures', () => {
    expect(smartCopyText(lines(['the ﬁrst ﬂow of eﬀort', 'is signiﬁcant']))).toBe(
      'the first flow of effort is significant'
    );
  });

  it('breaks a paragraph on a vertical gap', () => {
    const first = lines(['end of the first thought.'], 700);
    const second = lines(['A second thought begins.'], 670);
    expect(smartCopyText([...first, ...second])).toBe(
      'end of the first thought.\n\nA second thought begins.'
    );
  });

  it('breaks a paragraph on an indent even without a gap', () => {
    const body = lines(['the first paragraph ends here.'], 700);
    const indented = [run('A new paragraph starts.', 72, 686, { w: 460 })];
    expect(smartCopyText([...body, ...indented])).toBe(
      'the first paragraph ends here.\n\nA new paragraph starts.'
    );
  });

  it('keeps prose flowing across a column break', () => {
    // Second column restarts at the top of the page: the text moves *up*, so
    // the vertical gap must not be read as a paragraph boundary.
    const left = lines(['a sentence that runs', 'off the first column'], 700, 60);
    const right = lines(['and continues here'], 700, 330);
    expect(smartCopyText([...left, ...right])).toBe(
      'a sentence that runs off the first column and continues here'
    );
  });

  it('keeps prose flowing across a page break', () => {
    const end = [run('the argument continues', 60, 90, { w: 480, page: 0 })];
    const next = [run('on the following page', 60, 700, { w: 480, page: 1 })];
    expect(smartCopyText([...end, ...next])).toBe('the argument continues on the following page');
  });

  it('does not split a line at a superscript citation marker', () => {
    // The marker sits higher and smaller but keeps moving rightwards, so it
    // belongs to the line it interrupts.
    const selection = [
      run('as shown previously', 60, 700, { w: 100 }),
      run('12', 160, 704, { w: 8, h: 6 }),
      run('the result holds', 172, 700, { w: 90 }),
    ];
    expect(smartCopyText(selection)).toBe('as shown previously12 the result holds');
  });

  it('inserts a space where positioned runs leave a word gap', () => {
    const selection = [
      run('two', 60, 700, { w: 20 }),
      run('words', 90, 700, { w: 30 }),
      run('.', 120, 700, { w: 3 }),
    ];
    expect(smartCopyText(selection)).toBe('two words.');
  });

  it('orders runs by the extractor char stream, not by arrival', () => {
    const a = run('first', 60, 700, { start: 10, w: 30 });
    const b = run('second', 100, 700, { start: 20, w: 40 });
    expect(smartCopyText([b, a])).toBe('first second');
  });

  it('returns empty for an empty selection', () => {
    expect(smartCopyText([])).toBe('');
  });
});
