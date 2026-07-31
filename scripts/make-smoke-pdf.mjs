#!/usr/bin/env node

import { mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve } from 'node:path';

const args = process.argv.slice(2).filter((arg) => arg !== '--');
const withQuery = args.includes('--with-query');
const outputArg = args.find((arg) => !arg.startsWith('--'));
const output = resolve(outputArg ?? join(tmpdir(), 'speedyf-smoke.pdf'));
const pageCount = 12;
const objects = new Map();

const pdfString = (value) =>
  value.replaceAll('\\', '\\\\').replaceAll('(', '\\(').replaceAll(')', '\\)');

objects.set(1, '<< /Type /Catalog /Pages 2 0 R >>');
objects.set(3, '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>');

const pageIds = [];
for (let index = 0; index < pageCount; index += 1) {
  const pageId = 4 + index * 2;
  const contentId = pageId + 1;
  pageIds.push(`${pageId} 0 R`);

  const wide = index === 5;
  const width = wide ? 1000 : 612;
  const height = wide ? 500 : 792;
  const rotation = index === 2 ? 90 : index === 8 ? 270 : 0;
  const rotationEntry = rotation === 0 ? '' : ` /Rotate ${rotation}`;
  const title = `SpeedyF smoke page ${index + 1}`;
  const token = index % 2 === 0 ? 'ALPHA searchable phrase' : 'BETA searchable phrase';
  const content = [
    'q',
    '0.92 0.95 1 rg',
    `54 ${Math.max(90, height - 330)} ${Math.max(200, width - 108)} 180 re f`,
    '0.12 0.23 0.42 RG',
    '2 w',
    `54 ${Math.max(90, height - 330)} ${Math.max(200, width - 108)} 180 re S`,
    'Q',
    'BT',
    '/F1 26 Tf',
    `72 ${height - 92} Td`,
    `(${pdfString(title)}) Tj`,
    '0 -42 Td',
    '/F1 14 Tf',
    `(${pdfString(`${token}; source rotation ${rotation} degrees.`)}) Tj`,
    '0 -26 Td',
    `(${pdfString('Select this sentence, zoom, annotate, edit pages, save, and reopen.')}) Tj`,
    '0 -26 Td',
    `(${pdfString(`Fixture line ${index + 1}: cafe "quotes" search test.`)}) Tj`,
    'ET',
    '',
  ].join('\n');

  objects.set(
    pageId,
    [
      '<< /Type /Page',
      '/Parent 2 0 R',
      `/MediaBox [0 0 ${width} ${height}]${rotationEntry}`,
      '/Resources << /Font << /F1 3 0 R >> >>',
      `/Contents ${contentId} 0 R`,
      '>>',
    ].join(' ')
  );
  objects.set(
    contentId,
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}endstream`
  );
}

objects.set(2, `<< /Type /Pages /Kids [${pageIds.join(' ')}] /Count ${pageCount} >>`);

const maxObjectId = Math.max(...objects.keys());
const chunks = ['%PDF-1.7\n%SpeedyF smoke fixture\n'];
const offsets = new Array(maxObjectId + 1).fill(0);
let byteOffset = Buffer.byteLength(chunks[0]);

for (let id = 1; id <= maxObjectId; id += 1) {
  const body = objects.get(id);
  if (!body) throw new Error(`missing PDF object ${id}`);
  offsets[id] = byteOffset;
  const chunk = `${id} 0 obj\n${body}\nendobj\n`;
  chunks.push(chunk);
  byteOffset += Buffer.byteLength(chunk);
}

const xrefOffset = byteOffset;
const xref = [
  `xref\n0 ${maxObjectId + 1}\n`,
  '0000000000 65535 f \n',
  ...offsets.slice(1).map((offset) => `${String(offset).padStart(10, '0')} 00000 n \n`),
  `trailer\n<< /Size ${maxObjectId + 1} /Root 1 0 R >>\n`,
  `startxref\n${xrefOffset}\n%%EOF\n`,
].join('');
chunks.push(xref);

await mkdir(dirname(output), { recursive: true });
await writeFile(output, chunks.join(''), 'binary');
if (withQuery) {
  const queryPath =
    extname(output).toLowerCase() === '.pdf' ? output.slice(0, -4) + '.query' : `${output}.query`;
  await writeFile(queryPath, 'ALPHA\n', 'utf8');
}
console.info(output);
