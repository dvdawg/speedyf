/** Reading a comment's quote as text with its math typeset: the selection
 * taken apart (`planQuote`), each inline formula read on its own by the
 * math converter (Math to LaTeX, when it is enabled) and each display by
 * the same path Send Equation uses, every reading checked on its own
 * (`quoteTexCheck`), and the quote assembled from what passed, the page's
 * plain text standing in for anything that did not (`fillQuote`).
 *
 * Then, quietly, an exact upgrade: the whole passage as the paper's own
 * TeX source writes it, when the converter finds it there for certain;
 * failing that, each display from the source. That goes online only
 * through the converter, whose enabling is the user's consent to it. */
import {
  convertEquation,
  convertSelection,
  registerSnapshot,
  type ConvertMode,
} from '../extensions/extensionStore';
import { equationToken } from '../extensions/editorHost';
import {
  fillQuote,
  oneFormula,
  prosy,
  type DisplayPart,
  type QuotePlan,
} from '../../lib/text/quoteParts';
import { plausibleQuoteTex, texParses, trustedFormula } from './quoteTexCheck';

type Rect = [number, number, number, number];

export interface QuoteContext {
  docId: number;
  /** a reader page's height in points, to turn y-up boxes top-left */
  pageHeight: (page: number) => number | undefined;
}

const trusted = trustedFormula;

function topLeft([x, y, w, h]: readonly number[], height: number): Rect {
  return [x!, height - y! - h!, w!, h!];
}

/** A display's LaTeX from the converter, its number as its tag. */
async function readDisplay(
  part: DisplayPart,
  ctx: QuoteContext,
  mode: ConvertMode
): Promise<string | null> {
  const height = ctx.pageHeight(part.page);
  if (height === undefined || prosy(part.text)) return null;
  const ref = {
    docId: ctx.docId,
    page: part.page,
    bbox: topLeft(part.block, height),
    number: part.number,
  };
  const answer = await convertEquation(equationToken(ref), ref, mode).catch(() => null);
  let latex = answer?.latex.trim() ?? '';
  if (!latex) return null;
  if (part.number && !latex.includes('\\tag')) latex = `${latex} \\tag{${part.number}}`;
  return trusted(latex, true);
}

/** Read `plan` and hand each better quote to `done`: the local reading as
 * soon as it is assembled (null when nothing in it could be read), then an
 * exact one if there is one. */
export async function readQuote(
  plan: QuotePlan,
  ctx: QuoteContext,
  /** the whole selection, as a snapshot token: what the source is searched
   * for */
  whole: string | null,
  done: (tex: string | null) => void
): Promise<void> {
  const tex = new Map<string, string | null>();
  for (const part of plan.inline) {
    const height = ctx.pageHeight(part.page);
    if (height === undefined) continue;
    const token = registerSnapshot({
      docId: ctx.docId,
      page: part.page,
      rects: part.rects.map((r) => topLeft(r, height)),
      text: part.text,
    });
    const answer = await convertSelection(token, 'local').catch(() => null);
    tex.set(part.key, await trusted(oneFormula(answer?.markdown), false));
  }
  for (const part of plan.displays) tex.set(part.key, await readDisplay(part, ctx, 'local'));
  const local = fillQuote(plan, tex);
  const read = [...tex.values()].some((t) => t !== null) ? local : null;
  done(read);
  if (!read) return;

  // The passage from the source, whole.
  if (whole) {
    const exact = await convertSelection(whole, 'exact').catch(() => null);
    const markdown = exact?.markdown.trim();
    if (markdown && plausibleQuoteTex(markdown, plan.plain) && (await texParses(markdown))) {
      done(markdown);
      return;
    }
  }
  // Else each display from the source.
  let better = false;
  for (const part of plan.displays) {
    const exact = await readDisplay(part, ctx, 'exact');
    if (exact && exact !== tex.get(part.key)) {
      tex.set(part.key, exact);
      better = true;
    }
  }
  if (better) done(fillQuote(plan, tex));
}
