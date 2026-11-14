/** Cheap guards on alignment choices in global.css that happy-dom cannot
 * lay out: each test reads one rule's declarations. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const css = readFileSync(join(process.cwd(), 'src', 'styles', 'global.css'), 'utf8').replace(
  /\/\*[\s\S]*?\*\//g,
  ''
);

/** The declarations of the first rule whose selector list is exactly `selector`. */
function rule(selector: string): Record<string, string> {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`).exec(css);
  if (!match) throw new Error(`no rule for ${selector}`);
  const out: Record<string, string> = {};
  for (const decl of match[1]!.split(';')) {
    const at = decl.indexOf(':');
    if (at > 0) out[decl.slice(0, at).trim()] = decl.slice(at + 1).trim();
  }
  return out;
}

describe('layout rules', () => {
  it('sets the graph notes and their buttons on one fixed baseline', () => {
    const notes = rule('.graph-notes');
    expect(notes['align-items']).toBe('baseline');
    expect(notes['height']).toBeDefined();
    expect(notes['line-height']).toBeDefined();
    expect(rule('.graph-note-link')['align-self']).toBeUndefined();
  });

  it('starts the graph canvas on the same gutter as the column head', () => {
    expect(rule('.paper-list-head')['padding']).toContain('var(--col-gutter)');
    expect(rule('.graph-notes')['padding']).toContain('var(--col-gutter)');
    expect(rule('.graph-canvas')['margin']).toContain('var(--col-gutter)');
  });

  it('gives Home and Projects one head, so the shared toggle never jumps', () => {
    expect(rule('.home-panel-head')['padding']).toBe('var(--screen-head-pad)');
    expect(rule('.projects-head')['padding']).toBe('var(--screen-head-pad)');
    expect(rule('.home-screen .home-launch-toggle')['top']).toBe('22px');
  });

  it('keeps the list head row at one control height', () => {
    expect(rule('.paper-search')['height']).toBe('var(--control-h)');
  });

  it('sizes the View popover controls to their content', () => {
    expect(rule('.graph-view-menu')['justify-items']).toBe('start');
    // The threshold stepper matches the segmented control's outer height.
    expect(rule('.graph-stepper')['height']).toBe('var(--control-h)');
  });
});
