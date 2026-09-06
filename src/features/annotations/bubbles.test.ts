import { describe, expect, it } from 'vitest';
import { shadeRect, shadingFor, SHADE_INSET } from './bubbles';
import type { Annotation } from '../../types/model';

/** The glyph box PDFium actually reports for a LaTeX \bigcirc in CMSY10. */
const glyphBox = { x: 72.61, y: 531.1, w: 10.63, h: 11.13 };

const bubble = (charIndex: number): Annotation => ({
  id: `an-${charIndex}`,
  pageId: 'p1',
  kind: 'bubble',
  rect: shadeRect(glyphBox),
  color: '#1a1a1a',
  opacity: 1,
  bubbleCharIndex: charIndex,
});

describe('shadeRect', () => {
  it('centres the fill in the glyph box', () => {
    const r = shadeRect(glyphBox);
    expect(r.x + r.w / 2).toBeCloseTo(glyphBox.x + glyphBox.w / 2, 6);
    expect(r.y + r.h / 2).toBeCloseTo(glyphBox.y + glyphBox.h / 2, 6);
  });

  // A glyph box is rarely square, and an oval blot does not read as a shaded
  // bubble.
  it('is round even when the glyph box is not', () => {
    const r = shadeRect(glyphBox);
    expect(r.w).toBeCloseTo(r.h, 6);
  });

  // A shaded bubble should read as filled in, not as a small dot floating
  // inside an outline that is still visible around it.
  it('fills the bubble rather than sitting inside the ring', () => {
    const r = shadeRect(glyphBox);
    const shorter = Math.min(glyphBox.w, glyphBox.h);
    expect(r.w).toBeCloseTo(shorter * (1 - 2 * SHADE_INSET), 6);
    expect(r.w).toBe(shorter);
    // Spans the box completely on its narrow axis, so nothing of the printed
    // ring is left showing along it.
    expect(r.x).toBeCloseTo(glyphBox.x, 6);
    expect(r.x + r.w).toBeCloseTo(glyphBox.x + glyphBox.w, 6);
  });
});

describe('shadingFor', () => {
  it('finds the shading covering a bubble', () => {
    expect(shadingFor([bubble(733), bubble(739)], 739)?.id).toBe('an-739');
  });

  it('reports nothing for an unshaded bubble', () => {
    expect(shadingFor([bubble(733)], 999)).toBeUndefined();
  });

  // A shading read back out of a saved file has no character index — the PDF
  // does not record which glyph it was aimed at — so it has to be recognised
  // by where it sits, or reopening a shaded quiz would show every bubble as
  // empty and a click would stack a second mark on the first.
  it('recognises a shading reloaded from a file by its position', () => {
    const reloaded: Annotation = {
      id: 'stamp-1',
      pageId: 'p1',
      kind: 'bubble',
      rect: shadeRect(glyphBox),
      color: '#1a1a1a',
      opacity: 1,
    };
    expect(shadingFor([reloaded], 733, glyphBox)?.id).toBe('stamp-1');
  });

  it('does not attach a reloaded shading to a bubble it sits nowhere near', () => {
    const elsewhere: Annotation = {
      id: 'stamp-1',
      pageId: 'p1',
      kind: 'bubble',
      rect: { x: 300, y: 100, w: 10, h: 10 },
      color: '#1a1a1a',
      opacity: 1,
    };
    expect(shadingFor([elsewhere], 733, glyphBox)).toBeUndefined();
  });

  // Every other annotation kind lacks a bubble index, and an ink stroke that
  // happens to sit over a bubble must not read as having shaded it.
  it('ignores annotations that are not shadings', () => {
    const ink: Annotation = {
      id: 'ink-1',
      pageId: 'p1',
      kind: 'ink',
      rect: glyphBox,
      color: '#000000',
      opacity: 1,
    };
    expect(shadingFor([ink], 733)).toBeUndefined();
  });
});
