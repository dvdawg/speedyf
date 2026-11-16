/** Selection on real two-column pages, in the stream's order and in reading
 * order: the dumps `reading_order_eval` writes
 * (src-tauri/src/engine/reading_order_eval.rs). Skipped unless ORDER_DUMP
 * names their directory. For each page, a drag from a line near the bottom
 * of the left column to one near the top of the right is resolved the way
 * the text layer resolves it (`hitTest`, a range of runs, `smartCopyText`),
 * and every prose line is triple-clicked (`paragraphRunRange`). */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import { paragraphRunRange } from './paragraph';
import { smartCopyText } from './smartCopy';
import { buildTextModel, hitTest, type TextModel, type TextPosition } from './textHit';

const DIR = process.env.ORDER_DUMP;

interface Dump {
  path: string;
  page: number;
  size: [number, number];
  columns: [number, number][];
  runs: TextRunDto[];
  order: number[];
}

const centreX = (r: TextRunDto) => r.x + r.w / 2;
const centreY = (r: TextRunDto) => r.y + r.h / 2;

function columnOf(columns: [number, number][], r: TextRunDto): number {
  const x = centreX(r);
  let best = 0;
  let bestD = Infinity;
  columns.forEach(([a, b], i) => {
    const d = Math.max(0, a - x, x - b);
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  });
  return best;
}

/** The text a drag from `a` to `b` selects, and the runs it takes. */
function drag(model: TextModel, a: TextPosition, b: TextPosition) {
  const [from, to] = a.run < b.run || (a.run === b.run && a.offset <= b.offset) ? [a, b] : [b, a];
  const pieces = [];
  for (let r = from.run; r <= to.run; r += 1) {
    const run = model.runs[r]!;
    if (model.margin[r]) continue;
    const s = r === from.run ? from.offset : 0;
    const e = r === to.run ? to.offset : run.text.length;
    if (e > s) pieces.push({ ...run, text: run.text.slice(s, e), page: 0, order: r });
  }
  return { text: smartCopyText(pieces), runs: pieces };
}

describe.skipIf(!DIR)('reading order on real pages', () => {
  it('drags from the left column into the right, and triple-clicks', () => {
    const files = readdirSync(DIR!).filter((f) => f.endsWith('.json'));
    type Three = [number, number, number];
    const totals = {
      before: [0, 0, 0] as Three,
      after: [0, 0, 0] as Three,
      pages: 0,
      mixedBefore: 0,
      linesBefore: [0, 0, 0] as Three,
      linesAfter: [0, 0, 0] as Three,
      mixedAfter: 0,
      clicks: 0,
    };
    const report: string[] = [];
    for (const file of files.sort()) {
      const dump = JSON.parse(readFileSync(join(DIR!, file), 'utf8')) as Dump;
      if (dump.columns.length !== 2) continue;
      const H = dump.size[1];
      const stream = dump.runs;
      const reading = dump.order.map((i) => stream[i]!);
      const after = buildTextModel(reading);
      const before = buildTextModel(stream);
      const [left, right] = dump.columns as [[number, number], [number, number]];
      const prose = (m: TextModel, c: [number, number]) =>
        m.lines.filter(
          (l) =>
            l.minX >= c[0] - 2 &&
            l.maxX <= c[1] + 2 &&
            l.maxX - l.minX >= 0.7 * (c[1] - c[0]) &&
            l.bottom > 0.1 * H &&
            l.top < 0.9 * H
        );
      const lefts = prose(after, left).sort((a, b) => a.top - b.top);
      const rights = prose(after, right).sort((a, b) => b.top - a.top);
      if (lefts.length < 6 || rights.length < 6) continue;
      const from = lefts[2]!;
      const to = rights[2]!;
      const p = { x: from.minX + 1, y: (from.top + from.bottom) / 2 };
      const q = { x: to.minX + 0.6 * (to.maxX - to.minX), y: (to.top + to.bottom) / 2 };
      // What belongs: the left column from the press down, the right
      // column down to the release - and nothing else.
      const belongs = (r: TextRunDto) => {
        const c = columnOf(dump.columns, r);
        const y = centreY(r);
        return (
          r.w < 0.95 * (right[1] - left[0]) &&
          ((c === 0 && y <= from.top && y > 0.06 * H) ||
            (c === 1 && y >= to.bottom && y < 0.94 * H))
        );
      };
      totals.pages += 1;
      const lines: string[] = [];
      for (const [name, model] of [
        ['before', before],
        ['after', after],
      ] as const) {
        const a = hitTest(model, p)!;
        const b = hitTest(model, q)!;
        const { text, runs } = drag(model, a, b);
        const foreign = runs.filter((r) => !belongs(r)).length;
        // Prose lines between the two ends the drag left out.
        const taken = new Set(runs.map((r) => r.order));
        let missing = 0;
        const lost: string[] = [];
        const between = [
          ...prose(model, left).filter((l) => l.top <= from.top + 0.5),
          ...prose(model, right).filter((l) => l.bottom >= to.bottom - 0.5),
        ];
        for (const l of between) {
          for (let r = l.first; r <= l.last; r += 1) {
            if (!model.margin[r] && belongs(model.runs[r]!) && !taken.has(r)) {
              missing += 1;
              lost.push(
                `${model.runs[r]!.text}@${Math.round(model.runs[r]!.x)},${Math.round(model.runs[r]!.y)}`
              );
            }
          }
        }
        totals[name][0] += foreign;
        totals[name][1] += runs.length;
        totals[name][2] += missing;
        lines.push(
          `  ${name}: ${runs.length} runs, ${foreign} foreign, ${missing} missing: ${JSON.stringify(text.slice(0, 90))} ... ${JSON.stringify(text.slice(-90))}`,
          `    foreign: ${JSON.stringify(
            runs
              .filter((r) => !belongs(r))
              .map((r) => `${r.text}@${Math.round(r.x)},${Math.round(r.y)}`)
              .join(' ')
              .slice(0, 300)
          )}`,
          `    missing: ${JSON.stringify(lost.join(' ').slice(0, 300))}`
        );
      }
      // Triple-click every prose line of either column.
      const page = {
        mixedBefore: [0, 0] as [number, number],
        mixedAfter: [0, 0] as [number, number],
      };
      for (const [model, key] of [
        [before, 'mixedBefore'],
        [after, 'mixedAfter'],
      ] as const) {
        for (const line of [...prose(model, left), ...prose(model, right)]) {
          const range = paragraphRunRange(model.runs, line.first);
          if (!range) continue;
          if (key === 'mixedAfter') totals.clicks += 1;
          const columns = new Set<number>();
          for (let r = range[0]; r <= range[1]; r += 1) {
            if (!model.margin[r]) columns.add(columnOf(dump.columns, model.runs[r]!));
          }
          if (columns.size > 1) totals[key] += 1;
          // how many lines the paragraph it selects holds
          const held = new Set<number>();
          for (let r = range[0]; r <= range[1]; r += 1) held.add(model.lineOfRun[r] ?? -1);
          held.delete(-1);
          const tally = key === 'mixedBefore' ? totals.linesBefore : totals.linesAfter;
          tally[0] += 1;
          tally[1] += held.size;
          if (held.size === 1) tally[2] += 1;
          page[key][0] += 1;
          page[key][1] += held.size;
        }
      }
      lines.push(
        `  triple-click: ${page.mixedBefore[1]}/${page.mixedBefore[0]} lines a paragraph before, ${page.mixedAfter[1]}/${page.mixedAfter[0]} after`
      );
      report.push(`${file}`, ...lines);
    }
    report.push(JSON.stringify(totals));
    writeFileSync(join(DIR!, 'report.txt'), report.join('\n'));
    expect(totals.pages).toBeGreaterThan(0);
  }, 600_000);
});
