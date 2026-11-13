import { describe, expect, it } from 'vitest';
import type { PaperFile } from '../../types/engine';
import { fileRows } from './fileRows';

const file = (name: string, duplicateOf?: string): PaperFile => ({
  path: `/l/${name}`,
  name,
  size: 1,
  mtimeMs: 1,
  fingerprint: null,
  missing: false,
  duplicateOf: duplicateOf ?? null,
});

describe('fileRows', () => {
  it('folds byte-identical copies into the row of the file shown', () => {
    const rows = fileRows([
      file('mot.pdf'),
      file('mot (1).pdf', '/l/mot.pdf'),
      file('2606.13614v1 (1).pdf', '/l/2606.13614v1.pdf'),
      file('2606.13614v1.pdf'),
      file('mot_tmlr.pdf'),
    ]);
    expect(rows.map((r) => [r.file.name, r.copies.map((c) => c.name)])).toEqual([
      ['mot.pdf', ['mot (1).pdf']],
      ['2606.13614v1.pdf', ['2606.13614v1 (1).pdf']],
      ['mot_tmlr.pdf', []],
    ]);
  });

  it('keeps a copy whose file is not listed as a row of its own', () => {
    const rows = fileRows([file('a.pdf'), file('b.pdf', '/l/gone.pdf')]);
    expect(rows.map((r) => r.file.name)).toEqual(['a.pdf', 'b.pdf']);
  });
});
