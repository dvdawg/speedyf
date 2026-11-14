/** The reader's right-hand drawer for an extension's editor
 * (`readerPanes`), its toolbar toggle, and the small project picker reader
 * actions ask with.
 *
 * The drawer sits beside the page in the tab's body, so opening it narrows
 * the page's column and the page re-fits, as it does for the search panel.
 * Its left edge drags to resize (pointer events; see projects/pointerDrag.ts
 * for why not drag and drop). It shows only while the document on screen
 * meets the pane's `when`. */
import { createMemo, createResource, For, onCleanup, onMount, Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { engine } from '../../lib/transport/engine';
import { formatShortcut } from '../../app/shortcutLabel';
import ExtensionIcon from './ExtensionIcon';
import NoteEditor from './editor/NoteEditor';
import { extensionIconPath, readerPanes } from './extensionStore';
import { projectQuestion, readerItem, readerProjectId } from './readerContext';
import {
  closeReaderPane,
  paneWidth,
  setPaneWidth,
  shownPane,
  switchReaderProject,
  toggleReaderPane,
} from './surfaces';

/** The toolbar's toggles: one quiet icon per pane offered for the document
 * on screen, and nothing at all for one that is not in a project. */
export function ReaderPaneToggles() {
  return (
    <For each={readerPanes()}>
      {(pane) => (
        <IconButton
          label={pane.key ? `${pane.title} (${formatShortcut(pane.key)})` : pane.title}
          active={shownPane()?.id === pane.id}
          onClick={() => toggleReaderPane(pane.id)}
        >
          <ExtensionIcon name={pane.title} iconPath={extensionIconPath(pane.extensionId)} />
        </IconButton>
      )}
    </For>
  );
}

export function ReaderPaneDrawer() {
  const pane = () => shownPane();
  const [projects] = createResource(
    () => readerItem()?.projectIds.join(',') ?? '',
    async () => engine.libraryProjects().catch(() => [])
  );
  const options = createMemo(() => {
    const ids = readerItem()?.projectIds ?? [];
    return (projects() ?? []).filter((p) => ids.includes(p.id));
  });
  const projectId = () => readerProjectId();
  const projectName = () => options().find((p) => p.id === projectId())?.name ?? null;

  let aside!: HTMLElement;
  const startResize = (e: PointerEvent) => {
    e.preventDefault();
    const right = aside.getBoundingClientRect().right;
    const move = (ev: PointerEvent) => setPaneWidth(right - ev.clientX);
    const finish = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      document.body.classList.remove('is-resizing-sidebar');
    };
    document.body.classList.add('is-resizing-sidebar');
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  };
  const onHandleKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 40 : 10;
    if (e.key === 'ArrowLeft') setPaneWidth(paneWidth() + step);
    else if (e.key === 'ArrowRight') setPaneWidth(paneWidth() - step);
    else return;
    e.preventDefault();
  };

  return (
    <Show when={pane()}>
      {(p) => (
        <aside
          ref={aside}
          class="reader-pane"
          style={{ width: `${paneWidth()}px` }}
          aria-label={p().title}
        >
          <div
            class="reader-pane-resize"
            role="separator"
            aria-orientation="vertical"
            aria-label={`Resize ${p().title}`}
            tabindex="0"
            onPointerDown={startResize}
            onKeyDown={onHandleKey}
          />
          <Show
            when={projectId()}
            fallback={
              <div class="reader-pane-pick">
                <div class="reader-pane-title">{p().title}</div>
                <p>This paper is in several projects. Whose {p().title.toLowerCase()}?</p>
                <For each={options()}>
                  {(option) => (
                    <button
                      type="button"
                      class="secondary-btn"
                      onClick={() => switchReaderProject(option.id)}
                    >
                      {option.name}
                    </button>
                  )}
                </For>
              </div>
            }
          >
            {(id) => (
              <NoteEditor
                extensionId={p().extensionId}
                projectId={id()}
                editor={p().editor}
                class="is-drawer"
                placeholder={`Notes for ${projectName() ?? 'this project'}. Type @ to cite an item.`}
                header={
                  <>
                    <span class="reader-pane-title" title={projectName() ?? ''}>
                      {projectName() ?? 'Project'} {p().title.toLowerCase()}
                    </span>
                    <Show when={options().length > 1}>
                      <select
                        class="reader-pane-switch"
                        aria-label="Project"
                        value={id()}
                        onChange={(e) => switchReaderProject(e.currentTarget.value)}
                      >
                        <For each={options()}>
                          {(option) => <option value={option.id}>{option.name}</option>}
                        </For>
                      </select>
                    </Show>
                  </>
                }
                end={
                  <button
                    type="button"
                    class="reader-pane-close"
                    aria-label={`Close ${p().title}`}
                    onClick={() => closeReaderPane()}
                  >
                    <IconClose />
                  </button>
                }
              />
            )}
          </Show>
        </aside>
      )}
    </Show>
  );
}

/** "Which project?" for a reader action on a paper in several. */
export function ProjectQuestion() {
  return (
    <Show when={projectQuestion()}>
      {(q) => {
        let list!: HTMLDivElement;
        onMount(() => {
          list.querySelector<HTMLButtonElement>('button')?.focus();
          const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              q().answer(null);
            }
          };
          window.addEventListener('keydown', onKey, true);
          onCleanup(() => window.removeEventListener('keydown', onKey, true));
        });
        return (
          <div class="project-question" role="dialog" aria-label="Which project?">
            <div class="project-question-card" ref={list}>
              <div class="project-question-title">Which project?</div>
              <For each={q().options}>
                {(option) => (
                  <button
                    type="button"
                    class="project-question-row"
                    onClick={() => q().answer(option.id)}
                    onKeyDown={(e) => {
                      const rows = [...list.querySelectorAll<HTMLButtonElement>('button')];
                      const at = rows.indexOf(e.currentTarget);
                      if (e.key === 'ArrowDown') rows[(at + 1) % rows.length]?.focus();
                      else if (e.key === 'ArrowUp')
                        rows[(at - 1 + rows.length) % rows.length]?.focus();
                      else return;
                      e.preventDefault();
                    }}
                  >
                    {option.name}
                  </button>
                )}
              </For>
              <div class="project-question-hint">Kept for this session. Escape cancels.</div>
            </div>
          </div>
        );
      }}
    </Show>
  );
}
