/** Guards on the polish pass in global.css: the shared tokens exist, and
 * every new piece of motion sits behind prefers-reduced-motion. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

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
