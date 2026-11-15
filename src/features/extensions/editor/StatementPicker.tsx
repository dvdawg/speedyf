/** The `@` picker's second step: after an item is chosen (or `@key,` is
 * typed), the paper's formal statements and numbered equations, to cite
 * one as `[@key, Prop. 2.2]{box=x,y,w,h page=3}` - read "[RZ26, Prop.
 * 2.2]", opening the paper at the statement. The paper need not be open;
 * its statements are read (and cached by the engine) on demand. */
import { For, Show } from 'solid-js';
import MathText from '../../../components/MathText';
import type { StatementPlace } from '../paperDocs';

/** How a citation names a statement: "Thm." for Theorem, "Eq. (3)". */
const ABBREVIATIONS: Record<string, string> = {
  theorem: 'Thm.',
  lemma: 'Lem.',
  proposition: 'Prop.',
  corollary: 'Cor.',
  definition: 'Def.',
  remark: 'Rem.',
  example: 'Ex.',
  conjecture: 'Conj.',
  claim: 'Claim',
  assumption: 'Assump.',
};

/** The locator a citation of `statement` carries: "Prop. 2.2", "Eq. (3)". */
export function statementLocator(statement: {
  kind: string;
  word: string;
  number: string;
}): string {
  if (statement.kind === 'equation') return `Eq. (${statement.number})`;
  return `${ABBREVIATIONS[statement.kind] ?? statement.word} ${statement.number}`;
}

/** The citation of a statement as the note stores it: the locator readable,
 * the box and page (one-based) for SpeedyF. */
export function statementCite(key: string, place: StatementPlace): string {
  const [x, y, w, h] = place.box;
  return `[@${key}, ${statementLocator(place.statement)}]{box=${x},${y},${w},${h} page=${place.statement.page + 1}}`;
}

/** How a row reads: "Proposition 2.2", "Equation (3)". */
export function statementName(statement: { kind: string; word: string; number: string }): string {
  return statement.kind === 'equation'
    ? `Equation (${statement.number})`
    : `${statement.word} ${statement.number}`;
}

/** A citation's statement step being typed before `caret` in `text`:
 * `@key,` and what follows the comma so far. */
export function statementQuery(
  text: string,
  caret: number
): { at: number; key: string; query: string } | null {
  const before = text.slice(Math.max(0, caret - 120), caret);
  const m =
    /(^|[\s([{;,"'])@([A-Za-z0-9_][A-Za-z0-9_:.-]*[A-Za-z0-9_]|[A-Za-z0-9_]),\s*([^\n@\]]{0,40})$/.exec(
      before
    );
  if (!m) return null;
  const at = caret - before.length + m.index + m[1]!.length;
  return { at, key: m[2]!, query: m[3]! };
}

/** The statements matching what was typed after the comma: by kind,
 * number and name ("prop 2.2", "2.2", "overlaps"), at most 12, in
 * document order. */
export function matchStatements(list: readonly StatementPlace[], query: string): StatementPlace[] {
  const words = query
    .toLowerCase()
    .split(/[\s,]+/)
    .filter(Boolean);
  const hit = (place: StatementPlace) => {
    const s = place.statement;
    const hay = `${s.word} ${statementLocator(s)} ${s.number} ${s.title ?? ''}`.toLowerCase();
    return words.every((w) => hay.includes(w.replace(/[()]/g, '')));
  };
  return list.filter(hit).slice(0, 12);
}

export interface StatementPick {
  at: number;
  key: string;
  item: string;
  query: string;
  /** The row chosen; -1 for none yet (the step offered after picking an
   * item: Enter there is still a line break). */
  index: number;
}

export default function StatementPicker(props: {
  rows: StatementPlace[];
  loading: boolean;
  index: number;
  onPick: (place: StatementPlace) => void;
}) {
  return (
    <div class="note-picker is-statements" role="listbox" aria-label="Cite a statement">
      <Show when={props.loading}>
        <div class="note-picker-row is-status" role="status">
          Reading statements...
        </div>
      </Show>
      <For each={props.rows}>
        {(place, i) => (
          <div
            role="option"
            class="note-picker-row"
            aria-selected={i() === props.index}
            classList={{ 'is-current': i() === props.index }}
            onMouseDown={(e) => {
              e.preventDefault();
              props.onPick(place);
            }}
          >
            <span class="note-picker-title">{statementName(place.statement)}</span>
            <span class="note-picker-detail">
              <Show when={place.statement.title} fallback={`p. ${place.statement.page + 1}`}>
                <MathText text={place.statement.title!} />
              </Show>
            </span>
          </div>
        )}
      </For>
    </div>
  );
}
