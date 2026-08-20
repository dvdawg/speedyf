import { describe, expect, it } from 'vitest';
import {
  anchorsForPosition,
  contextForMatch,
  contextForPosition,
  outlineAsEntries,
  pageSpaceY,
  READING_FOCUS,
} from './context';
import type { FormalEntry, OutlineNode } from '../../types/engine';

const entry = (depth: number, label: string, page: number, charIndex: number): FormalEntry => ({
  heading: depth === 0,
  depth,
  label,
  page,
  y: 0,
  charIndex,
});

const doc: FormalEntry[] = [
  entry(0, '1 Setup', 0, 10),
  entry(1, 'Definition 1.1', 0, 40),
  entry(1, 'Theorem 1.2', 0, 120),
  entry(0, '2 Results', 1, 5),
  entry(1, 'Theorem 2.1', 1, 60),
];

describe('contextForMatch', () => {
  it('attributes a hit to the environment it falls inside', () => {
    expect(contextForMatch(doc, 0, 130)).toEqual({
      section: '1 Setup',
      subsection: null,
      environment: 'Theorem 1.2',
    });
  });

  it('reports the section alone before the first environment in it', () => {
    expect(contextForMatch(doc, 0, 20)).toEqual({
      section: '1 Setup',
      subsection: null,
      environment: null,
    });
  });

  it('does not carry an environment across a section boundary', () => {
    // page 1 char 10 is past "2 Results" but before "Theorem 2.1"; the answer
    // is the new section, not the last theorem of the previous one
    expect(contextForMatch(doc, 1, 10)).toEqual({
      section: '2 Results',
      subsection: null,
      environment: null,
    });
  });

  it('crosses pages correctly', () => {
    expect(contextForMatch(doc, 1, 90)).toEqual({
      section: '2 Results',
      subsection: null,
      environment: 'Theorem 2.1',
    });
  });

  it('reports nothing for a hit before any structure', () => {
    expect(contextForMatch(doc, 0, 1)).toEqual({
      section: null,
      subsection: null,
      environment: null,
    });
  });

  it('reports nothing when the document has no index', () => {
    expect(contextForMatch([], 3, 50)).toEqual({
      section: null,
      subsection: null,
      environment: null,
    });
  });
});

// same document, now with y coordinates (page space, y-up: larger is higher)
const positioned: FormalEntry[] = [
  { heading: true, depth: 0, label: '1 Setup', page: 0, y: 700, charIndex: 10 },
  { heading: false, depth: 1, label: 'Definition 1.1', page: 0, y: 640, charIndex: 40 },
  { heading: false, depth: 1, label: 'Theorem 1.2', page: 0, y: 500, charIndex: 120 },
  { heading: true, depth: 0, label: '2 Results', page: 1, y: 720, charIndex: 5 },
  { heading: false, depth: 1, label: 'Theorem 2.1', page: 1, y: 600, charIndex: 60 },
];

describe('contextForPosition', () => {
  it('reports the environment the viewport is inside', () => {
    expect(contextForPosition(positioned, 0, 450)).toEqual({
      section: '1 Setup',
      subsection: null,
      environment: 'Theorem 1.2',
    });
  });

  it('treats a larger y on the same page as earlier in the document', () => {
    // y=660 is above Definition 1.1 (y=640) but below the section heading
    expect(contextForPosition(positioned, 0, 660)).toEqual({
      section: '1 Setup',
      subsection: null,
      environment: null,
    });
  });

  it('does not carry an environment across a section boundary', () => {
    expect(contextForPosition(positioned, 1, 700)).toEqual({
      section: '2 Results',
      subsection: null,
      environment: null,
    });
  });

  it('reports nothing above the first heading', () => {
    expect(contextForPosition(positioned, 0, 780)).toEqual({
      section: null,
      subsection: null,
      environment: null,
    });
  });
});

describe('outlineAsEntries', () => {
  const node = (
    title: string,
    page: number | null,
    y: number | null,
    children: OutlineNode[] = []
  ): OutlineNode => ({ title, page, y, children });

  it('keeps bookmark nesting as section and subsection', () => {
    const entries = outlineAsEntries([
      node('Introduction', 0, 700, [node('Motivation', 0, 500, [])]),
      node('Method', 2, 600),
    ]);
    expect(entries.map((e) => e.label)).toEqual(['Introduction', 'Motivation', 'Method']);
    expect(contextForPosition(entries, 0, 450)).toEqual({
      section: 'Introduction',
      subsection: 'Motivation',
      environment: null,
    });
  });

  it('caps deeper nesting rather than adding a third level', () => {
    const entries = outlineAsEntries([
      node('A', 0, 700, [node('A.1', 0, 600, [node('A.1.1', 0, 500, [])])]),
    ]);
    expect(entries.map((e) => e.depth)).toEqual([0, 1, 1]);
  });

  it('a following section clears the subsection under the previous one', () => {
    const entries = outlineAsEntries([
      node('One', 0, 700, [node('One.a', 0, 600, [])]),
      node('Two', 1, 700),
    ]);
    expect(contextForPosition(entries, 1, 500)).toEqual({
      section: 'Two',
      subsection: null,
      environment: null,
    });
  });

  it('skips bookmarks with no destination page', () => {
    expect(outlineAsEntries([node('Nowhere', null, null)])).toEqual([]);
  });

  it('puts a bookmark with no y at the top of its page', () => {
    const entries = outlineAsEntries([node('Chapter', 3, null)]);
    expect(contextForPosition(entries, 3, 800).section).toBe('Chapter');
  });

  it('orders bookmarks by document position even when authored out of order', () => {
    const entries = outlineAsEntries([node('Later', 5, 700), node('Earlier', 1, 700)]);
    expect(entries.map((e) => e.label)).toEqual(['Earlier', 'Later']);
  });
});

describe('anchorsForPosition', () => {
  it('returns the entry itself, so navigation is unambiguous', () => {
    // two sections share a title; looking one up by label could not tell them
    // apart, which is why the scan hands back the entry
    const repeated: FormalEntry[] = [
      { heading: true, depth: 0, label: 'Appendix', page: 0, y: 700, charIndex: 0 },
      { heading: true, depth: 0, label: 'Appendix', page: 4, y: 700, charIndex: 0 },
    ];
    expect(anchorsForPosition(repeated, 4, 500).section).toEqual(repeated[1]);
  });
});

describe('tracking a page of closely spaced entries', () => {
  // Real anchors from a pdflatex build: six definitions on one 792pt page,
  // each about 20pt below the last. This is the case that exposed probing the
  // viewport's top edge — reading the fourth still reported the second.
  const defs: FormalEntry[] = [
    { heading: true, depth: 0, label: '1 Setup', page: 0, y: 667.198, charIndex: 0 },
    { heading: false, depth: 1, label: 'Definition 1.1', page: 0, y: 647.37, charIndex: 1 },
    { heading: false, depth: 1, label: 'Definition 1.2', page: 0, y: 627.444, charIndex: 2 },
    { heading: false, depth: 1, label: 'Definition 1.3', page: 0, y: 607.519, charIndex: 3 },
    { heading: false, depth: 1, label: 'Definition 1.4', page: 0, y: 587.594, charIndex: 4 },
    { heading: false, depth: 1, label: 'Definition 1.5', page: 0, y: 567.668, charIndex: 5 },
    { heading: false, depth: 1, label: 'Definition 1.6', page: 0, y: 547.743, charIndex: 6 },
  ];

  const PAGE_TOP = 24;
  const PAGE_HEIGHT = 792;
  const CONTAINER = 700;

  /** What the header reports when the viewer is scrolled to `scrollTop`. */
  const reported = (scrollTop: number, zoom = 1) => {
    const focus = scrollTop + CONTAINER * READING_FOCUS;
    const y = pageSpaceY(focus, PAGE_TOP, zoom, PAGE_HEIGHT);
    return contextForPosition(defs, 0, y).environment;
  };

  it('advances through every entry as the page scrolls', () => {
    // scroll so the reading point lands just below each definition in turn
    const seen = [0, 20, 40, 60, 80, 100].map((extra) =>
      // put the focus 5pt below definition n's anchor
      reported(PAGE_TOP + (PAGE_HEIGHT - 647.37 + 5 + extra) - CONTAINER * READING_FOCUS)
    );
    expect(seen).toEqual([
      'Definition 1.1',
      'Definition 1.2',
      'Definition 1.3',
      'Definition 1.4',
      'Definition 1.5',
      'Definition 1.6',
    ]);
  });

  it('does not stick on an earlier entry once the next one is being read', () => {
    const atFourth = PAGE_TOP + (PAGE_HEIGHT - 587.594 + 5) - CONTAINER * READING_FOCUS;
    expect(reported(atFourth)).toBe('Definition 1.4');
    expect(reported(atFourth)).not.toBe('Definition 1.3');
  });

  it('reports the section alone above the first definition', () => {
    const aboveAll = PAGE_TOP + (PAGE_HEIGHT - 660) - CONTAINER * READING_FOCUS;
    expect(reported(aboveAll)).toBeNull();
    const y = pageSpaceY(aboveAll + CONTAINER * READING_FOCUS, PAGE_TOP, 1, PAGE_HEIGHT);
    expect(contextForPosition(defs, 0, y).section).toBe('1 Setup');
  });

  it('holds under zoom, since the anchor space is zoom independent', () => {
    const contentAt = (zoom: number) =>
      PAGE_TOP + (PAGE_HEIGHT - 587.594 + 5) * zoom - CONTAINER * READING_FOCUS;
    expect(reported(contentAt(1.75), 1.75)).toBe('Definition 1.4');
    expect(reported(contentAt(0.6), 0.6)).toBe('Definition 1.4');
  });
});
