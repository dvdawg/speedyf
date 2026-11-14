/** The right column: the selected item's details, or, under a project with
 * nothing selected, that project's pane (an extension's, the notepad).
 *
 * The notepad is not a view of the project's items, so it is not a mode of
 * the middle column: the items stay listed beside it, where a row can be
 * pulled onto the note as a citation. Selecting an item shows its details
 * here instead, under one quiet line back to the note; that line, Escape in
 * this column, or a click on empty space in the list lets the item go.
 *
 * Under every other scope (All items, Not filed, Other PDFs, Health, New, a
 * person's items, a graph focus), and while no extension offers a pane, the
 * column is the details as it always was. */
import { createMemo, Show } from 'solid-js';
import { IconChevronLeft } from '../../components/icons';
import NoteEditor from '../extensions/editor/NoteEditor';
import { projectPane } from '../extensions/surfaces';
import PaperDetail from './PaperDetail';
import { projectsStore as store } from './projectsStore';

/** Nothing is picked for the detail column: no item, no missing work. */
export function nothingSelected(): boolean {
  return store.state.selectedId === null && store.state.ghostKey === null;
}

/** The pane the column shows right now, or null while it shows details. */
export function shownProjectPane() {
  const pane = projectPane();
  return pane && nothingSelected() ? pane : null;
}

/** Let the item go, back to the project's pane. */
export function backToProjectPane(): void {
  store.select(null);
  store.selectGhost(null);
}

export default function DetailColumn(props: { onMoveFocus: (dir: -1 | 1) => void }) {
  // Keyed on the pane and project, so moving to another project mounts its
  // note afresh and nothing reads the last one's after it is gone.
  const pane = createMemo(projectPane, null, {
    equals: (a, b) => a?.view.id === b?.view.id && a?.projectId === b?.projectId,
  });
  const projectName = (projectId: string) =>
    store.state.projects.find((p) => p.id === projectId)?.name ?? 'Project';

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || e.defaultPrevented || !pane() || nothingSelected()) return;
    e.preventDefault();
    backToProjectPane();
  };

  return (
    <Show when={pane()} keyed fallback={<PaperDetail onMoveFocus={props.onMoveFocus} />}>
      {(current) => (
        <Show
          when={nothingSelected()}
          fallback={
            <div class="project-pane-detail" onKeyDown={onKeyDown}>
              <button
                type="button"
                class="project-pane-back"
                title="Back to the note (Escape)"
                onClick={backToProjectPane}
              >
                <IconChevronLeft />
                <span>
                  {projectName(current.projectId)} {current.view.title.toLowerCase()}
                </span>
              </button>
              <PaperDetail onMoveFocus={props.onMoveFocus} />
            </div>
          }
        >
          <NoteEditor
            extensionId={current.view.extensionId}
            projectId={current.projectId}
            editor={current.view.editor}
            class="is-project-pane"
            header={<span class="note-editor-title">{current.view.title}</span>}
            placeholder={`Notes for ${projectName(current.projectId)}. Type @ to cite an item; drag items here.`}
          />
        </Show>
      )}
    </Show>
  );
}
