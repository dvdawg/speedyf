import { describe, expect, it } from 'vitest';
import { matchScore, subsequenceScore, usageBoost } from './fuzzyScore';
import {
  defaultSections,
  flatten,
  pageJumpTarget,
  rankCandidates,
  type Candidate,
} from './paletteItems';

function candidate(over: Partial<Candidate> & { title: string }): Candidate {
  return {
    key: over.key ?? over.title,
    group: 'File',
    enabled: true,
    run: () => undefined,
    ...over,
  };
}

const none = () => 0;
const titles = (candidates: readonly Candidate[]) => candidates.map((c) => c.title);

describe('subsequenceScore', () => {
  it('rejects a query that is not a subsequence', () => {
    expect(subsequenceScore('Rotate Page Right', 'zzz')).toBeNull();
  });

  it('scores a prefix above any scattered match', () => {
    // 'Insert Blank Page' really does contain s-e-t as a subsequence
    // ("in|s||e|r|t|"), which is exactly the kind of accidental hit that
    // must never outrank a title that starts with the query.
    const prefix = subsequenceScore('Settings', 'set')!;
    const scattered = subsequenceScore('Insert Blank Page', 'set')!;
    expect(scattered).not.toBeNull();
    expect(prefix).toBeGreaterThan(scattered);
  });

  it('prefers the shorter title for the same prefix', () => {
    expect(subsequenceScore('Save', 'save')!).toBeGreaterThan(
      subsequenceScore('Save As…', 'save')!
    );
  });

  it('rewards a match at a word boundary', () => {
    // 'rp' spans two word starts in the first, sits mid-word in the second.
    expect(subsequenceScore('Rotate Page Right', 'rp')!).toBeGreaterThan(
      subsequenceScore('Corpus Report', 'rp')!
    );
  });

  it('matches an empty query at zero rather than failing', () => {
    expect(subsequenceScore('anything', '')).toBe(0);
  });
});

describe('matchScore', () => {
  it('finds a command through a hidden keyword', () => {
    expect(matchScore('Notes & Annotations', ['export', 'markdown'], 'export')).not.toBeNull();
  });

  it('ranks a keyword hit below a title hit', () => {
    const byTitle = matchScore('Export Something', undefined, 'export')!;
    const byKeyword = matchScore('Notes & Annotations', ['export'], 'export')!;
    expect(byTitle).toBeGreaterThan(byKeyword);
  });
});

describe('usageBoost', () => {
  it('is nothing for a command never used', () => {
    expect(usageBoost(0)).toBe(0);
  });

  it('flattens out rather than growing without bound', () => {
    expect(usageBoost(2000)).toBeLessThanOrEqual(40);
    expect(usageBoost(200) - usageBoost(100)).toBeLessThan(usageBoost(2) - usageBoost(1));
  });
});

describe('rankCandidates', () => {
  const base = [
    candidate({ title: 'Save' }),
    candidate({ title: 'Save As…' }),
    candidate({ title: 'Insert Blank Page', group: 'Page' }),
  ];

  it('puts the exact prefix first', () => {
    expect(titles(flatten(rankCandidates(base, 'save', none)))[0]).toBe('Save');
  });

  it('does not let a much-used command outrank a prefix match', () => {
    // The regression this scoring exists for. 'Insert Blank Page' matches
    // 'set' as a scattered subsequence; no amount of use may float it above
    // the command actually called Settings.
    const pair = [
      candidate({ title: 'Settings', group: 'Settings' }),
      candidate({ title: 'Insert Blank Page', group: 'Page' }),
    ];
    const heavy = (key: string) => (key === 'Insert Blank Page' ? 5000 : 0);
    expect(titles(flatten(rankCandidates(pair, 'set', heavy)))[0]).toBe('Settings');
  });

  it('still lets usage break a tie between comparable matches', () => {
    const pair = [
      candidate({ title: 'Rotate Page Left' }),
      candidate({ title: 'Rotate Page Right' }),
    ];
    const favouring = (key: string) => (key === 'Rotate Page Right' ? 50 : 0);
    expect(titles(flatten(rankCandidates(pair, 'rotate page', favouring)))[0]).toBe(
      'Rotate Page Right'
    );
  });

  it('hides disabled commands', () => {
    const withDisabled = [...base, candidate({ title: 'Save Everything', enabled: false })];
    expect(titles(flatten(rankCandidates(withDisabled, 'sav', none)))).not.toContain(
      'Save Everything'
    );
  });

  it('surfaces a disabled command typed out in full, and puts it last', () => {
    const withDisabled = [
      candidate({ title: 'Delete Page', enabled: false, reason: 'only one page' }),
      candidate({ title: 'Delete Page Later' }),
    ];
    const rows = flatten(rankCandidates(withDisabled, 'Delete Page', none));
    expect(titles(rows)).toContain('Delete Page');
    expect(rows[rows.length - 1]!.title).toBe('Delete Page');
  });

  it('returns nothing for an empty query', () => {
    expect(rankCandidates(base, '   ', none)).toEqual([]);
  });

  it('caps each group', () => {
    const many = Array.from({ length: 12 }, (_, i) => candidate({ title: `Save ${i}` }));
    const sections = rankCandidates(many, 'save', none, 5);
    expect(sections[0]!.items).toHaveLength(5);
  });
});

describe('defaultSections', () => {
  const base = [
    candidate({ title: 'Save' }),
    candidate({ title: 'Rotate Page Right', group: 'Page' }),
    candidate({ title: 'Delete Page', group: 'Page', enabled: false }),
  ];

  it('leads with what gets used', () => {
    const sections = defaultSections(base, ['Rotate Page Right']);
    expect(sections[0]!.group).toBe('Recent');
    expect(titles(sections[0]!.items)).toEqual(['Rotate Page Right']);
  });

  it('drops a remembered command that is not currently runnable', () => {
    // Offering it and doing nothing is worse than a shorter list.
    const sections = defaultSections(base, ['Delete Page']);
    expect(sections[0]!.group).not.toBe('Recent');
  });

  it('never lists a command twice', () => {
    const keys = flatten(defaultSections(base, ['Save'])).map((c) => c.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('omits disabled commands entirely', () => {
    expect(titles(flatten(defaultSections(base, [])))).not.toContain('Delete Page');
  });
});

describe('pageJumpTarget', () => {
  it('reads a bare number as a page', () => {
    expect(pageJumpTarget('12', 30)).toBe(12);
  });

  it('refuses a page the document does not have', () => {
    expect(pageJumpTarget('900', 30)).toBeNull();
    expect(pageJumpTarget('0', 30)).toBeNull();
  });

  it('ignores anything that is not purely a number', () => {
    expect(pageJumpTarget('page 12', 30)).toBeNull();
    expect(pageJumpTarget('12a', 30)).toBeNull();
  });
});

/** A ranking table over the real command titles.
 *
 * The unit tests above check individual scoring rules; this checks the thing
 * anyone actually notices — that typing a few letters puts the right row on
 * top. It is the test that would have caught the greedy-alignment bug, where
 * "tool" scored Ink Tool at 172 and Note Tool at 69 and dropped Select Tool
 * out of the results entirely.
 *
 * Assertions are about ordering, never exact scores, so the constants stay
 * tunable.
 */
describe('ranking over the real command titles', () => {
  const TITLES = [
    'Open PDF…',
    'Save',
    'Save As…',
    'Print…',
    'Close Tab',
    'Undo',
    'Redo',
    'Delete Selected Annotation',
    'Select Tool',
    'Highlight Tool',
    'Ink Tool',
    'Rectangle Tool',
    'Text Box Tool',
    'Note Tool',
    'Rotate Page Right',
    'Rotate Page Left',
    'Duplicate Page',
    'Delete Page',
    'Insert Blank Page',
    'Zoom In',
    'Zoom Out',
    'Fit Page',
    'Fit Width',
    'Toggle Sidebar',
    'Table of Contents',
    'Notes & Annotations',
    'Find in Document…',
    'Next Page',
    'Next Tab',
    'Previous Tab',
  ];

  const rank = (query: string) =>
    TITLES.map((title) => ({ title, score: matchScore(title, undefined, query) }))
      .filter((row): row is { title: string; score: number } => row.score !== null)
      .sort((a, b) => b.score - a.score);

  const scoreOf = (query: string, title: string) =>
    rank(query).find((row) => row.title === title)?.score ?? null;

  it('puts every Tool command together for "tool"', () => {
    // The greedy bug ordered these by where the `t` happened to land, which
    // meant one of them beat another by 2.5x for no reason a reader could see.
    const top = rank('tool').slice(0, 4);
    expect(top.every((row) => row.title.endsWith('Tool'))).toBe(true);
    const scores = top.map((row) => row.score);
    expect(Math.max(...scores) / Math.min(...scores)).toBeLessThan(1.5);
  });

  it('finds "Select Tool" from its initials', () => {
    expect(rank('st')[0]!.title).toBe('Select Tool');
    // And decisively, not by a point: both letters land on word starts.
    expect(scoreOf('st', 'Select Tool')!).toBeGreaterThan(scoreOf('st', 'Close Tab')! * 1.5);
  });

  it('prefers the shorter of two prefix matches', () => {
    expect(rank('note')[0]!.title).toBe('Note Tool');
    expect(rank('del')[0]!.title).toBe('Delete Page');
    expect(rank('fit')[0]!.title).toBe('Fit Page');
  });

  it('ranks a prefix above any alignment, however good', () => {
    expect(rank('tab')[0]!.title).toBe('Table of Contents');
    expect(scoreOf('tab', 'Table of Contents')!).toBeGreaterThan(scoreOf('tab', 'Next Tab')!);
  });

  it('puts the obvious answer first for ordinary two-letter queries', () => {
    expect(rank('zo')[0]!.title).toBe('Zoom In');
    expect(rank('tt')[0]!.title).toBe('Text Box Tool');
  });

  it('scores a tight match above a scattered one', () => {
    // 'ins' is a prefix of one and smeared across the others.
    const rows = rank('ins');
    expect(rows[0]!.title).toBe('Insert Blank Page');
    expect(rows[0]!.score).toBeGreaterThan(rows[1]!.score * 10);
  });
});

describe('long prose does not match everything', () => {
  // The real section titles that turned up under a "tool" query in the app.
  const LONG = 'B. Proof of the Leading-Order Identification and Variance Collapse';

  it('rejects a short query smeared across a long title', () => {
    // "tool" genuinely occurs in it — *t*he, *o*rder, identificati*o*n,
    // co*l*lapse — which is exactly why subsequence alone is not enough.
    expect(subsequenceScore(LONG, 'tool')).toBeNull();
  });

  it('keeps a long query that legitimately spans the same title', () => {
    // Two distant words the reader actually meant. Same span, different
    // ratio, so the bound has to be relative and not a cap.
    expect(subsequenceScore(LONG, 'proof collapse')).not.toBeNull();
  });

  it('still matches a dense hit inside a long title', () => {
    expect(subsequenceScore('B.2. Fiber Posterior Normal Form', 'fiber')).not.toBeNull();
    expect(subsequenceScore('B.2. Fiber Posterior Normal Form', 'normal form')).not.toBeNull();
  });

  it('drops the weak tail once a good answer exists', () => {
    const rows = flatten(
      rankCandidates(
        [
          candidate({ title: 'Ink Tool', group: 'Tools' }),
          candidate({ title: 'Select Tool', group: 'Tools' }),
          candidate({ title: 'B.1. Local Coordinates on a Curved Manifold', group: 'Sections' }),
        ],
        'tool',
        none
      )
    );
    expect(titles(rows)).toEqual(['Ink Tool', 'Select Tool']);
  });

  it('does not let a prefix match delete the alignment matches beside it', () => {
    // Prefix scores an order of magnitude higher by design, so measuring the
    // weak-match floor against one would wipe out every other row: typing
    // "tab" would show Table of Contents and drop Next Tab entirely.
    const rows = flatten(
      rankCandidates(
        [
          candidate({ title: 'Table of Contents', group: 'Panels' }),
          candidate({ title: 'Next Tab', group: 'Navigate' }),
          candidate({ title: 'Close Tab', group: 'File' }),
        ],
        'tab',
        none
      )
    );
    expect(titles(rows)).toContain('Next Tab');
    expect(titles(rows)).toContain('Close Tab');
    expect(titles(rows)[0]).toBe('Table of Contents');
  });
});
