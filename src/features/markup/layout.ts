/** Laying the exported pages out, given measured heights: the notes in a
 * page's margin (beside what they are about, never overlapping, shortened
 * when a page has more to say than room), and the feedback pages (blocks
 * filled page by page, a heading kept with what follows it, a table's head
 * repeated where it runs over). Pure; all in points (CSS px == pt). */

export type NoteForm = 'full' | 'short' | 'tiny';

export interface MarginNote {
  id: string;
  /** Where it would like its top: level with its anchor. */
  desiredTop: number;
  /** Heights of the note as it is, shortened ("first lines... see N on
   * p. X"), and as small as it goes (number and "see N on p. X"). */
  full: number;
  short: number;
  tiny: number;
}

export interface Placement {
  id: string;
  top: number;
  height: number;
  form: NoteForm;
}

export interface MarginBox {
  /** Usable band of the sheet, from the top. */
  top: number;
  bottom: number;
  /** Space kept between two notes. */
  gap: number;
}

function heightOf(note: MarginNote, form: NoteForm): number {
  return form === 'full'
    ? note.full
    : form === 'short'
      ? Math.min(note.short, note.full)
      : note.tiny;
}

/** Stack the notes down the margin. Each sits at its anchor's height if it
 * can, else just below the one before. When they do not all fit, the
 * longest are shortened first, then shortened further; then everything is
 * pushed back up from the bottom so the last note ends on the page. */
export function stackMargin(notes: readonly MarginNote[], box: MarginBox): Placement[] {
  const sorted = [...notes].sort((a, b) => a.desiredTop - b.desiredTop);
  const forms: NoteForm[] = sorted.map(() => 'full');
  const avail = box.bottom - box.top;
  const total = () =>
    sorted.reduce((sum, note, i) => sum + heightOf(note, forms[i]!), 0) +
    box.gap * Math.max(0, sorted.length - 1);

  for (const from of ['full', 'short'] as const) {
    const to: NoteForm = from === 'full' ? 'short' : 'tiny';
    while (total() > avail) {
      let pick = -1;
      let saving = 0;
      sorted.forEach((note, i) => {
        if (forms[i] !== from) return;
        const s = heightOf(note, from) - heightOf(note, to);
        if (s > saving) {
          saving = s;
          pick = i;
        }
      });
      if (pick < 0) break;
      forms[pick] = to;
    }
    if (total() <= avail) break;
  }

  const heights = sorted.map((note, i) => heightOf(note, forms[i]!));
  const tops: number[] = [];
  sorted.forEach((note, i) => {
    const after = i === 0 ? box.top : tops[i - 1]! + heights[i - 1]! + box.gap;
    tops.push(Math.max(note.desiredTop, box.top, after));
  });
  // Back up from the bottom, never above the band's top.
  for (let i = sorted.length - 1; i >= 0; i--) {
    const limit = i === sorted.length - 1 ? box.bottom : tops[i + 1]! - box.gap;
    tops[i] = Math.max(box.top, Math.min(tops[i]!, limit - heights[i]!));
  }
  return sorted.map((note, i) => ({
    id: note.id,
    top: Math.round(tops[i]! * 100) / 100,
    height: heights[i]!,
    form: forms[i]!,
  }));
}

/** The margin strip's frame, in points: notes are set `left` in from the
 * page edge, `right` short of the sheet's, in the band between `top` and
 * the page height less `bottom`, `gap` apart. */
export const MARGIN = { left: 14, right: 12, top: 18, bottom: 18, gap: 5 };

/** Stack a page's notes in its margin strip, a page `pageHeight` tall. */
export function layoutMargin(notes: readonly MarginNote[], pageHeight: number): Placement[] {
  return stackMargin(notes, {
    top: MARGIN.top,
    bottom: pageHeight - MARGIN.bottom,
    gap: MARGIN.gap,
  });
}

/* ---- feedback pages ---- */

export interface FlowBlock {
  height: number;
  /** A heading: never the last thing on a page. */
  keepWithNext?: boolean;
  /** A table row: when it starts a page, its table's head (this tall) is
   * drawn above it again. */
  headerHeight?: number;
}

export interface FlowPage {
  /** Indices of the blocks on the page, in order. */
  blocks: number[];
  /** The first block is a table row continued from the page before. */
  repeatHeader: boolean;
}

/** Fill pages `room` points tall with the blocks in order. A block taller
 * than a page gets one to itself (and is cut at its foot). */
export function paginate(blocks: readonly FlowBlock[], room: number): FlowPage[] {
  const pages: FlowPage[] = [];
  let current: FlowPage = { blocks: [], repeatHeader: false };
  let used = 0;
  const newPage = () => {
    if (current.blocks.length > 0) pages.push(current);
    current = { blocks: [], repeatHeader: false };
    used = 0;
  };
  const cost = (i: number, atTop: boolean) => {
    const b = blocks[i]!;
    return b.height + (atTop && b.headerHeight ? b.headerHeight : 0);
  };

  for (let i = 0; i < blocks.length; i++) {
    const atTop = current.blocks.length === 0;
    let need = cost(i, atTop);
    // A heading takes the next block along with it.
    if (blocks[i]!.keepWithNext && i + 1 < blocks.length) need += cost(i + 1, false);
    if (!atTop && used + need > room) {
      newPage();
    }
    if (current.blocks.length === 0 && blocks[i]!.headerHeight) {
      // A row opening a page: is it a continuation (its table started on an
      // earlier page)? The table's first row follows its head directly, and
      // the head is its own block, so only a row not preceded by one is.
      const prev = blocks[i - 1];
      current.repeatHeader = prev !== undefined && prev.headerHeight !== undefined;
    }
    used += cost(i, current.blocks.length === 0);
    current.blocks.push(i);
  }
  newPage();
  return pages;
}
