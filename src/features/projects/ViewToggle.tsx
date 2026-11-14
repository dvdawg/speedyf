/** List | Graph | (an extension's project views, e.g. Notepad): which view
 * of the scope the middle column shows. In the list and graph it sits in the
 * column's own head; an extension's view hides that head (its BibTeX and
 * lookup actions are about the list, not the writing), so the switch moves
 * up into the screen header beside the project's name. */
import { For, Show } from 'solid-js';
import { projectViews } from '../extensions/extensionStore';
import { VIEW_PREFIX, activeProjectView } from '../extensions/surfaces';
import { projectsStore as store, type ProjectsView } from './projectsStore';

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
  if (activeProjectView()) return store.state.view;
  const chosen = store.state.view.startsWith(VIEW_PREFIX) ? 'list' : store.state.view;
  return graphable() ? chosen : 'list';
}

function viewOptions(): { id: ProjectsView; label: string }[] {
  return store.state.scope.kind === 'project' && !store.state.person && !store.state.focus
    ? [
        ...VIEWS,
        ...projectViews().map((v) => ({
          id: `${VIEW_PREFIX}${v.id}` as ProjectsView,
          label: v.title,
        })),
      ]
    : VIEWS;
}

export default function ViewToggle() {
  return (
    <Show when={graphable()}>
      <div class="projects-launch-toggle paper-view-toggle" role="radiogroup" aria-label="View">
        <For each={viewOptions()}>
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
