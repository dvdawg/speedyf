import { describe, expect, it } from 'vitest';
import { runsUnderQuads } from './annotationText';
import type { PdfQuad } from '../../types/model';
import type { TextRunDto } from '../../types/engine';

/** A run of text at a position, with its place in the character stream. */
const run = (text: string, start: number, x: number, y: number, w = 40, h = 10): TextRunDto => ({
  text,
  start,
  x,
  y,
  w,
  h,
});

/** A highlight rectangle as the four corner points a PDF stores. */
const quad = (x: number, y: number, w: number, h: number): PdfQuad => ({
  p1: { x, y: y + h },
  p2: { x: x + w, y: y + h },
  p3: { x, y },
  p4: { x: x + w, y },
});

describe('runsUnderQuads', () => {
  it('takes the runs the highlight actually covers', () => {
    const runs = [run('inside', 0, 10, 100), run('outside', 1, 10, 300)];
    expect(runsUnderQuads([quad(0, 95, 200, 20)], runs).map((r) => r.text)).toEqual(['inside']);
  });

  it('returns runs in reading order, not page order', () => {
    // The two-column case. The right column sits at a larger x but earlier
    // text can appear below it, so sorting by position would interleave the
    // columns. The character stream is the order the words were written in.
    const runs = [run('second', 5, 320, 200), run('first', 1, 40, 100), run('third', 9, 320, 180)];
    const covering = [quad(0, 90, 600, 130)];
    expect(runsUnderQuads(covering, runs).map((r) => r.text)).toEqual(['first', 'second', 'third']);
  });

  it('follows a highlight across several lines, one quad each', () => {
    const runs = [run('line one', 0, 40, 200), run('line two', 9, 40, 185)];
    const quads = [quad(35, 198, 100, 12), quad(35, 183, 100, 12)];
    expect(runsUnderQuads(quads, runs).map((r) => r.text)).toEqual(['line one', 'line two']);
  });

  it('ignores a run the highlight merely grazes', () => {
    // The line above must not bleed in because the highlight overlaps its
    // descenders by a pixel.
    const runs = [run('grazed', 0, 40, 118, 40, 10)];
    expect(runsUnderQuads([quad(35, 100, 100, 20)], runs)).toHaveLength(0);
  });

  it('keeps a word the highlight only partly covers', () => {
    // Stopping mid-word still means you meant the word.
    const runs = [run('partial', 0, 40, 100, 40, 10)];
    expect(runsUnderQuads([quad(35, 98, 60, 14)], runs).map((r) => r.text)).toEqual(['partial']);
  });

  it('has nothing to say without quads or runs', () => {
    expect(runsUnderQuads([], [run('x', 0, 0, 0)])).toEqual([]);
    expect(runsUnderQuads([quad(0, 0, 10, 10)], [])).toEqual([]);
  });
});
