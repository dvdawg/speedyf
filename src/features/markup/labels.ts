/** Where a comment sits in the paper, worked out live and never stored: the
 * section (from the outline, the PDF's own or the generated one), the
 * formal statement ("Lemma 2.3", "proof of Theorem 3.1") and the numbered
 * equation it falls in. Pure: the store hands in what the engine found. */
import { anchorBounds, anchorStart } from './geometry';
import type { Anchor } from './types';

/** A heading, in the shape `outlineAsEntries` gives (page of the file, y-up
 * top of the heading; depth 0 a section, 1 a subsection). */
export interface Heading {
  depth: number;
  label: string;
  page: number;
  y: number;
}

/** A statement or numbered equation as the engine reports it
 * (`get_doc_statements`): y-up rectangles on page `page` of the file. */
export interface StatementLike {
  kind: string;
  word: string;
  number: string;
  page: number;
  region: [number, number, number, number];
}

/** Where a proof begins: the "Proof" line's top, y-up. */
export interface ProofStart {
  page: number;
  y: number;
}

export interface LabelContext {
  headings: readonly Heading[];
  statements: readonly StatementLike[];
  proofs: readonly ProofStart[];
}

export const EMPTY_CONTEXT: LabelContext = { headings: [], statements: [], proofs: [] };

export interface SectionRef {
  /** The heading as printed: "2.1 Setup". */
  title: string;
  /** Short form for a label line: "Sec. 2.1", or the title when unnumbered. */
  short: string;
  /** Its position, for ordering and grouping. */
  page: number;
  y: number;
}

export interface CommentLabel {
  /** One-based page number as printed on the card. */
  page: number;
  section: SectionRef | null;
  /** "Lemma 2.3" or "proof of Theorem 3.1". */
  statement: string | null;
  /** "eq. (12)". */
  equation: string | null;
}

/** Is a (page, y) point at or after another in reading order? */
function atOrAfter(page: number, y: number, refPage: number, refY: number): boolean {
  return page > refPage || (page === refPage && y <= refY);
}

/** "2", "2.1", "A.1", or "A." (an appendix letter needs its dot, so "A note
 * on..." is not numbered). */
const NUMBERED = /^(\d+(?:\.\d+)*|[A-Z](?:\.\d+)+|[A-Z](?=\.))\.?\s+\S/;

/** "Sec. 2.1" for a numbered heading, else the title cut short. */
export function shortSection(title: string): string {
  const t = title.replace(/\s+/g, ' ').trim();
  const m = NUMBERED.exec(t);
  if (m) return `Sec. ${m[1]}`;
  if (t.length <= 28) return t;
  const cut = t.slice(0, 27);
  const space = cut.lastIndexOf(' ');
  return `${(space > 12 ? cut.slice(0, space) : cut).trimEnd()}...`;
}

/** The deepest heading at or above the point: the subsection when there is
 * one under the current section, else the section. */
export function sectionAt(
  headings: readonly Heading[],
  page: number,
  y: number
): SectionRef | null {
  let section: Heading | null = null;
  let sub: Heading | null = null;
  for (const h of headings) {
    if (!atOrAfter(page, y, h.page, h.y)) break;
    if (h.depth <= 0) {
      section = h;
      sub = null;
    } else {
      sub = h;
    }
  }
  const found = sub ?? section;
  if (!found) return null;
  return { title: found.label, short: shortSection(found.label), page: found.page, y: found.y };
}

/** A little slack round a region, for a selection's edges. */
const SLACK = 2;

function inside(
  region: readonly [number, number, number, number],
  x: number,
  y: number,
  slack = SLACK
): boolean {
  const [rx, ry, rw, rh] = region;
  return x >= rx - slack && x <= rx + rw + slack && y >= ry - slack && y <= ry + rh + slack;
}

/** How a statement is named in a label: "Lemma 2.3". */
export function statementName(s: StatementLike): string {
  return `${s.word} ${s.number}`.trim();
}

/** Farthest a proof is followed from where it starts. */
const PROOF_PAGES = 3;

/** The statement the point lies in, or the one whose proof it lies in. */
export function statementAt(ctx: LabelContext, page: number, x: number, y: number): string | null {
  const formal = ctx.statements.filter((s) => s.kind !== 'equation');
  const within = formal.find((s) => s.page === page && inside(s.region, x, y));
  if (within) return statementName(within);

  // The last proof start before the point...
  let proof: ProofStart | null = null;
  for (const p of ctx.proofs) {
    if (atOrAfter(page, y, p.page, p.y)) proof = p;
  }
  if (!proof || page - proof.page > PROOF_PAGES) return null;
  const start = proof;
  const top = (s: StatementLike) => s.region[1] + s.region[3];
  // ...with no statement or heading begun between it and the point...
  const between = (refPage: number, refY: number) =>
    atOrAfter(refPage, refY, start.page, start.y) && atOrAfter(page, y, refPage, refY);
  if (formal.some((s) => between(s.page, top(s)))) return null;
  if (ctx.headings.some((h) => between(h.page, h.y))) return null;
  // ...belongs to the statement just before it.
  let owner: StatementLike | null = null;
  for (const s of formal) {
    if (atOrAfter(start.page, start.y + SLACK, s.page, top(s))) owner = s;
  }
  if (!owner || start.page - owner.page > 1) return null;
  return `proof of ${statementName(owner)}`;
}

/** The numbered display the anchor is in: "eq. (12)". */
export function equationAt(ctx: LabelContext, anchor: Anchor): string | null {
  const b = anchorBounds(anchor);
  const cy = b.y + b.h / 2;
  const hit = ctx.statements.find((s) => {
    if (s.kind !== 'equation' || s.page !== anchor.page) return false;
    const [rx, ry, rw, rh] = s.region;
    const xOverlap = Math.min(rx + rw, b.x + b.w) - Math.max(rx, b.x);
    return xOverlap > 0 && cy >= ry - SLACK && cy <= ry + rh + SLACK;
  });
  return hit ? `eq. (${hit.number})` : null;
}

export function labelFor(anchor: Anchor, ctx: LabelContext): CommentLabel {
  const start = anchorStart(anchor);
  return {
    page: anchor.page + 1,
    section: sectionAt(ctx.headings, anchor.page, start.y),
    statement: statementAt(ctx, anchor.page, start.x, start.y - 1),
    equation: equationAt(ctx, anchor),
  };
}

/** The parts of a label line, in order: "p. 5", "Sec. 2.1", "Lemma 2.3",
 * "eq. (4)". Shown with a quiet separator between them. */
export function labelParts(label: CommentLabel, withSection = true): string[] {
  const parts = [`p. ${label.page}`];
  if (withSection && label.section) parts.push(label.section.short);
  if (label.statement) parts.push(label.statement);
  if (label.equation) parts.push(label.equation);
  return parts;
}

/** A label line as plain ASCII text. */
export function labelText(label: CommentLabel, withSection = true): string {
  return labelParts(label, withSection).join(' - ');
}

/* ---- proofs, from the text of a page ---- */

export interface RunLike {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

const PROOF = /^\s*(Proof|PROOF)(?=$|[\s.:,(])/;

/** Lines on a page that open a proof: a run reading "Proof" with nothing on
 * its line to its left. */
export function proofStarts(page: number, runs: readonly RunLike[]): ProofStart[] {
  const out: ProofStart[] = [];
  for (const run of runs) {
    if (!PROOF.test(run.text)) continue;
    const leftOf = runs.some(
      (other) =>
        other !== run &&
        other.text.trim() !== '' &&
        other.x + other.w <= run.x + 0.5 &&
        Math.min(other.y + other.h, run.y + run.h) - Math.max(other.y, run.y) >
          0.5 * Math.min(other.h, run.h)
    );
    if (!leftOf) out.push({ page, y: run.y + run.h });
  }
  return out.sort((a, b) => b.y - a.y);
}
