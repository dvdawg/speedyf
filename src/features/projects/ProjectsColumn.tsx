/** The left column: the three fixed scopes, then the projects in their
 * stored order, then the way to make one.
 *
 * Everything a project needs done to it happens here and in place: rename on
 * double-click or F2, a small menu for the rest, a two-step confirm for
 * delete inside the row itself. No browser dialogs: the screen is a list,
 * and a modal over a list is a modal over the thing you were looking at.
 *
 * Rows are dropped onto a project row from the middle column; projects are
 * dragged among themselves to reorder. Both are pointer-event drags
 * (pointerDrag.ts), never HTML5 ones: with native file drops on, WebKit
 * would hand an HTML5 drag to Tauri and the row would never land. A project
 * row says it takes drops with `data-drop-project`, and the hover to
 * highlight comes from the one signal in dnd.ts, since a row drag starts in
 * another column. Keyboard handling lives on the list element with a roving
 * tabindex: the active scope is the one row in the tab order, and arrows
 * move the scope itself, the way a sidebar does. */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { IconMore, IconPlus } from '../../components/icons';
import type { Project } from '../../types/engine';
import { exportBib, exportBundle, stopSyncingBib, syncBib } from './bibActions';
import { dropHover, reorderSlotAt, setDropHover } from './dnd';
import { beginPointerDrag } from './pointerDrag';
import { projectsStore, sameScope } from './projectsStore';
import type { Scope } from './projectsStore';
import { newProjectRequested, takeNewProjectRequest } from './screenIntents';

/** A text field that commits on Enter or blur and cancels on Escape. Used for
 * renaming and for naming a new project, so the two cannot behave
 * differently. Blur commits because a name typed and then clicked away from
 * is a name the user meant; Escape is the explicit way out. */
function InlineInput(props: {
  initial: string;
  label: string;
  placeholder?: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  let input!: HTMLInputElement;
  // Enter followed by the blur that removing the field causes must not
  // commit twice, and Escape followed by that blur must not commit at all.
  let done = false;
  const finish = (value: string | null) => {
    if (done) return;
    done = true;
    if (value === null) props.onCancel();
    else props.onCommit(value);
  };
  onMount(() => {
    input.focus();
    input.select();
  });
  return (
    <input
      ref={input}
      type="text"
      class="projects-inline-input"
      value={props.initial}
      placeholder={props.placeholder}
      aria-label={props.label}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          e.stopPropagation();
          finish(e.currentTarget.value);
        } else if (e.key === 'Escape') {
          e.preventDefault();
          e.stopPropagation();
          finish(null);
        }
      }}
      onBlur={(e) => finish(e.currentTarget.value)}
    />
  );
}

/** The per-project menu. Closes on Escape, on a click anywhere else, and
 * before any item runs, since half the items replace the row underneath. */
function ProjectMenu(props: {
  project: Project;
  onClose: () => void;
  onRename: () => void;
  onDelete: () => void;
}) {
  let menu!: HTMLDivElement;
  const items = (): { label: string; run: () => void; danger?: boolean }[] => [
    { label: 'Rename', run: props.onRename },
    { label: 'Export .bib...', run: () => void exportBib(props.project) },
    { label: 'Export bundle (.zip)...', run: () => void exportBundle(props.project) },
    props.project.bibPath
      ? { label: 'Stop syncing .bib', run: () => void stopSyncingBib(props.project) }
      : { label: 'Sync .bib...', run: () => void syncBib(props.project) },
    { label: 'Delete', run: props.onDelete, danger: true },
  ];
  const focusItem = (delta: number) => {
    const buttons = Array.from(menu.querySelectorAll<HTMLButtonElement>('button'));
    const current = buttons.findIndex((b) => b === document.activeElement);
    const next = current < 0 ? 0 : (current + delta + buttons.length) % buttons.length;
    buttons[next]?.focus();
  };
  onMount(() => {
    focusItem(0);
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      // The toggle that opened this menu closes it on click; closing here
      // too would let that click reopen it.
      if (target?.closest('.projects-row-menu')) return;
      if (!menu.contains(e.target as Node)) props.onClose();
    };
    window.addEventListener('pointerdown', onPointerDown, true);
    onCleanup(() => window.removeEventListener('pointerdown', onPointerDown, true));
  });
  return (
    <div
      ref={menu}
      class="projects-menu"
      role="menu"
      aria-label={`Actions for ${props.project.name}`}
      onKeyDown={(e) => {
        // The list behind the menu moves the scope on arrows; this menu is
        // the only thing the keys should reach while it is open.
        e.stopPropagation();
        if (e.key === 'Escape') {
          e.preventDefault();
          props.onClose();
        } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          focusItem(e.key === 'ArrowDown' ? 1 : -1);
        }
      }}
    >
      <For each={items()}>
        {(item) => (
          <button
            type="button"
            class="projects-menu-item"
            classList={{ 'is-danger': item.danger === true }}
            role="menuitem"
            onClick={() => {
              props.onClose();
              item.run();
            }}
          >
            {item.label}
          </button>
        )}
      </For>
    </div>
  );
}

export default function ProjectsColumn(props: { onMoveFocus: (dir: -1 | 1) => void }) {
  const store = projectsStore;
  const [renaming, setRenaming] = createSignal<string | null>(null);
  const [menuFor, setMenuFor] = createSignal<string | null>(null);
  const [confirming, setConfirming] = createSignal<string | null>(null);
  const [creating, setCreating] = createSignal(false);
  let list!: HTMLDivElement;

  // New Project... from the palette: the same field the button opens.
  createEffect(() => {
    if (!newProjectRequested()) return;
    takeNewProjectRequest();
    setCreating(true);
  });

  const scopes = (): Scope[] => [
    { kind: 'all' },
    { kind: 'unfiled' },
    { kind: 'others' },
    ...store.state.projects.map((p): Scope => ({ kind: 'project', id: p.id })),
  ];
  /** Rows before the first project, which is also the first project's index. */
  const FIXED = 3;
  const activeIndex = () =>
    Math.max(
      0,
      scopes().findIndex((s) => sameScope(s, store.state.scope))
    );
  const isActive = (scope: Scope) => sameScope(scope, store.state.scope);

  const focusRow = (index: number) =>
    list.querySelector<HTMLElement>(`[data-index="${index}"]`)?.focus();

  const move = (delta: number) => {
    const all = scopes();
    const next = Math.max(0, Math.min(all.length - 1, activeIndex() + delta));
    store.setScope(all[next]!);
    focusRow(next);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const tag = (e.target as HTMLElement).tagName;
    if (tag === 'INPUT') return;
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        move(1);
        break;
      case 'ArrowUp':
        e.preventDefault();
        move(-1);
        break;
      case 'ArrowRight':
      case 'Enter':
        if (tag === 'BUTTON') return;
        e.preventDefault();
        props.onMoveFocus(1);
        break;
      case 'F2': {
        const scope = store.state.scope;
        if (scope.kind === 'project') {
          e.preventDefault();
          setRenaming(scope.id);
        }
        break;
      }
    }
  };

  const create = async (name: string) => {
    const project = await store.createProject(name);
    if (project) store.setScope({ kind: 'project', id: project.id });
  };

  const rename = (project: Project, value: string) => {
    setRenaming(null);
    const name = value.trim();
    if (name && name !== project.name) void store.renameProject(project.id, name);
  };

  /** Reorder: the dragged project lands before or after the row it was
   * dropped on. Nothing is written when that is where it already was. */
  const reorder = (movedId: string, slot: { id: string; after: boolean }) => {
    const before = store.state.projects.map((p) => p.id);
    const ids = before.filter((id) => id !== movedId);
    ids.splice(ids.indexOf(slot.id) + (slot.after ? 1 : 0), 0, movedId);
    if (ids.every((id, i) => id === before[i])) return;
    void store.reorderProjects(ids);
  };

  /** A press on a project row may become a reorder drag. The row itself is
   * no target: an insertion line on the thing being moved says nothing. */
  const onRowPointerDown = (project: Project, e: PointerEvent) => {
    if (renaming() === project.id || confirming() === project.id) return;
    beginPointerDrag(e, {
      label: project.name,
      targetAt: (x, y) => {
        const slot = reorderSlotAt(x, y);
        return slot && slot.id !== project.id ? slot : null;
      },
      onHover: (slot) => setDropHover(slot ? { kind: 'slot', ...slot } : null),
      onDrop: (slot) => reorder(project.id, slot),
    });
  };

  const hoverInto = (id: string) => {
    const h = dropHover();
    return h?.kind === 'into' && h.id === id;
  };
  const hoverSlot = (id: string, after: boolean) => {
    const h = dropHover();
    return h?.kind === 'slot' && h.id === id && h.after === after;
  };

  const fixedRow = (scope: Scope, index: number, label: string, count: number, title?: string) => (
    <div
      class="projects-row"
      classList={{ 'is-active': isActive(scope) }}
      role="option"
      aria-selected={isActive(scope)}
      tabindex={isActive(scope) ? 0 : -1}
      data-index={index}
      title={title}
      onClick={() => store.setScope(scope)}
    >
      <span class="projects-row-name">{label}</span>
      <span class="projects-row-count">{count}</span>
    </div>
  );

  return (
    <>
      <div
        ref={list}
        class="projects-col-scroll projects-list"
        role="listbox"
        aria-label="Projects"
        onKeyDown={onKeyDown}
      >
        {fixedRow({ kind: 'all' }, 0, 'All items', store.state.counts.all)}
        {fixedRow({ kind: 'unfiled' }, 1, 'Not filed', store.state.counts.unfiled)}
        {/* The files the scan did not take for papers: a manual, a form, a
            slide deck. Kept out of the two rows above, since a library folder
            like Downloads holds more PDFs than papers, and listed here so
            nothing is hidden and a wrong verdict can be fixed. */}
        {fixedRow(
          { kind: 'others' },
          2,
          'Other PDFs',
          store.state.counts.others,
          'PDFs in the library folders that are not treated as items'
        )}
        <div class="projects-rule" role="presentation" />
        <For each={store.state.projects}>
          {(project, index) => {
            const scope = (): Scope => ({ kind: 'project', id: project.id });
            const active = () => isActive(scope());
            return (
              <div
                class="projects-row is-project"
                classList={{
                  'is-active': active(),
                  'is-drop-target': hoverInto(project.id),
                  'is-drop-before': hoverSlot(project.id, false),
                  'is-drop-after': hoverSlot(project.id, true),
                  'is-confirming': confirming() === project.id,
                }}
                role="option"
                aria-selected={active()}
                tabindex={active() ? 0 : -1}
                data-index={index() + FIXED}
                data-drop-project={project.id}
                onClick={() => store.setScope(scope())}
                onDblClick={() => setRenaming(project.id)}
                onPointerDown={(e) => onRowPointerDown(project, e)}
              >
                <Show
                  when={confirming() === project.id}
                  fallback={
                    <Show
                      when={renaming() === project.id}
                      fallback={<span class="projects-row-name">{project.name}</span>}
                    >
                      <InlineInput
                        initial={project.name}
                        label={`Rename ${project.name}`}
                        onCommit={(value) => rename(project, value)}
                        onCancel={() => setRenaming(null)}
                      />
                    </Show>
                  }
                >
                  <div class="projects-confirm" role="group" aria-label={`Delete ${project.name}?`}>
                    <span class="projects-confirm-text">
                      Delete "{project.name}"? Its items stay in the library.
                    </span>
                    <button
                      type="button"
                      class="projects-confirm-yes"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirming(null);
                        void store.deleteProject(project.id);
                      }}
                    >
                      Delete
                    </button>
                    <button
                      type="button"
                      class="projects-confirm-no"
                      onClick={(e) => {
                        e.stopPropagation();
                        setConfirming(null);
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                </Show>
                <Show when={renaming() !== project.id && confirming() !== project.id}>
                  <span class="projects-row-count">{project.paperCount}</span>
                  <button
                    type="button"
                    class="projects-row-menu"
                    aria-label={`Actions for ${project.name}`}
                    aria-haspopup="menu"
                    aria-expanded={menuFor() === project.id}
                    tabindex={active() ? 0 : -1}
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuFor(menuFor() === project.id ? null : project.id);
                    }}
                  >
                    <IconMore />
                  </button>
                </Show>
                <Show when={menuFor() === project.id}>
                  <ProjectMenu
                    project={project}
                    onClose={() => setMenuFor(null)}
                    onRename={() => setRenaming(project.id)}
                    onDelete={() => setConfirming(project.id)}
                  />
                </Show>
              </div>
            );
          }}
        </For>
      </div>
      <div class="projects-new-row">
        <Show
          when={creating()}
          fallback={
            <button type="button" class="projects-new" onClick={() => setCreating(true)}>
              <IconPlus /> New project
            </button>
          }
        >
          <InlineInput
            initial=""
            label="New project name"
            placeholder="Project name"
            onCommit={(value) => {
              setCreating(false);
              const name = value.trim();
              if (name) void create(name);
            }}
            onCancel={() => setCreating(false)}
          />
        </Show>
      </div>
    </>
  );
}
