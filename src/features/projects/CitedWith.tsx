/** Works the library often cites alongside a project's members that are not
 * in it (projects.md sec. 8, "Cited with these"): the rows of the
 * missing-works list in its second mode, and the panel that stands in for
 * the detail column while one is picked. A library item can be added to
 * the project from either; a missing work is shown as printed, and what
 * can be done with it is what the missing-works panel offers. */
import { For, Show } from 'solid-js';
import type { CoCitedWork, MissingWork } from '../../types/engine';
import MathText from '../../components/MathText';
import { citerLabel, MissingWorkActions, referenceText } from './MissingWorks';
import { projectsStore, withKey } from './projectsStore';

/** The ghost node's id in the graph: never a record id, never a missing
 * work's ghost. */
export const withGhostId = (key: string) => `with:${key}`;

/** The missing-works shape of a cited-with work, for what is shared with
 * missing works (the reference text, the actions). */
export const asMissing = (work: CoCitedWork): MissingWork => ({
  key: work.key,
  text: work.text,
  year: work.year,
  doi: work.doi,
  arxiv: work.arxiv,
  citedBy: [],
});

/** The item's title, or the reference as printed. */
export function citedWithText(work: CoCitedWork): string {
  if (work.paperId !== null) return work.title?.trim() || 'Untitled item';
  return referenceText(asMissing(work));
}

/** "cited with 4 of these" */
export const citedWithReason = (work: CoCitedWork) => `cited with ${work.members.length} of these`;

/** "Cited with 4 of these": the ghost's label, on hover or pick only. */
export const citedWithLabel = (work: CoCitedWork) => `Cited with ${work.members.length} of these`;

/** The cited-with work picked in the graph or the list, if any. */
export function pickedCitedWith(): CoCitedWork | null {
  const key = projectsStore.state.ghostKey;
  if (!key?.startsWith('with:')) return null;
  return projectsStore.state.coCited.find((w) => withKey(w.key) === key) ?? null;
}

/** The project the list is for, or null outside one. */
const projectId = () => {
  const scope = projectsStore.state.scope;
  return scope.kind === 'project' ? scope.id : null;
};

function AddButton(props: { work: CoCitedWork }) {
  return (
    <Show when={props.work.paperId !== null && projectId() !== null}>
      <button
        type="button"
        class="secondary-btn cited-with-add"
        title="Add this item to the project"
        onClick={(e) => {
          e.stopPropagation();
          void projectsStore.setMembership(props.work.paperId!, projectId()!, true);
        }}
      >
        Add
      </button>
    </Show>
  );
}

/** The list's rows in "Cited with these" mode. */
export function CitedWithRows() {
  const store = projectsStore;
  const works = () => store.state.coCited;
  return (
    <Show
      when={works().length > 0}
      fallback={<p class="library-hint missing-list-empty">Nothing is often cited with these</p>}
    >
      <div role="listbox" aria-label="Cited with these">
        <For each={works()}>
          {(work) => {
            const key = () => withKey(work.key);
            return (
              <div
                class="paper-row missing-row"
                classList={{ 'is-selected': store.state.ghostKey === key() }}
                role="option"
                aria-selected={store.state.ghostKey === key()}
                tabindex={0}
                data-key={work.key}
                onClick={() => store.selectGhost(key())}
                onKeyDown={(e) => {
                  if (e.target !== e.currentTarget) return;
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    store.selectGhost(key());
                  }
                }}
              >
                <span class="paper-row-main">
                  <MathText class="missing-row-text" text={citedWithText(work)} />
                  <span class="paper-row-meta">
                    {[work.year !== null ? String(work.year) : null, citedWithReason(work)]
                      .filter((p): p is string => p !== null)
                      .join(', ')}
                  </span>
                </span>
                <AddButton work={work} />
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

/** The detail column while a cited-with work is picked. */
export function CitedWithPanel(props: { work: CoCitedWork }) {
  const store = projectsStore;
  const item = () => props.work.paperId;
  return (
    <section class="missing-panel" aria-label="Cited with these">
      <span class="paper-field-label">
        {item() !== null ? 'In the library, not in this project' : 'Not in the library'}
      </span>
      <p class="missing-panel-text">
        <MathText text={citedWithText(props.work)} />
      </p>
      <div class="missing-panel-citers">
        <span class="paper-field-label">Cited with</span>
        <span>
          <For each={props.work.members}>
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
      <p class="paper-notice missing-note">
        {`By ${props.work.citers} of ${props.work.citedInLibrary} items in the library that cite it`}
      </p>
      <Show when={item()} fallback={<MissingWorkActions work={asMissing(props.work)} />}>
        {(id) => (
          <div class="paper-actions missing-actions">
            <AddButton work={props.work} />
            <button type="button" class="secondary-btn" onClick={() => store.select(id())}>
              Show item
            </button>
          </div>
        )}
      </Show>
    </section>
  );
}
