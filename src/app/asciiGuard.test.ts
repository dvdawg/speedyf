/** The app's own text is plain ASCII: no Unicode math symbols, arrows,
 * center dots, curly quotes, dashes or key glyphs. Math goes through
 * MathText as TeX (`$\geq 2$`), buttons use the SVG icons, shortcuts read
 * "Cmd+Shift+C". This scans every string literal, template chunk and JSX text
 * in src (comments, tests and test fixtures are not app text) and the
 * `content:` values in the stylesheet. */
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const ROOT = join(process.cwd(), 'src');

/** Files allowed to hold non-ASCII literals, and why. */
const ALLOW = new Map<string, string>([
  [
    'lib/text/smartCopy.ts',
    'matches ligatures and Unicode hyphens in text copied out of PDFs, to turn them into ASCII; never displayed',
  ],
]);

/** HTML entities that would bring the same glyphs back by another door. */
const ENTITY_OK = new Set(['&nbsp;', '&amp;', '&lt;', '&gt;', '&quot;', '&apos;']);

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) sources(path, out);
    else if (
      /\.(ts|tsx)$/.test(entry.name) &&
      !/\.test\.(ts|tsx)$/.test(entry.name) &&
      entry.name !== 'testFixtures.ts'
    ) {
      out.push(path);
    }
  }
  return out;
}

const TEXT_KINDS = new Set([
  ts.SyntaxKind.StringLiteral,
  ts.SyntaxKind.NoSubstitutionTemplateLiteral,
  ts.SyntaxKind.TemplateHead,
  ts.SyntaxKind.TemplateMiddle,
  ts.SyntaxKind.TemplateTail,
  ts.SyntaxKind.JsxText,
  ts.SyntaxKind.RegularExpressionLiteral,
]);

function offenders(path: string): string[] {
  const text = readFileSync(path, 'utf8');
  const kind = path.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
  const file = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, kind);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (TEXT_KINDS.has(node.kind)) {
      const literal = node.getText(file);
      const entities =
        node.kind === ts.SyntaxKind.JsxText ? (literal.match(/&[a-z]+;/g) ?? []) : [];
      if (/[^\t\n\r\x20-\x7e]/.test(literal) || entities.some((e) => !ENTITY_OK.has(e))) {
        const { line } = file.getLineAndCharacterOfPosition(node.getStart(file));
        found.push(`${relative(ROOT, path)}:${line + 1}: ${literal.trim().slice(0, 80)}`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe('app text is ASCII', () => {
  it('has no non-ASCII characters in string literals or JSX text', () => {
    const found = sources(ROOT)
      .filter((path) => !ALLOW.has(relative(ROOT, path).split('\\').join('/')))
      .flatMap(offenders);
    expect(found).toEqual([]);
  });

  it('has no non-ASCII characters in stylesheet content values', () => {
    const css = readFileSync(join(ROOT, 'styles', 'global.css'), 'utf8').replace(
      /\/\*[\s\S]*?\*\//g,
      ''
    );
    const values = css.match(/(?<![-\w])content\s*:[^;]*;/g) ?? [];
    expect(values.filter((value) => /[^\t\n\r\x20-\x7e]|\\[0-9a-f]{2,6}/i.test(value))).toEqual([]);
  });

  it('catches what it is meant to catch', () => {
    const probe = ts.createSourceFile(
      'probe.tsx',
      "// a comment with \u2265 is fine\nconst a = 'x \u00b7 y';\nconst b = <p>older \u2190</p>;",
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX
    );
    const hits: string[] = [];
    const visit = (node: ts.Node) => {
      if (TEXT_KINDS.has(node.kind) && /[^\t\n\r\x20-\x7e]/.test(node.getText(probe))) {
        hits.push(node.getText(probe));
      }
      ts.forEachChild(node, visit);
    };
    visit(probe);
    expect(hits).toEqual(["'x \u00b7 y'", 'older \u2190']);
  });
});
