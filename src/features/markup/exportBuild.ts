/** The `markup_export` request: everything the backend needs to write the
 * annotated copy, laid out here.
 *
 * - **Marks**: native PDF annotations on the original pages (a soft
 *   highlight per comment, a strikeout and caret per typo fix, a square per
 *   region), each carrying its numbered comment as its contents.
 * - **Margin sheets**: the strip added to the right of each page with
 *   comments, the notes level with what they are about, stacked without
 *   overlap and shortened when a page has more to say than room.
 * - **Feedback sheets**: the letter at the end, set like a careful referee
 *   report - title and date, the Overall paragraph, the comments by section
 *   in paper order, a table of typos, the library items cited.
 *
 * Every block is measured in the same stylesheet the print window uses
 * (print.css, CSS px == pt) before it is placed, so pages break where they
 * will print. Numbers link both ways: a margin note to its entry, an entry
 * to its place on the page. Measuring is handed in, so this is tested
 * without a browser. */
import { escapeHtml, inlineHtml, renderBody } from './bodyHtml';
import { anchorTopFromTop } from './geometry';
import { markColor } from './kindColors';
import { labelParts } from './labels';
import {
  citedKeys,
  citationsAsText,
  groupBySection,
  isTypoEntry,
  mergeRepeats,
  shortQuote,
  typoSuggestion,
  type Entry,
} from './model';
import { layoutMargin, MARGIN, paginate, type FlowBlock } from './layout';
import type {
  ExportMark,
  ExportSheet,
  LinkTarget,
  MarkupExportRequest,
  SheetLink,
} from './markupTransport';
import { KIND_LABEL, type MarkupComment } from './types';

export const DEFAULT_MARGIN_PT = 180;

/** The feedback pages' frame, in points. */
export const FEEDBACK = {
  top: 64,
  bottom: 64,
  /** The text column: never wider than this, centred, at least `side` in. */
  column: 468,
  side: 64,
  /** The folio's baseline band at the foot. */
  folio: 30,
};

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
  /** The paper's title as the letter names it. */
  title: string;
  date: string;
  summary: string;
  /** Every comment, numbered and labelled (`buildEntries`). */
  entries: readonly Entry[];
  /** Pages in the file: feedback page k prints as page `sourcePages + k + 1`. */
  sourcePages: number;
  /** A source page's size as displayed, points. */
  pageSize: (page: number) => { w: number; h: number } | null;
  marginPt?: number;
  math: ((tex: string, display: boolean) => string) | null;
  /** Cite key -> label without brackets, null when no item has it. */
  cite: (key: string) => string | null;
  /** Cite key -> the reference line (plain text, may hold $math$). */
  reference: (key: string) => string | null;
  measure: MeasureFn;
}

/* ---- small pieces of markup ---- */

const sep = '<span class="mk-sep" aria-hidden="true"></span>';

function labelHtml(entry: Entry): string {
  return labelParts(entry.label, false).map(escapeHtml).join(sep);
}

function kindTag(comment: MarkupComment): string {
  return `<span class="mk-kind mk-k-${comment.kind}">${KIND_LABEL[comment.kind]}</span>`;
}

/** The typo fix as struck words and their replacement. */
function fixHtml(comment: MarkupComment, opts: BuildInput): string {
  const fix = typoSuggestion(comment);
  const original = `<span class="mk-del">${escapeHtml(shortQuote(comment.anchor.quote, 60))}</span>`;
  return fix
    ? `${original} <span class="mk-ins">${inlineHtml(fix, { math: opts.math })}</span>`
    : original;
}

/** A comment's words for a native annotation's contents: plain text. */
export function plainContents(number: number, comment: MarkupComment, opts: BuildInput): string {
  const body = citationsAsText(comment.body.trim(), opts.cite).replace(/\s+/g, ' ');
  if (comment.kind === 'typo') {
    const fix = typoSuggestion(comment);
    const head = fix
      ? `${number}. Replace "${shortQuote(comment.anchor.quote, 60)}" with "${fix}"`
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
      continue;
    }
    marks.push({ page: a.page, kind: 'highlight', quads: a.quads, rect: null, color, contents });
  }
  return marks;
}

/* ---- feedback blocks ---- */

interface FeedbackBlock extends FlowBlock {
  html: string;
  /** What each `data-link` key inside points at. */
  targets: Map<string, LinkTarget>;
  /** Entry numbers this block stands for, for links into it. */
  numbers: number[];
}

/** Where a comment's place on its page is, for a link. */
function pageTarget(comment: MarkupComment, opts: BuildInput): LinkTarget {
  const size = opts.pageSize(comment.anchor.page);
  const top = size ? anchorTopFromTop(comment.anchor, size.h) : 0;
  return { kind: 'page', page: comment.anchor.page, y: Math.max(0, Math.round(top - 24)) };
}

function feedbackBlocks(opts: BuildInput): FeedbackBlock[] {
  const blocks: FeedbackBlock[] = [];
  const add = (html: string, extra: Partial<FeedbackBlock> = {}) =>
    blocks.push({ height: 0, html, targets: new Map(), numbers: [], ...extra });

  add(
    `<div class="mk-block mk-title"><div class="mk-eyebrow">Referee comments</div>` +
      `<h1>Comments on ${inlineHtml(opts.title, { math: opts.math })}</h1>` +
      `<div class="mk-date">${escapeHtml(opts.date)}</div></div>`
  );
  if (opts.summary.trim()) {
    add(
      `<div class="mk-block mk-overall"><h2>Overall</h2>` +
        `<div class="mk-body">${renderBody(opts.summary, { math: opts.math, cite: opts.cite })}</div></div>`
    );
  }

  const prose = opts.entries.filter((e) => !isTypoEntry(e.comment));
  const merged = mergeRepeats(prose);
  const groups = groupBySection(merged.map((m) => m.entry));
  if (groups.length > 0) {
    add('<div class="mk-block mk-part"><h2>Comments</h2></div>', { keepWithNext: true });
  }
  for (const group of groups) {
    add(
      `<div class="mk-block mk-section"><h3>${inlineHtml(group.title, { math: opts.math })}</h3></div>`,
      {
        keepWithNext: true,
      }
    );
    for (const entry of group.entries) {
      const m = merged.find((x) => x.entry === entry)!;
      const c = entry.comment;
      const targets = new Map<string, LinkTarget>([['num', pageTarget(c, opts)]]);
      const quote = c.anchor.quote.trim()
        ? `<div class="mk-quote">${escapeHtml(shortQuote(c.anchor.quote, 220))}</div>`
        : c.anchor.rect
          ? '<div class="mk-quote is-region">The marked region</div>'
          : '';
      const body = c.body.trim()
        ? `<div class="mk-body">${renderBody(c.body, { math: opts.math, cite: opts.cite })}</div>`
        : '';
      let also = '';
      if (m.also.length > 0) {
        also =
          '<div class="mk-also">Also ' +
          m.also
            .map((e, i) => {
              targets.set(`also${i}`, pageTarget(e.comment, opts));
              return `<a class="mk-link" data-link="also${i}">p. ${e.label.page}</a>`;
            })
            .join(', ') +
          '.</div>';
      }
      add(
        `<div class="mk-block mk-entry">` +
          `<a class="mk-num" data-link="num">${entry.number}</a>` +
          `<div class="mk-entry-main"><div class="mk-entry-head">` +
          `<span class="mk-label">${labelHtml(entry)}</span>${kindTag(c)}</div>` +
          `${quote}${body}${also}</div></div>`,
        { targets, numbers: [entry.number, ...m.also.map((e) => e.number)] }
      );
    }
  }

  const typos = opts.entries.filter((e) => isTypoEntry(e.comment));
  if (typos.length > 0) {
    add('<div class="mk-block mk-section mk-typos-title"><h2>Typos and wording</h2></div>', {
      keepWithNext: true,
    });
    const head =
      '<div class="mk-block mk-trow mk-thead"><span>No.</span><span>Page</span>' +
      '<span>Original</span><span>Suggested</span></div>';
    add(head, { keepWithNext: true });
    for (const e of typos) {
      const c = e.comment;
      const fix = typoSuggestion(c);
      const note =
        c.replacement !== null && c.body.trim()
          ? `<div class="mk-tnote">${inlineHtml(c.body.trim(), { math: opts.math, cite: opts.cite })}</div>`
          : '';
      add(
        `<div class="mk-block mk-trow">` +
          `<a class="mk-tnum" data-link="num">${e.number}</a>` +
          `<span>${e.label.page}</span>` +
          `<span class="mk-del">${escapeHtml(shortQuote(c.anchor.quote, 90))}</span>` +
          `<span>${fix ? `<span class="mk-ins">${inlineHtml(fix, { math: opts.math })}</span>` : '<span class="mk-dim">see note</span>'}${note}</span>` +
          `</div>`,
        {
          targets: new Map([['num', pageTarget(c, opts)]]),
          numbers: [e.number],
          headerHeight: 0,
        }
      );
    }
  }

  const keys = [
    ...new Set([opts.summary, ...opts.entries.map((e) => e.comment.body)].flatMap(citedKeys)),
  ];
  const refs = keys
    .map((key) => ({ label: opts.cite(key), line: opts.reference(key) }))
    .filter((r): r is { label: string; line: string } => !!r.label && !!r.line);
  if (refs.length > 0) {
    add('<div class="mk-block mk-section"><h2>References</h2></div>', { keepWithNext: true });
    for (const r of refs) {
      add(
        `<div class="mk-block mk-ref"><span class="mk-ref-label">[${escapeHtml(r.label)}]</span>` +
          `<span class="mk-ref-text">${inlineHtml(r.line, { math: opts.math })}</span></div>`
      );
    }
  }
  return blocks;
}

/** The text column of a feedback sheet `width` wide. */
export function feedbackColumn(width: number): { left: number; width: number } {
  const column = Math.min(FEEDBACK.column, width - 2 * FEEDBACK.side);
  return { left: Math.round((width - column) / 2), width: Math.round(column) };
}

function sheetHtml(kind: 'margin' | 'feedback', width: number, height: number, inner: string) {
  return (
    `<div class="mk-sheet mk-${kind}" style="width:${width}px;height:${height}px">` +
    `${inner}</div>`
  );
}

const px = (v: number) => `${Math.round(v * 100) / 100}px`;

/* ---- the whole request ---- */

export async function buildExportRequest(opts: BuildInput): Promise<MarkupExportRequest> {
  const marginPt = opts.marginPt ?? DEFAULT_MARGIN_PT;
  const first = opts.pageSize(0) ?? { w: 612, h: 792 };
  const sheetW = first.w + marginPt;
  const sheetH = first.h;
  const column = feedbackColumn(sheetW);

  // The letter first: the margin notes say which of its pages to see.
  const blocks = feedbackBlocks(opts);
  const thead = blocks.findIndex((b) => b.html.includes('mk-thead'));
  const measured = await opts.measure(
    blocks.map((b) => ({ html: b.html, sheet: 'feedback', width: column.width }))
  );
  blocks.forEach((b, i) => {
    b.height = measured[i]!.height;
    if (b.headerHeight !== undefined && thead >= 0) b.headerHeight = measured[thead]!.height;
  });
  const room = sheetH - FEEDBACK.top - FEEDBACK.bottom;
  const pages = paginate(blocks, room);

  const sheets: ExportSheet[] = [];
  /** Entry number -> where it is in the letter. */
  const located = new Map<number, { sheet: number; y: number; page: number }>();
  pages.forEach((page, k) => {
    const links: SheetLink[] = [];
    let y = FEEDBACK.top;
    let inner = '';
    if (page.repeatHeader && thead >= 0) {
      inner += `<div class="mk-flow" style="top:${px(y)};left:${px(column.left)};width:${px(column.width)}">${blocks[thead]!.html.replace('mk-thead', 'mk-thead is-continued')}</div>`;
      y += blocks[thead]!.height;
    }
    for (const i of page.blocks) {
      const b = blocks[i]!;
      inner += `<div class="mk-flow" style="top:${px(y)};left:${px(column.left)};width:${px(column.width)}">${b.html}</div>`;
      for (const link of measured[i]!.links) {
        const target = b.targets.get(link.key);
        if (target) {
          links.push({
            rect: { x: column.left + link.x, y: y + link.y, w: link.w, h: link.h },
            target,
          });
        }
      }
      for (const n of b.numbers) {
        located.set(n, {
          sheet: k,
          y: Math.max(0, Math.round(y - 6)),
          page: opts.sourcePages + k + 1,
        });
      }
      y += b.height;
    }
    inner += `<div class="mk-folio">${k + 1} of ${pages.length}</div>`;
    sheets.push({
      kind: 'feedback',
      widthPt: sheetW,
      heightPt: sheetH,
      html: sheetHtml('feedback', sheetW, sheetH, inner),
      links,
    });
  });

  // Then the margins, page by page.
  const byPage = new Map<number, Entry[]>();
  for (const entry of opts.entries) {
    const list = byPage.get(entry.comment.anchor.page);
    if (list) list.push(entry);
    else byPage.set(entry.comment.anchor.page, [entry]);
  }
  const noteWidth = marginPt - MARGIN.left - MARGIN.right;
  for (const [page, entries] of [...byPage.entries()].sort((a, b) => a[0] - b[0])) {
    const size = opts.pageSize(page);
    if (!size) continue;
    const notes = entries.map((entry) => {
      const where = located.get(entry.number);
      const see = where ? `see ${entry.number} on p. ${where.page}` : '';
      return {
        entry,
        forms: noteForms(entry, see, opts),
        desiredTop: anchorTopFromTop(entry.comment.anchor, size.h) - 1,
      };
    });
    const flat = notes.flatMap((n) => [n.forms.full, n.forms.short, n.forms.tiny]);
    const sizes = await opts.measure(
      flat.map((html) => ({ html, sheet: 'margin', width: noteWidth }))
    );
    const placed = layoutMargin(
      notes.map((n, i) => ({
        id: String(n.entry.number),
        desiredTop: n.desiredTop,
        full: sizes[i * 3]!.height,
        short: sizes[i * 3 + 1]!.height,
        tiny: sizes[i * 3 + 2]!.height,
      })),
      size.h
    );
    const links: SheetLink[] = [];
    let inner = '';
    for (const p of placed) {
      const i = notes.findIndex((n) => String(n.entry.number) === p.id);
      const note = notes[i]!;
      const which = p.form === 'full' ? 0 : p.form === 'short' ? 1 : 2;
      inner += `<div class="mk-flow" style="top:${px(p.top)};left:${px(MARGIN.left)};width:${px(noteWidth)}">${[note.forms.full, note.forms.short, note.forms.tiny][which]}</div>`;
      const where = located.get(note.entry.number);
      if (where) {
        for (const link of sizes[i * 3 + which]!.links) {
          links.push({
            rect: { x: MARGIN.left + link.x, y: p.top + link.y, w: link.w, h: link.h },
            target: { kind: 'sheet', sheet: where.sheet, y: where.y },
          });
        }
      }
    }
    sheets.push({
      kind: 'margin',
      page,
      widthPt: marginPt,
      heightPt: size.h,
      html: sheetHtml('margin', marginPt, size.h, inner),
      links,
    });
  }

  return {
    source: opts.source,
    dest: opts.dest,
    marginPt,
    marks: buildMarks(opts.entries, opts),
    sheets,
  };
}

/** A margin note in its three lengths. */
function noteForms(entry: Entry, see: string, opts: BuildInput) {
  const c = entry.comment;
  const badge = `<a class="mk-badge mk-k-${c.kind}" data-link="badge">${entry.number}</a>`;
  let text: string;
  if (c.kind === 'typo') {
    const note =
      c.replacement !== null && c.body.trim()
        ? ` <span class="mk-dim">${inlineHtml(c.body.trim(), { math: opts.math, cite: opts.cite })}</span>`
        : '';
    text = `${fixHtml(c, opts)}${note}`;
  } else if (c.body.trim()) {
    text = renderBody(c.body, { math: opts.math, cite: opts.cite });
  } else {
    text = `<span class="mk-dim">${KIND_LABEL[c.kind]}</span>`;
  }
  const seeHtml = see ? `<div class="mk-see">... ${escapeHtml(see)}</div>` : '';
  return {
    full: `<div class="mk-note">${badge}<div class="mk-note-text">${text}</div></div>`,
    short: `<div class="mk-note is-short">${badge}<div class="mk-note-text">${text}</div>${seeHtml}</div>`,
    tiny: `<div class="mk-note is-tiny">${badge}<div class="mk-see">${escapeHtml(see || KIND_LABEL[c.kind])}</div></div>`,
  };
}
