/** What a paper's BibTeX entry is missing, said the way the entry will say
 * it, and the one fix worth offering for each gap.
 *
 * The engine names a gap by the metadata field that would fill it
 * (`PaperRecord::incomplete`): an article's journal and a conference
 * paper's proceedings are both its `venue`, a thesis's school its
 * `publisher`. The fix is the cheapest that could still work: a lookup
 * that has not run yet, by identifier or else by title, and once one has
 * run and left the gap, typing it in. */
import type { Paper, PaperKind } from '../../types/engine';

/** The name a gap goes by for an entry of this kind. */
export function gapLabel(field: string, kind: PaperKind | null | undefined): string {
  if (field === 'venue') {
    if (kind === 'article') return 'journal';
    if (kind === 'inproceedings') return 'proceedings';
    return 'venue';
  }
  if (field === 'publisher') return kind === 'thesis' ? 'school' : 'publisher';
  return field;
}

/** "no year, no venue": a list row's note in the incomplete filter. */
export function gapsText(fields: readonly string[]): string {
  return fields.map((field) => `no ${gapLabel(field, null)}`).join(', ');
}

export type GapFix =
  /** Fetch by the DOI or arXiv id the paper has. */
  | { kind: 'lookup' }
  /** Search by title, to pick a match. */
  | { kind: 'title' }
  /** Type it in; `lookedUp` when a lookup already ran and left the gap. */
  | { kind: 'edit'; lookedUp: boolean };

/** The fix for a gap: a lookup while one has not been tried and lookups
 * are allowed, else editing the field. */
export function gapFix(paper: Paper, online: boolean): GapFix {
  const lookedUp = Boolean(paper.lookedUpAt);
  if (online && paper.isPaper && !lookedUp) {
    return paper.doi || paper.arxiv ? { kind: 'lookup' } : { kind: 'title' };
  }
  return { kind: 'edit', lookedUp };
}
