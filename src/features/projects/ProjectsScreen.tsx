/** The Projects launch screen: projects, papers, the selected paper.
 *
 * Three columns in a grid, each scrolling on its own. The screen itself
 * never scrolls, for the reason LibraryPanel gives: two nested scrollers
 * leave a wheel gesture with two things to move, and it moves the wrong one.
 *
 * Column-to-column keyboard moves are wired here because only the screen
 * knows its neighbours. Each column gets an `onMoveFocus`, and the screen
 * lands focus on whichever element in the next column currently carries
 * tabindex 0, which is what each column's roving tabindex maintains. Nothing
 * listens on the window.
 *
 * The store is initialized here and never disposed: it subscribes to
 * library:changed once, and the reader's Paper panel shares it. */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import LaunchToggle from './LaunchToggle';
import PaperDetail from './PaperDetail';
import PaperList from './PaperList';
import ProjectsColumn from './ProjectsColumn';
import { projectsStore } from './projectsStore';
import { IconClose } from '../../components/icons';

/** Below this the detail column folds behind a Details button; the sidebar
 * on Home is 240px and the same breakpoint is used there. */
const NARROW = '(max-width: 900px)';

export default function ProjectsScreen() {
  const store = projectsStore;
  const [narrow, setNarrow] = createSignal(false);
  const [detailsOpen, setDetailsOpen] = createSignal(false);
  const columns: (HTMLElement | undefined)[] = [];

  onMount(() => {
    void store.initialize();
    const query = window.matchMedia?.(NARROW);
    if (!query) return;
    const update = () => setNarrow(query.matches);
    update();
    query.addEventListener('change', update);
    onCleanup(() => query.removeEventListener('change', update));
  });

  const focusColumn = (index: number) => {
    const column = columns[index];
    if (!column) return;
    const target =
      column.querySelector<HTMLElement>('[tabindex="0"]') ??
      column.querySelector<HTMLElement>('button:not([disabled]), input, select, textarea');
    target?.focus();
  };

  const moveFocus = (from: number, dir: -1 | 1) => {
    const next = from + dir;
    if (next < 0 || next > 2) return;
    if (next === 2 && narrow() && !detailsOpen()) {
      setDetailsOpen(true);
      queueMicrotask(() => focusColumn(2));
      return;
    }
    focusColumn(next);
  };

  const scopeName = () => {
    const scope = store.state.scope;
    switch (scope.kind) {
      case 'all':
        return 'All items';
      case 'unfiled':
        return 'Not filed';
      case 'others':
        return 'Other PDFs';
      case 'project':
        return store.state.projects.find((p) => p.id === scope.id)?.name ?? 'Project';
    }
  };
  const countText = () => {
    const total = store.state.total;
    if (store.state.query.trim()) return `${total} matching`;
    // Under Other PDFs the rows are, by definition, not items.
    const noun = store.state.scope.kind === 'others' ? 'PDF' : 'item';
    return `${total} ${noun}${total === 1 ? '' : 's'}`;
  };

  return (
    <div
      class="projects-screen"
      classList={{ 'is-narrow': narrow(), 'is-details-open': detailsOpen() }}
    >
      <header class="projects-head">
        <h2>{scopeName()}</h2>
        <span class="projects-head-count">{countText()}</span>
        <span class="projects-head-spacer" />
        <Show when={narrow()}>
          <button
            type="button"
            class="secondary-btn"
            aria-pressed={detailsOpen()}
            onClick={() => setDetailsOpen(!detailsOpen())}
          >
            Details
          </button>
        </Show>
        <LaunchToggle />
      </header>
      <Show when={store.state.error}>
        {(error) => (
          <div class="projects-error" role="alert">
            <span class="projects-error-text">{error()}</span>
            <button
              type="button"
              class="projects-error-dismiss"
              aria-label="Dismiss"
              onClick={() => store.clearError()}
            >
              <IconClose />
            </button>
          </div>
        )}
      </Show>
      <div class="projects-columns">
        <section
          class="projects-col projects-col-projects"
          aria-label="Projects"
          ref={(el) => {
            columns[0] = el;
          }}
        >
          <ProjectsColumn onMoveFocus={(dir) => moveFocus(0, dir)} />
        </section>
        <section
          class="projects-col projects-col-papers"
          aria-label="Items"
          ref={(el) => {
            columns[1] = el;
          }}
        >
          <PaperList onMoveFocus={(dir) => moveFocus(1, dir)} />
        </section>
        <section
          class="projects-col projects-col-detail"
          aria-label="Item"
          ref={(el) => {
            columns[2] = el;
          }}
        >
          <PaperDetail onMoveFocus={(dir) => moveFocus(2, dir)} />
        </section>
      </div>
    </div>
  );
}
