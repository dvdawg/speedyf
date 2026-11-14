/** Guards on the polish pass in global.css: the shared tokens exist, and
 * every new piece of motion sits behind prefers-reduced-motion. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PAGE_GAP } from '../stores/viewportStore';

const css = readFileSync(join(process.cwd(), 'src', 'styles', 'global.css'), 'utf8').replace(
  /\/\*[\s\S]*?\*\//g,
  ''
);

/** The bodies of every block opened by `head`, braces matched. */
function blocks(source: string, head: string): string[] {
  const out: string[] = [];
  let from = 0;
  for (;;) {
    const at = source.indexOf(head, from);
    if (at < 0) return out;
    const open = source.indexOf('{', at);
    let depth = 0;
    let i = open;
    for (; i < source.length; i++) {
      if (source[i] === '{') depth++;
      else if (source[i] === '}' && --depth === 0) break;
    }
    out.push(source.slice(open + 1, i));
    from = i + 1;
  }
}

const MOTION_OK = '@media (prefers-reduced-motion: no-preference)';

/** The stylesheet with every no-preference block cut out. */
function outsideMotionBlocks(): string {
  let rest = css;
  for (const body of blocks(css, MOTION_OK)) rest = rest.replace(body, '');
  return rest;
}

/** The declarations of the first rule in :root. */
function rootTokens(): string {
  return /:root\s*\{([^}]*)\}/.exec(css)![1]!;
}

describe('polish: motion', () => {
  it('defines the quick ease once, short and without spring', () => {
    expect(rootTokens()).toMatch(/--ease-quick:\s*1[0-5]0ms ease-out/);
  });

  it('uses the quick ease and the open animations only when motion is welcome', () => {
    const rest = outsideMotionBlocks();
    // Defined in :root, used nowhere outside the guarded blocks.
    expect(rest.match(/var\(--ease-quick\)/g)).toBeNull();
    expect(rest).not.toMatch(/animation:\s*(pop-in|drawer-in)/);
    const guarded = blocks(css, MOTION_OK).join('\n');
    expect(guarded).toMatch(/animation:\s*pop-in/);
    expect(guarded).toMatch(/animation:\s*drawer-in 1[5-8]0ms/);
  });

  it('moves popovers by a hair at most', () => {
    const popIn = blocks(css, '@keyframes pop-in')[0]!;
    expect(popIn).toMatch(/scale:\s*0\.98/);
    expect(popIn).not.toMatch(/transform|translate/);
  });
});

describe('polish: graph', () => {
  it('eases paint only, never position, and the find ring plays once', () => {
    const guarded = blocks(css, MOTION_OK).join('\n');
    const graphTransitions = [...guarded.matchAll(/\.graph-[^{]*\{[^}]*transition:([^;]*);/g)].map(
      (m) => m[1]!
    );
    expect(graphTransitions.length).toBeGreaterThan(0);
    for (const t of graphTransitions)
      expect(t).not.toMatch(/transform|translate|\b(x|y|cx|cy|d)\b /);
    const stepped = /\.graph-node\.is-stepped \.graph-node-halo\s*\{([^}]*)\}/.exec(guarded)![1]!;
    expect(stepped).not.toMatch(/infinite/);
  });
});

describe('polish: numerals', () => {
  /** Every selector that asks for tabular figures. */
  const tabular = new Set(
    [...css.matchAll(/([^{}]+)\{[^}]*font-variant-numeric:\s*tabular-nums/g)].flatMap((m) =>
      m[1]!.split(',').map((sel) => sel.trim())
    )
  );

  it('keeps changing and stacked figures one width', () => {
    for (const sel of [
      '.page-input',
      '.zoom-input',
      '.tb-total',
      '.status-bar',
      '.paper-find-count',
      '.search-count',
      '.graph-stepper-value',
      '.projects-row-count',
      '.paper-group-count',
      '.paper-row-meta',
      '.library-result-count',
    ]) {
      expect(tabular.has(sel), sel).toBe(true);
    }
  });
});

describe('polish: rows', () => {
  /** The declarations of the first rule whose selector list is exactly `selector`. */
  function rule(selector: string): string {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const match = new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`).exec(css);
    if (!match) throw new Error(`no rule for ${selector}`);
    return match[1]!;
  }

  it('defines one hover, selection and radius', () => {
    for (const token of ['--row-hover', '--row-selected', '--row-selected-text', '--row-radius']) {
      expect(rootTokens()).toContain(`${token}:`);
    }
  });

  it('gives every list-like row the shared hover and selection', () => {
    for (const sel of [
      '.paper-row:hover',
      '.projects-row:hover',
      '.home-recent-row:hover',
      '.note-row:hover',
      '.outline-row:hover:not(:disabled)',
      '.search-result:hover',
      '.library-result-hit:hover',
      '.ext-panel-row.is-actionable:hover',
      '.note-picker-row:hover',
    ]) {
      expect(rule(sel), sel).toContain('var(--row-hover)');
    }
    for (const sel of [
      '.paper-row.is-selected',
      '.projects-row.is-active',
      '.palette-row.is-active',
      '.search-result.is-current',
      '.note-picker-row.is-current',
    ]) {
      expect(rule(sel), sel).toContain('var(--row-selected)');
    }
  });
});

describe('polish: dim pages', () => {
  it('filters the rasters only under the dark theme, never by an overlay', () => {
    const dimRules = [...css.matchAll(/([^{}]*data-dim-pages[^{}]*)\{([^}]*)\}/g)];
    expect(dimRules.length).toBeGreaterThan(0);
    for (const [, selector] of dimRules) {
      for (const part of selector!.split(/,(?![^(]*\))/)) {
        expect(part).toContain(":root[data-theme='dark'][data-dim-pages='on']");
      }
    }
    const filters = dimRules.map(([, , body]) => body!).join('\n');
    const brightness = Number(/brightness\(([\d.]+)\)/.exec(filters)![1]);
    const sepia = Number(/sepia\(([\d.]+)\)/.exec(filters)![1]);
    expect(brightness).toBeGreaterThanOrEqual(0.8);
    expect(brightness).toBeLessThan(1);
    expect(sepia).toBeLessThan(0.1);
  });
});

describe('polish: page presentation', () => {
  it('sets pages a steady gap apart on a desk, shadowed less than a popover', () => {
    expect(PAGE_GAP).toBeGreaterThanOrEqual(12);
    expect(PAGE_GAP).toBeLessThanOrEqual(16);
    expect(/(?:^|})\s*\.page\s*\{([^}]*)\}/.exec(css)![1]).toContain(
      'box-shadow: var(--page-shadow)'
    );
    const dark = /:root\[data-theme='dark'\]\s*\{([^}]*)\}/.exec(css)![1]!;
    expect(rootTokens()).toContain('--page-shadow:');
    expect(dark).toContain('--page-shadow:');
    expect(dark).toContain('--viewer-bg:');
  });
});

describe('polish: scrollbars', () => {
  it('thins the columns from shared tokens and leaves the page viewport alone', () => {
    expect(rootTokens()).toMatch(/--scrollbar-thumb:/);
    expect(rootTokens()).toMatch(/--scrollbar-thumb-hover:/);
    expect(css).toMatch(/scrollbar-color:\s*var\(--scrollbar-thumb\) transparent/);
    expect(css).not.toMatch(/\.viewer[^{,]*::-webkit-scrollbar/);
  });
});
