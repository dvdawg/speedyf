/** List | Graph: which view of the scope the middle column shows, in the
 * column's own head. Only these two: a project's notepad is not a view of
 * its items but a page of its own, and lives in the detail column
 * (ProjectPane.tsx). */
import { For, Show } from 'solid-js';
import { projectsStore as store, readView, type ProjectsView } from './projectsStore';

const VIEWS: { id: ProjectsView; label: string }[] = [
  { id: 'list', label: 'List' },
  { id: 'graph', label: 'Graph' },
];

/** Whether the scope has more than a list to show. */
export function graphable(): boolean {
  // A person's items are a list.
  if (store.state.person) return false;
  const kind = store.state.scope.kind;
  // A focus is one item's neighbourhood, which any item has.
  return kind === 'project' || kind === 'all' || store.state.focus !== null;
}

/** The view showing now. */
export function currentView(): ProjectsView {
  return graphable() ? readView(store.state.view) : 'list';
}

export default function ViewToggle() {
  return (
    <Show when={graphable()}>
      <div class="projects-launch-toggle paper-view-toggle" role="radiogroup" aria-label="View">
        <For each={VIEWS}>
          {(option) => (
            <button
              type="button"
              role="radio"
              aria-checked={currentView() === option.id}
              classList={{ 'is-active': currentView() === option.id }}
              onClick={() => store.setView(option.id)}
            >
              {option.label}
            </button>
          )}
        </For>
      </div>
    </Show>
  );
}
