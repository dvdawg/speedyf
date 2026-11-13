/** The same rule as `asciiGuard.test.ts`, past `src/`: what SpeedyF shows a
 * person is plain ASCII. That covers the extensions (built-in and samples),
 * their manifests, the Markdown docs and the strings the Rust backend sends
 * to the UI.
 *
 * Code that must recognise Unicode in its input (a Unicode -> LaTeX table, a
 * dash or quote class in a parser) keeps it between a marker pair:
 *
 *   // ascii-guard: data begin
 *   ...
 *   // ascii-guard: data end
 *
 * (`<!-- ascii-guard: data begin -->` in Markdown). Comments are not shown to
 * anyone and are skipped in code; Rust test modules (`#[cfg(test)]`) are
 * fixtures, not UI text, and are skipped too. */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

const REPO = process.cwd();

const NON_ASCII = /[^\t\n\r\x20-\x7e]/;

/** Files allowed to hold non-ASCII text outside marked regions, and why.
 * Kept empty on purpose: mark the data region instead. */
const ALLOW = new Map<string, string>([]);

function walk(dir: string, test: (name: string) => boolean, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'node_modules' && entry.name !== 'target') walk(path, test, out);
    } else if (test(entry.name)) out.push(path);
  }
  return out;
}

const rel = (path: string) => relative(REPO, path).split('\\').join('/');

function exists(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

// ---------- marked data regions ----------

const BEGIN = /ascii-guard: data begin/;
const END = /ascii-guard: data end/;

/** 1-based line numbers inside a marked region (the markers included). An
 * unclosed region is an error, so a stray `begin` cannot hide a whole file. */
export function dataLines(text: string): Set<number> {
  const lines = text.split('\n');
  const out = new Set<number>();
  let open = 0;
  lines.forEach((line, i) => {
    if (BEGIN.test(line)) {
      if (open) throw new Error(`nested "ascii-guard: data begin" at line ${i + 1}`);
      open = i + 1;
    }
    if (open) out.add(i + 1);
    if (END.test(line)) {
      if (!open) throw new Error(`"ascii-guard: data end" without a begin at line ${i + 1}`);
      open = 0;
    }
  });
  if (open) throw new Error(`"ascii-guard: data begin" at line ${open} is never closed`);
  return out;
}

// ---------- JavaScript ----------

const JS_TEXT = new Set([
  ts.SyntaxKind.StringLiteral,
  ts.SyntaxKind.NoSubstitutionTemplateLiteral,
  ts.SyntaxKind.TemplateHead,
  ts.SyntaxKind.TemplateMiddle,
  ts.SyntaxKind.TemplateTail,
  ts.SyntaxKind.RegularExpressionLiteral,
  // An object key written bare (`{ ⁿ: 'n' }`) is as much text as a quoted one.
  ts.SyntaxKind.Identifier,
]);

/** Non-ASCII literals in JavaScript source, outside marked regions. */
export function jsOffenders(name: string, text: string): string[] {
  const skip = dataLines(text);
  const file = ts.createSourceFile(name, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (JS_TEXT.has(node.kind)) {
      const literal = node.getText(file);
      const { line } = file.getLineAndCharacterOfPosition(node.getStart(file));
      if (NON_ASCII.test(literal) && !skip.has(line + 1)) {
        found.push(`${name}:${line + 1}: ${literal.trim().slice(0, 80)}`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

// ---------- JSON manifests ----------

/** Every string in a manifest, keys and values. */
export function jsonOffenders(name: string, text: string): string[] {
  const found: string[] = [];
  const visit = (value: unknown, path: string) => {
    if (typeof value === 'string') {
      if (NON_ASCII.test(value)) found.push(`${name}:${path}: ${value.slice(0, 80)}`);
    } else if (Array.isArray(value)) {
      value.forEach((v, i) => visit(v, `${path}[${i}]`));
    } else if (value && typeof value === 'object') {
      for (const [k, v] of Object.entries(value)) {
        if (NON_ASCII.test(k)) found.push(`${name}:${path}: key ${k}`);
        visit(v, `${path}.${k}`);
      }
    }
  };
  visit(JSON.parse(text), '$');
  return found;
}

// ---------- Markdown ----------

const MD_BEGIN = /<!-- ascii-guard: data begin -->/;
const MD_END = /<!-- ascii-guard: data end -->/;

/** Every line of a Markdown file, outside marked regions. */
export function mdOffenders(name: string, text: string): string[] {
  const found: string[] = [];
  let open = false;
  text.split('\n').forEach((line, i) => {
    if (MD_BEGIN.test(line)) open = true;
    if (!open && NON_ASCII.test(line)) found.push(`${name}:${i + 1}: ${line.trim().slice(0, 80)}`);
    if (MD_END.test(line)) open = false;
  });
  return found;
}

// ---------- Rust ----------

interface RustLiteral {
  line: number;
  text: string;
}

/** String and char literals of a Rust file, with `#[cfg(test)]` items left
 * out. A small lexer: comments (nested block comments too), strings, byte
 * and raw strings, char literals and lifetimes. */
export function rustLiterals(src: string): RustLiteral[] {
  const out: RustLiteral[] = [];
  let i = 0;
  let line = 1;
  /** Brace depth below which a `#[cfg(test)]` item ends, or -1. */
  let skipUntilDepth = -1;
  let depth = 0;
  /** A `#[cfg(test)]` was read and its item has not started yet. */
  let pendingTest = false;

  const advance = (to: number) => {
    for (let k = i; k < to && k < src.length; k++) if (src[k] === '\n') line++;
    i = to;
  };
  const push = (start: number, end: number, startLine: number) => {
    if (skipUntilDepth < 0) out.push({ line: startLine, text: src.slice(start, end) });
  };

  while (i < src.length) {
    const c = src[i];
    const next = src[i + 1];
    if (c === '/' && next === '/') {
      const end = src.indexOf('\n', i);
      advance(end < 0 ? src.length : end);
      continue;
    }
    if (c === '/' && next === '*') {
      let level = 0;
      let k = i;
      while (k < src.length) {
        if (src[k] === '/' && src[k + 1] === '*') {
          level++;
          k += 2;
        } else if (src[k] === '*' && src[k + 1] === '/') {
          level--;
          k += 2;
          if (level === 0) break;
        } else k++;
      }
      advance(k);
      continue;
    }
    // Raw strings: r"..", r#".."#, br#".."#, cr#".."#.
    const raw = /^(?:b|c)?r(#*)"/.exec(src.slice(i, i + 260));
    if (raw && !/[A-Za-z0-9_]/.test(src[i - 1] ?? '')) {
      const close = `"${raw[1]}`;
      const end = src.indexOf(close, i + raw[0].length);
      const stop = end < 0 ? src.length : end + close.length;
      const startLine = line;
      push(i, stop, startLine);
      advance(stop);
      continue;
    }
    if (
      c === '"' ||
      ((c === 'b' || c === 'c') && next === '"' && !/[A-Za-z0-9_]/.test(src[i - 1] ?? ''))
    ) {
      let k = c === '"' ? i + 1 : i + 2;
      while (k < src.length && src[k] !== '"') k += src[k] === '\\' ? 2 : 1;
      const startLine = line;
      push(i, k + 1, startLine);
      advance(k + 1);
      continue;
    }
    if (c === "'") {
      // A char literal, or a lifetime / label.
      let k = i + 1;
      if (src[k] === '\\') {
        k += 2;
        while (k < src.length && src[k] !== "'") k++;
        push(i, k + 1, line);
        advance(k + 1);
        continue;
      }
      const cp = src.codePointAt(k);
      const len = cp !== undefined && cp > 0xffff ? 2 : 1;
      if (src[k + len] === "'") {
        push(i, k + len + 1, line);
        advance(k + len + 1);
        continue;
      }
      advance(i + 1);
      continue;
    }
    if (c === '#' && src.startsWith('#[cfg(test)]', i)) {
      if (skipUntilDepth < 0) pendingTest = true;
      advance(i + '#[cfg(test)]'.length);
      continue;
    }
    if (c === '{') {
      if (pendingTest) {
        skipUntilDepth = depth;
        pendingTest = false;
      }
      depth++;
    } else if (c === '}') {
      depth--;
      if (skipUntilDepth >= 0 && depth === skipUntilDepth) skipUntilDepth = -1;
    } else if (c === ';' && pendingTest) {
      // `#[cfg(test)] mod tests;` or a `use`: an item with no body.
      pendingTest = false;
    }
    advance(i + 1);
  }
  return out;
}

/** `mod name;` declarations behind `#[cfg(test)]`: whole files of tests. */
function rustTestModules(src: string): string[] {
  return [
    ...src.matchAll(
      /#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g
    ),
  ].map((m) => m[1] as string);
}

/** Non-ASCII string literals in Rust source, outside tests and marked
 * regions. */
export function rustOffenders(name: string, text: string): string[] {
  const skip = dataLines(text);
  return rustLiterals(text)
    .filter((lit) => NON_ASCII.test(lit.text) && !skip.has(lit.line))
    .map((lit) => `${name}:${lit.line}: ${lit.text.slice(0, 80)}`);
}

// ---------- the files ----------

const EXTENSION_DIRS = ['extensions', 'examples/extensions'].map((d) => join(REPO, d));

const DOCS = [
  'README.md',
  'EXTENSIONS.md',
  'extension_api.md',
  'projects.md',
  'CONTRIBUTING.md',
  'bench/README.md',
]
  .map((p) => join(REPO, p))
  .filter(exists)
  .concat(EXTENSION_DIRS.flatMap((d) => walk(d, (n) => n.endsWith('.md'))));

function rustFiles(): string[] {
  const root = join(REPO, 'src-tauri', 'src');
  const files = walk(root, (n) => n.endsWith('.rs'));
  const testFiles = new Set<string>();
  for (const path of files) {
    const dir =
      path.endsWith('mod.rs') || path.endsWith('lib.rs') || path.endsWith('main.rs')
        ? join(path, '..')
        : path.slice(0, -3);
    for (const mod of rustTestModules(readFileSync(path, 'utf8'))) {
      testFiles.add(join(dir, `${mod}.rs`));
      testFiles.add(join(dir, mod, 'mod.rs'));
    }
  }
  return files.filter((p) => !testFiles.has(p));
}

function scan(paths: string[], check: (name: string, text: string) => string[]): string[] {
  return paths
    .filter((p) => !ALLOW.has(rel(p)))
    .flatMap((p) => check(rel(p), readFileSync(p, 'utf8')));
}

describe('text beyond src/ is ASCII', () => {
  it('extension workers: string literals and template chunks', () => {
    const files = EXTENSION_DIRS.flatMap((d) => walk(d, (n) => n.endsWith('.js')));
    expect(files.length).toBeGreaterThan(0);
    expect(scan(files, jsOffenders)).toEqual([]);
  });

  it('extension manifests: every string', () => {
    const files = EXTENSION_DIRS.flatMap((d) => walk(d, (n) => n === 'speedyf.json'));
    expect(files.length).toBeGreaterThan(0);
    expect(scan(files, jsonOffenders)).toEqual([]);
  });

  it('Markdown docs: the full text', () => {
    expect(DOCS.length).toBeGreaterThan(5);
    expect(scan(DOCS, mdOffenders)).toEqual([]);
  });

  it('Rust backend: string literals outside tests', () => {
    const files = rustFiles();
    expect(files.length).toBeGreaterThan(0);
    expect(scan(files, rustOffenders)).toEqual([]);
  });
});

describe('the guard catches what it is meant to catch', () => {
  it('JavaScript: a literal, not a comment, not a marked region', () => {
    const src = [
      '// a comment with \u2014 is fine',
      "const a = 'x \u00b7 y';",
      'const b = `older \u2190 ${a}`;',
      '// ascii-guard: data begin',
      "const TABLE = { '\u2014': '-', \u207f: 'n' };",
      '// ascii-guard: data end',
      'const c = { \u207f: 1 };',
    ].join('\n');
    expect(jsOffenders('probe.js', src).map((s) => s.split(':')[1])).toEqual(['2', '3', '7']);
  });

  it('manifests: values and nested strings', () => {
    const src = JSON.stringify({ name: 'ok', commands: [{ title: 'Copy \u2026' }] });
    expect(jsonOffenders('speedyf.json', src)).toEqual([
      'speedyf.json:$.commands[0].title: Copy \u2026',
    ]);
  });

  it('Markdown: any line, unless marked as data', () => {
    const src = [
      '# Title',
      'a \u2192 b',
      '<!-- ascii-guard: data begin -->',
      '    caf\u00e9',
      '<!-- ascii-guard: data end -->',
      '```',
      'x \u2265 1',
      '```',
    ].join('\n');
    expect(mdOffenders('doc.md', src).map((s) => s.split(':')[1])).toEqual(['2', '7']);
  });

  it('Rust: strings and chars, not comments, tests, lifetimes or marked data', () => {
    const src = [
      '// a comment \u2014 fine',
      '/* block /* nested \u2192 */ still comment \u2014 */',
      "fn f<'a>(s: &'a str) -> String {",
      '    let e = "bad \u2026";',
      "    let ch = '\u2014';",
      '    let raw = r#"raw \u00d7 "quoted""#;',
      '    // ascii-guard: data begin',
      "    let t = [('\u2014', '-')];",
      '    // ascii-guard: data end',
      '    format!("{e}")',
      '}',
      '#[cfg(test)]',
      'mod tests {',
      '    fn t() { let _ = "caf\u00e9 { \u2014"; }',
      '}',
      'fn g() -> &\'static str { "after \u00b7 tests" }',
    ].join('\n');
    expect(rustOffenders('probe.rs', src).map((s) => s.split(':')[1])).toEqual([
      '4',
      '5',
      '6',
      '16',
    ]);
  });

  it('refuses an unclosed data region', () => {
    expect(() => dataLines('// ascii-guard: data begin\nconst a = 1;')).toThrow(/never closed/);
  });
});
