/** The `markup_export` request: the paper, each page widened by a margin
 * strip holding its notes, with the commented words marked by native PDF
 * annotations. Nothing else - no title page, no letter.
 *
 * - **Marks**: a soft highlight per comment, a strikeout and caret per typo
 *   fix, a square per region, each carrying its numbered comment; and the
 *   comment's number just after its phrase (`label`), drawn into the page
 *   in the colour of its note's badge, so the two can be matched.
 * - **Margin sheets**: each note is its number and its words, level with
 *   what it is about. Notes that collide are moved up as well as down, as
 *   little as possible in all, keeping their order (`placeNotes`), over the
 *   whole height of the page. The Overall summary, when there is one, heads
 *   page 1's margin. A crowded page is set a little smaller.
 * - **Continuation pages**, only as a last resort: when a page's notes
 *   still do not fit at the smallest size, the longest keep their first
 *   line and "(cont. p. N)" in the margin, and their full text goes on one
 *   page added at the end for that page. A normal review adds none.
 *
 * Every note is measured in the stylesheet the print window uses
 * (print.css, CSS px == pt) before it is placed. Measuring is handed in, so
 * this is tested without a browser. */
import { escapeHtml, inlineHtml, renderBody } from './bodyHtml';
import { anchorTopFromTop, markerRects } from './geometry';
import { KIND_INK, markColor } from './kindColors';
import { citationsAsText, shortQuote, typoSuggestion, type Entry } from './model';
import { MARGIN, paginate, placeNotes } from './layout';
import type {
  MarkupExportRequest,
  MarkupLinkTarget as LinkTarget,
  MarkupMark as ExportMark,
  MarkupSheet as ExportSheet,
  MarkupSheetLink as SheetLink,
} from '../../lib/transport/markup';
import { KIND_LABEL, type MarkupComment } from './types';

export const DEFAULT_MARGIN_PT = 180;

/** Margin type sizes, px == pt, tried in turn on a crowded page. The
 * stylesheet's own size is the first; never below the last. */
export const NOTE_SIZES = [7.4, 7, 6.6] as const;

/** A continuation page's frame, in points. */
export const CONTINUATION = { top: 56, bottom: 56, column: 420, side: 56, size: 9.5 };

export interface MeasureItem {
  html: string;
  /** The sheet the block will sit on: its rules apply while measuring. */
  sheet: 'margin' | 'feedback';
  /** Width of the column the block is set in. */
  width: number;
}

export interface MeasuredLink {
  key: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Measured {
  height: number;
  /** Every `[data-link]` element inside, relative to the block's top-left. */
  links: MeasuredLink[];
}

export type MeasureFn = (items: MeasureItem[]) => Promise<Measured[]>;

export interface BuildInput {
  source: string;
  dest: string;
  /** The Overall paragraph; heads page 1's margin when not empty. */
  summary: string;
  /** Every comment, numbered (`buildEntries`). */
  entries: readonly Entry[];
  /** Pages in the file: an added page k prints as page `sourcePages + k + 1`. */
  sourcePages: number;
  /** A source page's size as displayed, points. */
  pageSize: (page: number) => { w: number; h: number } | null;
  marginPt?: number;
  math: ((tex: string, display: boolean) => string) | null;
  /** Cite key -> label without brackets, null when no item has it. */
  cite: (key: string) => string | null;
  measure: MeasureFn;
}

/* ---- the words of a note ---- */

/** A typo fix: the struck words and what goes in their place. */
function fixHtml(comment: MarkupComment, opts: BuildInput): string {
  const fix = typoSuggestion(comment);
  const original = `<span class="mk-del">${escapeHtml(shortQuote(comment.anchor.quote, 60))}</span>`;
  if (comment.replacement === '') return `${original} <span class="mk-dim">(delete)</span>`;
  return fix
    ? `${original} <span class="mk-ins">${inlineHtml(fix, { math: opts.math })}</span>`
    : original;
}

/** What a note says: the fix for a typo (and its remark), else the body. */
function noteText(comment: MarkupComment, opts: BuildInput): string {
  const body = comment.body.trim();
  if (comment.kind === 'typo' && comment.replacement !== null) {
    const note = body
      ? ` <span class="mk-dim">${inlineHtml(body, { math: opts.math, cite: opts.cite })}</span>`
      : '';
    return `<p>${fixHtml(comment, opts)}${note}</p>`;
  }
  if (body) return renderBody(body, { math: opts.math, cite: opts.cite });
  return `<p class="mk-dim">${KIND_LABEL[comment.kind]}</p>`;
}

/** The note's number in its kind's colour; a link when `link` names one. */
const badge = (c: MarkupComment, n: number, link?: string) =>
  link
    ? `<a class="mk-badge mk-k-${c.kind}" data-link="${link}">${n}</a>`
    : `<span class="mk-badge mk-k-${c.kind}">${n}</span>`;

function noteHtml(entry: Entry, opts: BuildInput, size: number, link?: string): string {
  const c = entry.comment;
  return (
    `<div class="mk-note" style="font-size:${size}px">${badge(c, entry.number, link)}` +
    `<div class="mk-note-text">${noteText(c, opts)}</div></div>`
  );
}

/** A note cut to its first line, pointing to its full text on page `to`. */
function stubHtml(entry: Entry, opts: BuildInput, size: number, to: string): string {
  const c = entry.comment;
  return (
    `<div class="mk-note is-stub" style="font-size:${size}px">${badge(c, entry.number)}` +
    `<div class="mk-note-text">${noteText(c, opts)}</div>` +
    `<a class="mk-cont" data-link="cont">(cont. p. ${to})</a></div>`
  );
}

function summaryHtml(opts: BuildInput, size: number): string {
  return (
    `<div class="mk-summary" style="font-size:${size}px">` +
    `${renderBody(opts.summary, { math: opts.math, cite: opts.cite })}</div>`
  );
}

/** A comment's words for a native annotation's contents: plain text. */
export function plainContents(number: number, comment: MarkupComment, opts: BuildInput): string {
  const body = citationsAsText(comment.body.trim(), opts.cite).replace(/\s+/g, ' ');
  if (comment.kind === 'typo') {
    const fix = typoSuggestion(comment);
    const quote = shortQuote(comment.anchor.quote, 60);
    const head =
      comment.replacement === ''
        ? `${number}. Delete "${quote}"`
        : fix
          ? `${number}. Replace "${quote}" with "${fix}"`
          : `${number}. Typo`;
    return comment.replacement !== null && body ? `${head}: ${body}` : head;
  }
  return body ? `${number}. ${body}` : `${number}. ${KIND_LABEL[comment.kind]}`;
}

/* ---- marks ---- */

export function buildMarks(entries: readonly Entry[], opts: BuildInput): ExportMark[] {
  const marks: ExportMark[] = [];
  for (const { comment, number } of entries) {
    const a = comment.anchor;
    const contents = plainContents(number, comment, opts);
    const color = markColor(comment.kind);
    if (a.rect) {
      marks.push({ page: a.page, kind: 'square', quads: null, rect: a.rect, color, contents });
      continue;
    }
    if (!a.quads || a.quads.length === 0) continue;
    if (comment.kind === 'typo' && comment.replacement !== null) {
      marks.push({ page: a.page, kind: 'strikeout', quads: a.quads, rect: null, color, contents });
      if (comment.replacement !== '') {
        const last = a.quads.at(-1)!;
        const h = Math.max(4, Math.min(8, (last.y1 - last.y3) * 0.6));
        marks.push({
          page: a.page,
          kind: 'caret',
          quads: null,
          rect: { x: last.x2 - h / 2, y: last.y3 - h * 0.35, w: h, h },
          color,
          contents: `${number}. Insert "${comment.replacement}"`,
        });
      }
      continue;
    }
    marks.push({ page: a.page, kind: 'highlight', quads: a.quads, rect: null, color, contents });
  }
  // The number by each phrase, matching its note's badge.
  const markers = markerRects(entries.map((e) => ({ number: e.number, anchor: e.comment.anchor })));
  for (const { comment, number } of entries) {
    const rect = markers.get(number);
    if (!rect) continue;
    marks.push({
      page: comment.anchor.page,
      kind: 'label',
      quads: null,
      rect,
      color: KIND_INK[comment.kind],
      contents: String(number),
    });
  }
  return marks;
}

/* ---- the margin of one page ---- */

interface PagePlan {
  page: number;
  size: { w: number; h: number };
  noteSize: number;
  /** The summary's block on this page (page 1 only). */
  summary: { html: string; height: number } | null;
  notes: {
    entry: Entry;
    top: number;
    /** Cut to its first line, its full text on a continuation page. */
    stub: boolean;
    full: Measured;
    stubbed: Measured;
  }[];
}

const px = (v: number) => `${Math.round(v * 100) / 100}px`;

/** Lay out one page's margin: at each type size in turn, the notes as they
 * are; failing that, at the smallest, the longest cut to a line until the
 * rest fit. */
async function planPage(
  page: number,
  size: { w: number; h: number },
  entries: Entry[],
  opts: BuildInput,
  width: number
): Promise<PagePlan> {
  const withSummary = page === 0 && opts.summary.trim() !== '';
  const items: MeasureItem[] = [];
  for (const s of NOTE_SIZES) {
    if (withSummary) items.push({ html: summaryHtml(opts, s), sheet: 'margin', width });
    for (const e of entries) {
      items.push({ html: noteHtml(e, opts, s), sheet: 'margin', width });
      items.push({ html: stubHtml(e, opts, s, '999'), sheet: 'margin', width });
    }
  }
  const measured = await opts.measure(items);
  const per = entries.length * 2 + (withSummary ? 1 : 0);

  let plan: PagePlan | null = null;
  NOTE_SIZES.forEach((noteSize, level) => {
    if (plan && plan.notes.every((n) => !n.stub)) return;
    const base = level * per;
    const summary = withSummary
      ? { html: summaryHtml(opts, noteSize), height: measured[base]!.height }
      : null;
    const at = base + (withSummary ? 1 : 0);
    const full = entries.map((_, i) => measured[at + i * 2]!);
    const stubbed = entries.map((_, i) => measured[at + i * 2 + 1]!);
    const top = MARGIN.top + (summary ? summary.height + MARGIN.gap * 2 : 0);
    const bottom = size.h - MARGIN.bottom;
    const desired = entries.map((e) => anchorTopFromTop(e.comment.anchor, size.h) - 1);
    const stub = entries.map(() => false);
    const heights = () => full.map((m, i) => (stub[i] ? stubbed[i]!.height : m.height));
    const fits = () =>
      heights().reduce((s, h) => s + h, 0) + MARGIN.gap * Math.max(0, entries.length - 1) <=
      bottom - top;
    // Only the last size cuts notes; before that, a smaller size is tried.
    if (!fits() && level < NOTE_SIZES.length - 1) return;
    while (!fits()) {
      let pick = -1;
      let saving = 0;
      full.forEach((m, i) => {
        const s = m.height - stubbed[i]!.height;
        if (!stub[i] && s > saving) {
          saving = s;
          pick = i;
        }
      });
      if (pick < 0) break;
      stub[pick] = true;
    }
    const tops = placeNotes(desired, heights(), { top, bottom, gap: MARGIN.gap });
    plan = {
      page,
      size,
      noteSize,
      summary,
      notes: entries.map((entry, i) => ({
        entry,
        top: tops[i]!,
        stub: stub[i]!,
        full: full[i]!,
        stubbed: stubbed[i]!,
      })),
    };
  });
  return plan!;
}

/* ---- the whole request ---- */

export async function buildExportRequest(opts: BuildInput): Promise<MarkupExportRequest> {
  const marginPt = opts.marginPt ?? DEFAULT_MARGIN_PT;
  const noteWidth = marginPt - MARGIN.left - MARGIN.right;

  const byPage = new Map<number, Entry[]>();
  for (const entry of opts.entries) {
    const list = byPage.get(entry.comment.anchor.page);
    if (list) list.push(entry);
    else byPage.set(entry.comment.anchor.page, [entry]);
  }
  if (opts.summary.trim() && !byPage.has(0)) byPage.set(0, []);

  const plans: PagePlan[] = [];
  for (const [page, entries] of [...byPage.entries()].sort((a, b) => a[0] - b[0])) {
    const size = opts.pageSize(page);
    if (!size) continue;
    const sorted = [...entries].sort((a, b) => a.number - b.number);
    plans.push(await planPage(page, size, sorted, opts, noteWidth));
  }

  // The last resort: one page (or more, for a lot) at the end per page whose
  // notes did not fit, with their full text.
  const first = opts.pageSize(0) ?? { w: 612, h: 792 };
  const contW = first.w + marginPt;
  const contH = first.h;
  const column = Math.min(CONTINUATION.column, contW - 2 * CONTINUATION.side);
  const left = Math.round((contW - column) / 2);
  const continued: ExportSheet[] = [];
  /** Entry number -> where its full text is. */
  const contAt = new Map<number, { sheet: number; y: number; page: number }>();
  for (const plan of plans) {
    const cut = plan.notes.filter((n) => n.stub);
    if (cut.length === 0) continue;
    const blocks = [
      `<div class="mk-block mk-cont-head">Notes continued from p. ${plan.page + 1}</div>`,
      ...cut.map(
        (n) =>
          `<div class="mk-block mk-cont-note">${noteHtml(n.entry, opts, CONTINUATION.size, 'back')}</div>`
      ),
    ];
    const sizes = await opts.measure(
      blocks.map((html) => ({ html, sheet: 'feedback', width: column }))
    );
    const pages = paginate(
      sizes.map((m, i) => ({ height: m.height, ...(i === 0 ? { keepWithNext: true } : {}) })),
      contH - CONTINUATION.top - CONTINUATION.bottom
    );
    for (const p of pages) {
      const index = continued.length;
      let y = CONTINUATION.top;
      let inner = '';
      const links: SheetLink[] = [];
      for (const i of p.blocks) {
        inner += `<div class="mk-flow" style="top:${px(y)};left:${px(left)};width:${px(column)}">${blocks[i]}</div>`;
        if (i > 0) {
          const note = cut[i - 1]!;
          contAt.set(note.entry.number, {
            sheet: index,
            y: Math.max(0, Math.round(y - 8)),
            page: opts.sourcePages + index + 1,
          });
          for (const link of sizes[i]!.links) {
            links.push({
              rect: { x: left + link.x, y: y + link.y, w: link.w, h: link.h },
              target: {
                kind: 'page',
                page: plan.page,
                y: Math.max(
                  0,
                  Math.round(anchorTopFromTop(note.entry.comment.anchor, plan.size.h) - 24)
                ),
              },
            });
          }
        }
        y += sizes[i]!.height;
      }
      continued.push({
        kind: 'feedback',
        widthPt: contW,
        heightPt: contH,
        html: sheet('feedback', contW, contH, inner),
        links,
      });
    }
  }

  const margins: ExportSheet[] = plans.map((plan) => {
    let inner = '';
    const links: SheetLink[] = [];
    if (plan.summary) {
      inner += `<div class="mk-flow" style="top:${px(MARGIN.top)};left:${px(MARGIN.left)};width:${px(noteWidth)}">${plan.summary.html}</div>`;
    }
    for (const n of plan.notes) {
      const where = contAt.get(n.entry.number);
      const html =
        n.stub && where
          ? stubHtml(n.entry, opts, plan.noteSize, String(where.page))
          : noteHtml(n.entry, opts, plan.noteSize);
      inner += `<div class="mk-flow" style="top:${px(n.top)};left:${px(MARGIN.left)};width:${px(noteWidth)}">${html}</div>`;
      if (n.stub && where) {
        for (const link of n.stubbed.links) {
          links.push({
            rect: { x: MARGIN.left + link.x, y: n.top + link.y, w: link.w, h: link.h },
            target: { kind: 'sheet', sheet: where.sheet, y: where.y } satisfies LinkTarget,
          });
        }
      }
    }
    return {
      kind: 'margin',
      page: plan.page,
      widthPt: marginPt,
      heightPt: plan.size.h,
      html: sheet('margin', marginPt, plan.size.h, inner),
      links,
    };
  });

  return {
    source: opts.source,
    dest: opts.dest,
    marginPt,
    marks: buildMarks(opts.entries, opts),
    // Continuation pages first, so a link's `sheet` is its index here.
    sheets: [...continued, ...margins],
  };
}

function sheet(kind: 'margin' | 'feedback', width: number, height: number, inner: string) {
  return `<div class="mk-sheet mk-${kind}" style="width:${width}px;height:${height}px">${inner}</div>`;
}
