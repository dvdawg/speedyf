/** Kept comments re-read from where they are (`piecesUnderQuads`), for
 * looking at: each comment of a real review, its stored quote, and what
 * the backfill makes of it. Reads page dumps (`quote_eval_dump`, pages
 * numbered as in the PDF) and a review's JSON; skipped unless BACKFILL_DUMP
 * and BACKFILL_REVIEW name them; writes backfill.txt beside the dumps.
 *
 *   BACKFILL_DUMP=/tmp/dump BACKFILL_REVIEW=/tmp/review.json \
 *     pnpm vitest run src/features/markup/backfillEval.test.ts
 */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it } from 'vitest';
import { marginNumbers } from '../../lib/text/marginNumbers';
import { piecesUnderQuads } from '../../lib/text/piecesUnder';
import { quotePieces, trimDump, type Dump } from './quoteHarness';
import { quadBox } from './quoteBackfill';
import type { MarkupDoc } from './types';

const DIR = process.env.BACKFILL_DUMP;
const REVIEW = process.env.BACKFILL_REVIEW;
/** Comment ids to keep as the fixture backfillReal.test.ts checks. */
const KEEP = process.env.BACKFILL_KEEP?.split(',') ?? [];

describe.skipIf(!DIR || !REVIEW)('backfill on a real review', () => {
  it('prints each comment before and after', async () => {
    const dumps = readdirSync(DIR!)
      .filter((f) => /^page\d+\.json$/.test(f))
      .map((f) => JSON.parse(readFileSync(join(DIR!, f), 'utf8')) as Dump);
    const review = JSON.parse(readFileSync(REVIEW!, 'utf8')) as MarkupDoc;
    const report: string[] = [];
    const kept: unknown[] = [];
    for (const dump of dumps) {
      const flagged = marginNumbers(dump.runs)
        .map((m, i) => (m ? dump.runs[i]!.text : null))
        .filter((t) => t !== null);
      report.push(`page ${dump.page}: ${flagged.length} margin numbers: ${flagged.join(' ')}`);
    }
    for (const c of review.comments) {
      const dump = dumps.find((d) => d.page === c.anchor.page);
      if (!dump || !c.anchor.quads) continue;
      const H = dump.size[1];
      const pieces = piecesUnderQuads(
        dump.runs,
        c.anchor.quads.map(quadBox),
        dump.displays.map(({ bbox: [x, y, w, h], number }) => ({ x, y: H - y - h, w, h, number })),
        0,
        c.anchor.quote
      );
      const q = await quotePieces(dump, pieces);
      if (KEEP.includes(c.id)) {
        const at = pieces.map((p) =>
          dump.runs.findIndex((r) => r.start === p.run.start && r.x === p.run.x)
        );
        const first = Math.max(0, Math.min(...at) - 2);
        const last = Math.min(dump.runs.length - 1, Math.max(...at) + 2);
        kept.push({
          dump: trimDump(dump, first, last),
          comment: c,
          plain: q.plain,
          after: q.after,
        });
      }
      report.push(
        [
          `=== ${c.id} ${c.kind} p.${c.anchor.page + 1}`,
          `stored: ${c.anchor.quote}`,
          `plain:  ${q.plain}`,
          `tex:    ${q.afterTrusted ? '' : '[none] '}${q.after}`,
          ...q.misread.map((m) => `  misread: ${m}`),
        ].join('\n')
      );
    }
    writeFileSync(join(DIR!, 'backfill.txt'), `${report.join('\n\n')}\n`);
    if (kept.length > 0) writeFileSync(join(DIR!, 'backfill-real.json'), JSON.stringify(kept));
  }, 120_000);
});
