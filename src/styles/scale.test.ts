/** Guards on the shell's type scale, corner radii and cursors: interface
 * text takes one of five sizes, corners one of three radii, and only links
 * show the pointing hand. Covers every stylesheet under src except the
 * markup export's print sheet, which lays out paper, not the window. */
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(process.cwd(), 'src');
const EXCLUDED = new Set(['features/markup/print.css']);

function stylesheets(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return stylesheets(path);
    return entry.name.endsWith('.css') && !EXCLUDED.has(relative(SRC, path)) ? [path] : [];
  });
}

interface Rule {
  file: string;
  selector: string;
  body: string;
}

/** Every innermost `selector { declarations }`, comments dropped. */
function rules(): Rule[] {
  return stylesheets(SRC).flatMap((path) => {
    const css = readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({
      file: relative(SRC, path),
      selector: m[1]!.trim().replace(/\s+/g, ' '),
      body: m[2]!,
    }));
  });
}

function declarations(property: string): { rule: Rule; value: string }[] {
  const pattern = new RegExp(`(?:^|;|\\s)${property}\\s*:\\s*([^;]+)`, 'g');
  return rules().flatMap((rule) =>
    [...rule.body.matchAll(pattern)].map((m) => ({ rule, value: m[1]!.trim() }))
  );
}

const isTokenBlock = (rule: Rule) => rule.selector === ':root';
const where = ({ rule, value }: { rule: Rule; value: string }) =>
  `${rule.file}: ${rule.selector} -> ${value}`;

describe('type scale', () => {
  /** Not interface text: lettering inside a 16px icon, a tab-order number
   * drawn on the page at page scale, and a signature specimen. */
  const ALLOWED = ['.ext-icon-initials', '.prepare-tab-badge', '.sign-style-sample'];

  it('defines five sizes, the ones macOS uses', () => {
    const root = rules().find((r) => r.file === 'styles/global.css' && isTokenBlock(r))!;
    const sizes = [...root.body.matchAll(/--fs-[a-z]+:\s*(\d+)px/g)].map((m) => Number(m[1]));
    expect(sizes).toEqual([11, 12, 13, 15, 20]);
  });

  it('sizes text by token or by em of its context, never a raw pixel value', () => {
    const raw = declarations('font-size').filter(
      (d) =>
        !isTokenBlock(d.rule) &&
        !/^var\(--fs-[a-z]+\)$/.test(d.value) &&
        !/^[\d.]+(em|%)$/.test(d.value) &&
        d.value !== 'inherit' &&
        !ALLOWED.some((s) => d.rule.selector.startsWith(s))
    );
    expect(raw.map(where)).toEqual([]);
  });
});

describe('corner radii', () => {
  /** Marks drawn on the page itself, at the page's scale: the reading
   * surface keeps its own geometry. */
  const PAGE_MARKS =
    /\.(text-selection|search-hit|reveal-mark|citation-hotspot|citation-preview-mark|annot-highlight|annot-note|annot-handle|annot-keyboard-target|form-field|form-check-face|form-widget|hover-span-target|equation-outline|mk-mark|prepare-field|prepare-tab-badge)(?![\w-])/;

  it('defines three radii', () => {
    const root = rules().find((r) => r.file === 'styles/global.css' && isTokenBlock(r))!;
    const radii = [...root.body.matchAll(/--radius-[a-z]+:\s*(\d+)px/g)].map((m) => Number(m[1]));
    expect(radii).toEqual([4, 6, 10]);
  });

  it('rounds corners by token, 50% for circles, or not at all', () => {
    const off = declarations('border-radius').filter(
      (d) =>
        !isTokenBlock(d.rule) &&
        !PAGE_MARKS.test(d.rule.selector) &&
        !d.value
          .split(/\s+/)
          .every((part) => /^(0|50%|inherit|var\(--(radius-[a-z]+|row-radius)\))$/.test(part))
    );
    expect(off.map(where)).toEqual([]);
  });
});

describe('cursors', () => {
  /** Real hyperlinks: links on the page, and link-styled text in prose that
   * takes you somewhere (another file, an item, an author, a citation). */
  const LINKS = [
    '.citation-hotspot',
    '.page-space.is-over-hover-span .text-layer',
    '.paper-panel-link',
    '.paper-verdict-link',
    '.author-link',
    '.ext-add-link',
    '.np-cite',
    '.np-xref',
  ];

  it('shows the pointing hand on links only', () => {
    const off = declarations('cursor')
      .filter((d) => d.value === 'pointer')
      .flatMap((d) => d.rule.selector.split(',').map((s) => s.trim()))
      .filter((s) => !LINKS.includes(s) && !/(^|\s)a(\[href[^\]]*\])?(:[\w-]+)*$/.test(s));
    expect(off).toEqual([]);
  });

  it('gives the chrome the arrow and fields the I-beam', () => {
    const all = rules();
    const base = all.find((r) => r.selector === 'html, body')!;
    expect(base.body).toMatch(/cursor:\s*default/);
    expect(base.body).toMatch(/overscroll-behavior:\s*none/);
    const fields = all.find(
      (r) => r.selector.endsWith("textarea, [contenteditable='true']") && /cursor/.test(r.body)
    )!;
    expect(fields.body).toMatch(/cursor:\s*text/);
    expect(fields.selector).toContain("[type='checkbox']");
  });
});
