import { describe, expect, it } from 'vitest';
import { MAX_ROWS, MAX_TEXT, isBlank, parseUi } from './ui';

describe('parseUi', () => {
  it('reads a flat list of rows as one untitled section', () => {
    const ui = parseUi({ rows: [{ id: 'a', title: 'One' }] });
    expect(ui.sections).toEqual([{ rows: [{ id: 'a', title: 'One' }] }]);
  });

  it('reads titled sections', () => {
    const ui = parseUi({
      sections: [{ title: 'In library', rows: [{ id: 'a', title: 'One' }] }],
    });
    expect(ui.sections[0]!.title).toBe('In library');
  });

  it('carries the optional parts of a row through', () => {
    const ui = parseUi({
      rows: [
        {
          id: 'a',
          title: 'Attention Is All You Need',
          subtitle: 'cited 3 times',
          badge: 'in library',
          action: { command: 'open', value: '1706.03762' },
        },
      ],
    });
    expect(ui.sections[0]!.rows[0]).toEqual({
      id: 'a',
      title: 'Attention Is All You Need',
      subtitle: 'cited 3 times',
      badge: 'in library',
      action: { command: 'open', value: '1706.03762' },
    });
  });

  it('falls back to a positional id when none is given', () => {
    const ui = parseUi({ rows: [{ title: 'One' }, { title: 'Two' }] });
    expect(ui.sections[0]!.rows.map((r) => r.id)).toEqual(['row-0', 'row-1']);
  });
});

describe('parseUi rejects what it cannot read', () => {
  it('answers with an empty description for anything that is not an object', () => {
    for (const bad of [null, undefined, 'rows', 42, []]) {
      expect(parseUi(bad).sections).toEqual([]);
    }
  });

  /** One malformed row must not cost the whole panel. */
  it('drops bad rows individually', () => {
    const ui = parseUi({
      rows: [{ title: 'Good' }, null, 'nope', { subtitle: 'no title' }, { title: '   ' }],
    });
    expect(ui.sections[0]!.rows).toHaveLength(1);
    expect(ui.sections[0]!.rows[0]!.title).toBe('Good');
  });

  it('drops an action that names a namespaced command', () => {
    // Commands are the extension's own; a slash would be an attempt to reach
    // another extension's, or SpeedyF's.
    const ui = parseUi({ rows: [{ title: 'One', action: { command: 'other.ext/run' } }] });
    expect(ui.sections[0]!.rows[0]!.action).toBeUndefined();
  });

  it('drops an action with no command', () => {
    const ui = parseUi({ rows: [{ title: 'One', action: { value: 'x' } }] });
    expect(ui.sections[0]!.rows[0]!.action).toBeUndefined();
  });

  it('ignores a section that is neither titled nor populated', () => {
    expect(parseUi({ sections: [{ rows: [] }, null, 7] }).sections).toEqual([]);
  });
});

describe('parseUi bounds what an extension can put on screen', () => {
  it('caps total rows', () => {
    const rows = Array.from({ length: MAX_ROWS + 50 }, (_, i) => ({ title: `Row ${i}` }));
    const ui = parseUi({ rows });
    expect(ui.sections[0]!.rows).toHaveLength(MAX_ROWS);
  });

  /** The cap is across the description, so splitting into sections is not a
   * way around it. */
  it('caps rows across sections, not per section', () => {
    const sections = Array.from({ length: 10 }, () => ({
      rows: Array.from({ length: 100 }, (_, i) => ({ title: `Row ${i}` })),
    }));
    const total = parseUi({ sections }).sections.reduce((n, s) => n + s.rows.length, 0);
    expect(total).toBe(MAX_ROWS);
  });

  it('truncates over-long text rather than dropping the row', () => {
    const ui = parseUi({ rows: [{ title: 'x'.repeat(MAX_TEXT + 500) }] });
    const title = ui.sections[0]!.rows[0]!.title;
    expect(title.length).toBeLessThanOrEqual(MAX_TEXT + 1);
    expect(title.endsWith('…')).toBe(true);
  });
});

describe('isBlank', () => {
  it('is true when there is genuinely nothing to draw', () => {
    expect(isBlank(parseUi({}))).toBe(true);
    expect(isBlank(parseUi({ rows: [] }))).toBe(true);
  });

  it('is false once the extension has said anything', () => {
    expect(isBlank(parseUi({ rows: [{ title: 'One' }] }))).toBe(false);
    expect(isBlank(parseUi({ empty: 'No references found' }))).toBe(false);
    expect(isBlank(parseUi({ error: 'Could not read the document' }))).toBe(false);
  });
});

describe('nested rows', () => {
  /** A row that belongs to the one above it. Anything other than `true` is
   * not a claim of nesting: an extension sending a truthy string or a number
   * has misunderstood the field, and drawing it as nested would hide that. */
  it('accepts a row that says it belongs to the one above', () => {
    const ui = parseUi({ rows: [{ title: 'page 4', nested: true }] });
    expect(ui?.sections[0]!.rows[0]!.nested).toBe(true);
  });

  it('treats anything but true as not nested', () => {
    for (const value of ['true', 1, {}, null]) {
      const ui = parseUi({ rows: [{ title: 'page 4', nested: value }] });
      expect(ui?.sections[0]!.rows[0]!.nested).toBeUndefined();
    }
  });
});
