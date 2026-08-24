import { describe, expect, it } from 'vitest';
import {
  COMMANDS_RUN,
  canRunGroup,
  canonicalChord,
  chordMatches,
  parseChord,
  refusalReason,
  resolveBinding,
  resolveBindings,
} from './capabilities';

const granted = [COMMANDS_RUN];

describe('canRunGroup', () => {
  it('allows the groups that only change what you are looking at', () => {
    for (const group of ['View', 'Navigate', 'Panels', 'Search'] as const) {
      expect(canRunGroup(granted, group)).toBe(true);
    }
  });

  /** The whole point of the permission: an extension bound to a key must not
   * be able to delete a page or overwrite the file. */
  it('refuses anything that changes the document, the file or settings', () => {
    for (const group of ['File', 'Edit', 'Page', 'Tools', 'Settings'] as const) {
      expect(canRunGroup(granted, group)).toBe(false);
    }
  });

  it('refuses everything without the permission', () => {
    expect(canRunGroup([], 'View')).toBe(false);
    expect(canRunGroup(['library:read'], 'Navigate')).toBe(false);
  });

  it('refuses a command that does not exist', () => {
    expect(canRunGroup(granted, undefined)).toBe(false);
  });
});

describe('refusalReason', () => {
  it('names the missing permission when there is none', () => {
    expect(refusalReason([], 'view.zoomIn', 'View')).toContain(COMMANDS_RUN);
  });

  it('names the group when the permission is held but the command is off limits', () => {
    const reason = refusalReason(granted, 'edit.undo', 'Edit');
    expect(reason).toContain('Edit');
    expect(reason).toContain('View');
  });

  it('says so plainly when the command is simply unknown', () => {
    expect(refusalReason(granted, 'nope.nope', undefined)).toContain('no command');
  });
});

describe('parseChord', () => {
  it('reads a bare key', () => {
    expect(parseChord('j')).toEqual({ key: 'j', mod: false, shift: false, alt: false });
  });

  it('lower-cases single characters so case cannot matter', () => {
    expect(parseChord('J')?.key).toBe('j');
  });

  it('keeps named keys as written', () => {
    expect(parseChord('ArrowDown')?.key).toBe('ArrowDown');
    expect(parseChord('Escape')?.key).toBe('Escape');
  });

  it('reads modifiers, including their aliases', () => {
    expect(parseChord('mod+shift+p')).toEqual({
      key: 'p',
      mod: true,
      shift: true,
      alt: false,
    });
    expect(parseChord('cmd+k')?.mod).toBe(true);
    expect(parseChord('ctrl+k')?.mod).toBe(true);
    expect(parseChord('option+k')?.alt).toBe(true);
  });

  it('tolerates whitespace around segments', () => {
    expect(parseChord(' mod + shift + p ')).toEqual(parseChord('mod+shift+p'));
  });

  it('refuses anything it would otherwise have to guess at', () => {
    for (const bad of ['', '+', 'mod+', 'shift', 'mod', 'meta+k', 'mod+nonsense+k']) {
      expect(parseChord(bad)).toBeNull();
    }
  });
});

describe('chordMatches', () => {
  const event = (init: Partial<KeyboardEvent>) => init as KeyboardEvent;

  it('matches a bare key only when no modifier is held', () => {
    const chord = parseChord('j')!;
    expect(chordMatches(chord, event({ key: 'j', shiftKey: false, altKey: false }), false)).toBe(
      true
    );
    expect(chordMatches(chord, event({ key: 'j', shiftKey: false, altKey: false }), true)).toBe(
      false
    );
  });

  it('distinguishes shift', () => {
    const chord = parseChord('shift+g')!;
    expect(chordMatches(chord, event({ key: 'G', shiftKey: true, altKey: false }), false)).toBe(
      true
    );
    expect(chordMatches(chord, event({ key: 'g', shiftKey: false, altKey: false }), false)).toBe(
      false
    );
  });

  it('does not match a different key', () => {
    const chord = parseChord('j')!;
    expect(chordMatches(chord, event({ key: 'k', shiftKey: false, altKey: false }), false)).toBe(
      false
    );
  });
});

describe('canonicalChord', () => {
  it('puts modifiers in a fixed order so equivalent specs compare equal', () => {
    expect(canonicalChord(parseChord('shift+mod+p')!)).toBe(
      canonicalChord(parseChord('mod+shift+p')!)
    );
  });
});

describe('resolveBinding', () => {
  const reserved = new Set(['mod+s', 'mod+shift+p']);

  it('accepts a free chord', () => {
    const { binding, error } = resolveBinding('com.a.b', { command: 'down', key: 'j' }, reserved);
    expect(error).toBeNull();
    expect(binding).toMatchObject({ extensionId: 'com.a.b', command: 'down' });
  });

  /** Built-ins always win. Losing the app's own save shortcut to an extension
   * would be a far worse failure than refusing the extension's binding. */
  it('refuses a chord SpeedyF has already bound', () => {
    const { binding, error } = resolveBinding('com.a.b', { command: 'x', key: 'mod+s' }, reserved);
    expect(binding).toBeNull();
    expect(error).toMatch(/already a SpeedyF shortcut/);
  });

  it('refuses a reserved chord however it was spelled', () => {
    const { error } = resolveBinding('com.a.b', { command: 'x', key: 'shift+mod+p' }, reserved);
    expect(error).toMatch(/already a SpeedyF shortcut/);
  });

  it('reports an unreadable chord rather than dropping it silently', () => {
    const { binding, error } = resolveBinding('com.a.b', { command: 'x', key: 'mod+' }, reserved);
    expect(binding).toBeNull();
    expect(error).toMatch(/not a key SpeedyF understands/);
  });
});

describe('resolveBindings', () => {
  const reserved = new Set(['mod+s']);

  it('collects bindings across extensions', () => {
    const { bindings, errors } = resolveBindings(
      [
        { extensionId: 'com.a.vim', bindings: [{ command: 'down', key: 'j' }] },
        { extensionId: 'com.b.other', bindings: [{ command: 'up', key: 'k' }] },
      ],
      reserved
    );
    expect(bindings).toHaveLength(2);
    expect(errors).toEqual([]);
  });

  it('gives a contested chord to the first claimant and tells the second why', () => {
    const { bindings, errors } = resolveBindings(
      [
        { extensionId: 'com.a.vim', bindings: [{ command: 'down', key: 'j' }] },
        { extensionId: 'com.b.other', bindings: [{ command: 'jump', key: 'j' }] },
      ],
      reserved
    );
    expect(bindings).toHaveLength(1);
    expect(bindings[0]!.extensionId).toBe('com.a.vim');
    expect(errors[0]).toMatchObject({ extensionId: 'com.b.other' });
    expect(errors[0]!.message).toContain('com.a.vim');
  });

  it('keeps the rest of an extensions bindings when one is refused', () => {
    const { bindings, errors } = resolveBindings(
      [
        {
          extensionId: 'com.a.vim',
          bindings: [
            { command: 'save', key: 'mod+s' },
            { command: 'down', key: 'j' },
          ],
        },
      ],
      reserved
    );
    expect(bindings).toHaveLength(1);
    expect(bindings[0]!.command).toBe('down');
    expect(errors).toHaveLength(1);
  });
});
