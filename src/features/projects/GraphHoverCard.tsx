/** The small card beside a hovered or focused node in the citation graph.
 *
 * It shows what the row already knows at once (title, authors, the alpha
 * label, year and venue, whether the file is there) and what only the full record knows
 * (every author, the kind, the abstract) when that read arrives. The
 * abstract's room is not reserved before then: an item without one would
 * carry an empty gap forever, so the card grows once, with a fade.
 *
 * It never takes the pointer: the graph under it stays hoverable, and
 * moving from node to node swaps the content in place, without a remount. */
import { Show } from 'solid-js';
import type { MissingWork, Paper, PaperKind, PaperRow } from '../../types/engine';
import { authorsLine } from './paperFields';

/** Past this many authors the list ends in "et al.". */
const MAX_AUTHORS = 6;

const KIND_LABEL: Partial<Record<PaperKind, string>> = {
  article: 'Paper',
  inproceedings: 'Paper',
  thesis: 'Thesis',
  book: 'Book',
};

export function cardAuthors(row: PaperRow, paper: Paper | null | undefined): string {
  const list = paper?.meta.authors?.value ?? [];
  if (list.length === 0) return row.authors;
  const shown = authorsLine(list.slice(0, MAX_AUTHORS));
  return list.length > MAX_AUTHORS ? `${shown} et al.` : shown;
}

export default function GraphHoverCard(props: {
  id: string;
  row: PaperRow;
  /** The node's alpha label in this view, suffix included: "ZD24a". */
  label: string;
  /** Undefined while the record is being read; null when it could not be. */
  paper: Paper | null | undefined;
  cites: number;
  citedBy: number;
  left: number;
  top: number;
  width: number;
  side: 'left' | 'right';
  ref?: (el: HTMLDivElement) => void;
}) {
  const kind = () => {
    const value = props.paper?.meta.kind?.value;
    return value ? (KIND_LABEL[value] ?? null) : null;
  };
  const meta = () =>
    [props.row.year, props.row.venue ?? props.paper?.meta.venue?.value]
      .filter((part) => part !== null && part !== undefined && part !== '')
      .join(' · ');
  const abstract = () => props.paper?.meta.abstract?.value.trim() || null;
  const authors = () => cardAuthors(props.row, props.paper);

  return (
    <div
      ref={props.ref}
      id={props.id}
      class="graph-card"
      classList={{ 'is-left': props.side === 'left' }}
      role="tooltip"
      style={{
        left: `${props.left}px`,
        top: `${props.top}px`,
        width: `${props.width}px`,
      }}
    >
      <div class="graph-card-title">{props.row.title}</div>
      <Show when={authors()}>
        <div class="graph-card-authors">{authors()}</div>
      </Show>
      <Show when={kind() || meta() || props.label}>
        <div class="graph-card-meta">
          <Show when={kind()}>
            <span class="graph-card-kind">{kind()}</span>
          </Show>
          <Show when={props.label}>
            <span class="graph-card-alpha">{props.label}</span>
          </Show>
          <span>{meta()}</span>
        </div>
      </Show>
      <Show when={abstract()}>
        <p class="graph-card-abstract">{abstract()}</p>
      </Show>
      <div class="graph-card-foot">
        <span>
          cites {props.cites} · cited by {props.citedBy} in this view
        </span>
        <Show when={props.row.missing}>
          <span class="graph-card-missing">File missing</span>
        </Show>
      </div>
    </div>
  );
}

/** The card for a missing work: the reference as printed (clamped), what
 * identifies it, and which items here cite it. */
export function GhostHoverCard(props: {
  id: string;
  work: MissingWork;
  /** The citers' labels in this view. */
  citers: string[];
  left: number;
  top: number;
  width: number;
  side: 'left' | 'right';
  ref?: (el: HTMLDivElement) => void;
}) {
  const ids = () =>
    [
      props.work.year !== null ? String(props.work.year) : null,
      props.work.doi ? `doi ${props.work.doi}` : null,
      props.work.arxiv ? `arXiv ${props.work.arxiv}` : null,
    ].filter((part): part is string => part !== null);
  return (
    <div
      ref={props.ref}
      id={props.id}
      class="graph-card is-ghost"
      classList={{ 'is-left': props.side === 'left' }}
      role="tooltip"
      style={{
        left: `${props.left}px`,
        top: `${props.top}px`,
        width: `${props.width}px`,
      }}
    >
      <div class="graph-card-kind graph-card-ghost-kind">Not in the library</div>
      <p class="graph-card-reference">{props.work.text?.trim() || 'A reference with no text'}</p>
      <Show when={ids().length > 0}>
        <div class="graph-card-meta">
          <span>{ids().join(' · ')}</span>
        </div>
      </Show>
      <div class="graph-card-foot">
        <span>cited by: {props.citers.join(', ')}</span>
      </div>
    </div>
  );
}
