/** Quotes built from real pages, for looking at: what a comment on a
 * selection mixing prose, inline math and a display reads as, the old way
 * (the whole selection through Math to LaTeX at once) and the new
 * (`planQuote`: prose as the page gives it, each formula and display
 * converted on its own). Reads the dumps `quote_eval_dump` writes
 * (src-tauri/src/engine/quote_eval.rs); skipped unless QUOTE_DUMP names
 * their directory; writes what it finds to quotes.txt there. Run with:
 *
 *   QUOTE_DUMP=/tmp/quotes pnpm vitest run src/features/markup/quoteEval.test.ts
 */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'vitest';
import { buildTextModel } from '../../lib/text/textHit';
import { quote, trimDump, type Dump } from './quoteHarness';

const DIR = process.env.QUOTE_DUMP;
/** Cases to keep as the fixture quoteReal.test.ts checks:
 * "file.pdf#page#first#last;..." */
const KEEP = process.env.QUOTE_KEEP;

describe.skipIf(!DIR)('quotes from real pages', () => {
  it('prints them', async () => {
    const report: string[] = [];
    const files = readdirSync(DIR!).filter((f) => /^page\d+\.json$/.test(f));
    for (const file of files) {
      const dump = JSON.parse(readFileSync(join(DIR!, file), 'utf8')) as Dump;
      const H = dump.size[1];
      const model = buildTextModel(
        dump.runs,
        undefined,
        dump.displays.map(({ bbox: [x, y, w, h], number }) => ({ x, y: H - y - h, w, h, number }))
      );
      // around each display: the line before it to the line after
      for (const d of model.displays.slice(0, 2)) {
        const first =
          model.lines[Math.max(0, (model.lineOfRun[d.first - 1] ?? 0) - 1)]?.first ?? d.first;
        const after = model.lineOfRun[d.last + 1];
        const last = after === undefined ? d.last : (model.lines[after]?.last ?? d.last);
        const q = await quote(dump, first, last);
        report.push(
          [
            `=== ${dump.path.split('/').at(-1)} p.${dump.page + 1} runs ${first}-${last}`,
            `plain:  ${q.plain}`,
            `before: ${q.beforeTrusted ? '' : '[untrusted] '}${q.before}`,
            `after:  ${q.afterTrusted ? '' : '[untrusted] '}${q.after}`,
          ].join('\n')
        );
      }
    }
    writeFileSync(join(DIR!, 'quotes.txt'), `${report.join('\n\n')}\n`);
    if (KEEP) {
      const dumps = files.map((f) => JSON.parse(readFileSync(join(DIR!, f), 'utf8')) as Dump);
      const cases = [];
      for (const spec of KEEP.split(';').filter(Boolean)) {
        const [name, page, first, last] = spec.split('#');
        const dump = dumps.find((d) => d.path.endsWith(name!) && d.page === Number(page));
        if (!dump) continue;
        const trimmed = trimDump(dump, Number(first), Number(last));
        const q = await quote(trimmed, 0, trimmed.runs.length - 1);
        cases.push({ dump: trimmed, before: q.before, after: q.after });
      }
      writeFileSync(join(DIR!, 'quotes-real.json'), JSON.stringify(cases));
    }
  }, 120_000);
});
