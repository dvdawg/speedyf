import { describe, expect, it } from 'vitest';
import { annotationsToMarkdown, type ExportEntry } from './annotationExport';
import type { Annotation } from '../../types/model';

const annot = (over: Partial<Annotation> = {}): Annotation => ({
  id: 'a1',
  pageId: 'p1',
  kind: 'highlight',
  rect: { x: 0, y: 0, w: 10, h: 10 },
  color: '#ffd54a',
  opacity: 0.4,
  ...over,
});

const entry = (over: Partial<ExportEntry> = {}): ExportEntry => ({
  annot: annot(),
  page: 1,
  ...over,
});

describe('annotationsToMarkdown', () => {
  it('quotes the highlighted text and cites its page', () => {
    const out = annotationsToMarkdown('Paper', [entry({ quoted: 'variance collapse', page: 4 })]);
    expect(out).toContain('# Paper');
    expect(out).toContain('> variance collapse');
    expect(out).toContain('> — p4');
  });

  it('puts your own note below what you highlighted', () => {
    const out = annotationsToMarkdown('Paper', [
      entry({ quoted: 'the bound is tight', annot: annot({ text: 'only for d > 2' }) }),
    ]);
    expect(out).toContain('> the bound is tight');
    expect(out).toContain('only for d > 2');
    expect(out.indexOf('> the bound')).toBeLessThan(out.indexOf('only for d > 2'));
  });

  it('quotes every line of a multi-line highlight', () => {
    const out = annotationsToMarkdown('', [entry({ quoted: 'first line\nsecond line' })]);
    expect(out).toContain('> first line');
    expect(out).toContain('> second line');
  });

  it('labels a standalone note that has no quote', () => {
    const out = annotationsToMarkdown('', [
      entry({ annot: annot({ kind: 'note', text: 'check this' }), page: 7 }),
    ]);
    expect(out).toContain('**Note** — p7');
    expect(out).toContain('check this');
  });

  it('leaves out annotations carrying no words at all', () => {
    // A bare box or ink stroke exports nothing a reader could use.
    const out = annotationsToMarkdown('Paper', [
      entry({ annot: annot({ kind: 'rect' }) }),
      entry({ annot: annot({ kind: 'ink' }) }),
    ]);
    expect(out).toContain('No notes yet.');
  });

  it('orders by page, however the annotations arrived', () => {
    const out = annotationsToMarkdown('', [
      entry({ quoted: 'later', page: 9 }),
      entry({ quoted: 'earlier', page: 2 }),
    ]);
    expect(out.indexOf('earlier')).toBeLessThan(out.indexOf('later'));
  });

  it('says so plainly when there is nothing to export', () => {
    expect(annotationsToMarkdown('Paper', [])).toContain('No notes yet.');
    expect(annotationsToMarkdown('', [])).toBe('No notes yet.\n');
  });
});
