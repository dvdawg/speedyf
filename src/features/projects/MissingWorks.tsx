/** Works the view's items cite that the library does not hold: the graph's
 * ghost nodes, the panel that stands in for the detail column while one is
 * picked, and the plain list the graph's notes lead to.
 *
 * A missing work is a reference as printed, not a record, so what can be
 * done with one is honest about that: open its DOI or arXiv page, copy the
 * reference, see who cites it. Looking it up needs a record to write the
 * answer into, so Look up says to add the file first. */
import { createSignal, For, onCleanup, Show } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import { settings } from '../../stores/settings';
import type { MissingWork } from '../../types/engine';
import { projectsStore } from './projectsStore';
import MathText from '../../components/MathText';
import { CitedWithRows } from './CitedWith';

/** The ghost node's id in the graph: never a record id. */
export const ghostId = (key: string) => `missing:${key}`;

export const doiUrl = (doi: string) => `https://doi.org/${encodeURI(doi)}`;
export const arxivUrl = (arxiv: string) => `https://arxiv.org/abs/${encodeURI(arxiv)}`;

/** The reference as printed, or what is known of it. */
export function referenceText(work: MissingWork): string {
  const text = work.text?.trim();
  if (text) return text;
  const parts = [
    work.doi ? `doi ${work.doi}` : null,
    work.arxiv ? `arXiv ${work.arxiv}` : null,
    work.year !== null ? String(work.year) : null,
  ].filter((p): p is string => p !== null);
  return parts.length > 0 ? parts.join(', ') : 'A reference with no text';
}

/** "Missing, cited by 3": the ghost's label, shown only on hover or
 * selection so a graph with many ghosts stays calm. */
export const ghostLabel = (work: MissingWork) => `Missing, cited by ${work.citedBy.length}`;

/** The citers by the graph's labels ("ZD24a"), falling back to the list's. */
export function citerLabel(id: string): string {
  const store = projectsStore;
  return (
    store.state.graphLabels[id] ??
    store.state.rows.find((r) => r.id === id)?.alphaLabel.trim() ??
    (store.state.selected?.id === id ? store.state.selected.citeKey : undefined) ??
    '?'
  );
}

export function missingMeta(work: MissingWork): string {
  return [
    work.year !== null ? String(work.year) : null,
    `cited by ${work.citedBy.map(citerLabel).join(', ')}`,
  ]
    .filter((p): p is string => p !== null)
    .join(', ');
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function MissingWorkActions(props: { work: MissingWork }) {
  const store = projectsStore;
  const [copied, setCopied] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  const open = (url: string) => {
    engine.openExternalUrl(url).catch((error: unknown) => store.reportError(error));
  };
  const copy = async () => {
    if (!(await copyText(referenceText(props.work)))) {
      store.reportError('Could not write to the clipboard');
      return;
    }
    setCopied(true);
    clearTimeout(timer);
    timer = setTimeout(() => setCopied(false), 1_200);
  };
  return (
    <>
      <div class="paper-actions missing-actions">
        <Show when={props.work.doi}>
          {(doi) => (
            <button type="button" class="secondary-btn" onClick={() => open(doiUrl(doi()))}>
              Open DOI
            </button>
          )}
        </Show>
        <Show when={props.work.arxiv}>
          {(arxiv) => (
            <button type="button" class="secondary-btn" onClick={() => open(arxivUrl(arxiv()))}>
              Open arXiv
            </button>
          )}
        </Show>
        <button type="button" class="secondary-btn" onClick={() => void copy()}>
          {copied() ? 'Copied' : 'Copy reference'}
        </button>
        <Show when={settings.onlineLookup}>
          <button
            type="button"
            class="secondary-btn"
            disabled
            aria-describedby="missing-lookup-note"
            title="Add it to the library first"
          >
            Look up
          </button>
        </Show>
      </div>
      <Show when={settings.onlineLookup}>
        <p class="paper-notice missing-note" id="missing-lookup-note">
          Add it to the library first
        </p>
      </Show>
    </>
  );
}

/** The detail column while a missing work is picked. */
export function MissingWorkPanel(props: { work: MissingWork }) {
  const store = projectsStore;
  return (
    <section class="missing-panel" aria-label="Missing work">
      <span class="paper-field-label">Not in the library</span>
      <p class="missing-panel-text">
        <MathText text={referenceText(props.work)} />
      </p>
      <Show when={props.work.year !== null || props.work.doi || props.work.arxiv}>
        <dl class="missing-panel-ids">
          <Show when={props.work.year !== null}>
            <dt>Year</dt>
            <dd>{props.work.year}</dd>
          </Show>
          <Show when={props.work.doi}>
            <dt>DOI</dt>
            <dd class="is-mono">{props.work.doi}</dd>
          </Show>
          <Show when={props.work.arxiv}>
            <dt>arXiv</dt>
            <dd class="is-mono">{props.work.arxiv}</dd>
          </Show>
        </dl>
      </Show>
      <div class="missing-panel-citers">
        <span class="paper-field-label">Cited by</span>
        <span>
          <For each={props.work.citedBy}>
            {(id, i) => (
              <>
                {i() > 0 ? ', ' : ''}
                <button
                  type="button"
                  class="paper-verdict-link is-mono"
                  title="Show this item"
                  onClick={() => store.select(id)}
                >
                  {citerLabel(id)}
                </button>
              </>
            )}
          </For>
        </span>
      </div>
      <MissingWorkActions work={props.work} />
    </section>
  );
}

/** The middle column's list of missing works, reached from the graph. In
 * a project it has a second mode, the works often cited with the members
 * (CitedWith.tsx); the switch picks which. */
export function MissingList() {
  const store = projectsStore;
  const works = () => store.state.missingWorks;
  const inProject = () => store.state.scope.kind === 'project';
  const mode = () => (inProject() ? store.state.missingMode : 'by');
  const head = () => {
    if (mode() === 'with') {
      const n = store.state.coCited.length;
      return `${n} ${n === 1 ? 'work' : 'works'} often cited with these, not in the project`;
    }
    return `${works().length} ${works().length === 1 ? 'work' : 'works'} cited by $\\geq ${store.state.missing.min}$, not in the library`;
  };
  return (
    <div class="projects-col-scroll missing-list">
      <Show when={inProject()}>
        <div class="missing-list-switch">
          <div class="projects-launch-toggle" role="radiogroup" aria-label="Which works">
            <For each={['by', 'with'] as const}>
              {(m) => (
                <button
                  type="button"
                  role="radio"
                  aria-checked={mode() === m}
                  classList={{ 'is-active': mode() === m }}
                  onClick={() => store.setMissingMode(m)}
                >
                  {m === 'by' ? 'Cited by these' : 'Cited with these'}
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
      <div class="missing-list-head">
        <span>
          <MathText text={head()} />
        </span>
        <button
          type="button"
          class="paper-verdict-link"
          onClick={() => store.showMissingList(false)}
        >
          Back to graph
        </button>
      </div>
      <Show when={mode() === 'with'}>
        <CitedWithRows />
      </Show>
      <Show
        when={mode() === 'by' && works().length > 0}
        fallback={
          <Show when={mode() === 'by'}>
            <p class="library-hint missing-list-empty">Nothing missing at this threshold</p>
          </Show>
        }
      >
        <div role="listbox" aria-label="Missing works">
          <For each={works()}>
            {(work) => (
              <div
                class="paper-row missing-row"
                classList={{ 'is-selected': store.state.ghostKey === work.key }}
                role="option"
                aria-selected={store.state.ghostKey === work.key}
                tabindex={0}
                onClick={() => store.selectGhost(work.key)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    store.selectGhost(work.key);
                  }
                }}
              >
                <span class="paper-row-main">
                  <MathText class="missing-row-text" text={referenceText(work)} />
                  <span class="paper-row-meta">{missingMeta(work)}</span>
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
