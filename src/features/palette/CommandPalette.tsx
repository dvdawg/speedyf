/** The command palette (⌘⇧P).
 *
 * One entry point to every action in the registry plus the tabs that are open,
 * ranked by what was typed and what gets used. Top-anchored rather than
 * centered: the list changes height on every keystroke, and centering would
 * move the row under the cursor as it did.
 */
import { batch, createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { COMMANDS, describeDisabled, formatShortcut, type CommandCtx } from '../../app/commands';
import { activeTab, tabsStore } from '../../stores/tabsStore';
import { activateTab } from '../document/tabsController';
import { jumpToAnchor } from '../outline/jumpToAnchor';
import { flattenOutline } from '../outline/structureStore';
import { closePalette, mostUsed, paletteOpen, recordUse, usesOf } from './paletteStore';
import {
  defaultSections,
  flatten,
  pageJumpTarget,
  rankCandidates,
  type Candidate,
  type Section,
} from './paletteItems';

export default function CommandPalette() {
  const [query, setQuery] = createSignal('');
  const [cursor, setCursor] = createSignal(0);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;

  const ctx = (): CommandCtx => ({ tab: activeTab() });

  /** Commands plus the tabs currently open. Nothing here triggers a fetch:
   * the palette must never make opening it cost a page extraction. */
  const candidates = createMemo<Candidate[]>(() => {
    if (!paletteOpen()) return [];
    const context = ctx();
    const items: Candidate[] = COMMANDS.map((command) => ({
      key: command.id,
      title: command.title,
      group: command.group,
      ...(command.keywords ? { keywords: command.keywords } : {}),
      ...(command.shortcut ? { shortcut: formatShortcut(command.shortcut) } : {}),
      enabled: command.enabled(context),
      reason: describeDisabled(command, context),
      run: () => command.run(context),
    }));

    for (const tab of tabsStore.state.tabs) {
      const name = tab.documentStore.state.name;
      if (!name) continue;
      items.push({
        key: `tab:${tab.id}`,
        title: name,
        group: 'Tabs',
        keywords: ['switch', 'tab'],
        enabled: true,
        run: () => activateTab(tab.id),
      });
    }

    // Sections and environments from the active document. Both are read from
    // the tab's structure store, never fetched here — the outline was loaded
    // when the document opened, and environments appear only once the panel
    // has paid for that pass.
    const tab = context.tab;
    if (tab) {
      const doc = tab.documentStore.state;
      const structure = tab.structureStore.state;

      for (const { node } of flattenOutline(structure.outline)) {
        const title = node.title.trim();
        const page = node.page;
        // A node with no destination is a container, not somewhere to go.
        if (!title || page === null) continue;
        items.push({
          key: `sec:${page}:${title}`,
          title,
          group: 'Sections',
          context: `p. ${page + 1}`,
          enabled: true,
          run: () => jumpToAnchor(tab.viewport, doc, page, node.y),
        });
      }

      for (const entry of structure.formal) {
        const label = entry.label.trim();
        if (!label) continue;
        items.push({
          key: `env:${entry.page}:${entry.charIndex}`,
          title: label,
          group: entry.heading ? 'Sections' : 'Environments',
          context: `p. ${entry.page + 1}`,
          enabled: true,
          run: () => jumpToAnchor(tab.viewport, doc, entry.page, entry.y),
        });
      }
    }
    return items;
  });

  const sections = createMemo<Section[]>(() => {
    const text = query();
    const base = candidates();
    if (text.trim().length === 0) return defaultSections(base, mostUsed(8));

    const ranked = rankCandidates(base, text, usesOf);
    const tab = activeTab();
    const pages = tab?.documentStore.state.pages.length ?? 0;
    const page = pageJumpTarget(text, pages);
    if (page === null || !tab) return ranked;
    // A bare number outranks everything: nobody types "12" hoping for a
    // command whose name happens to contain a 1 and a 2.
    return [
      {
        group: 'Jump',
        items: [
          {
            key: `page:${page}`,
            title: `Go to page ${page}`,
            group: 'Jump',
            enabled: true,
            run: () => tab.viewport.requestScrollToPage(page - 1),
          },
        ],
      },
      ...ranked,
    ];
  });

  const rows = createMemo(() => flatten(sections()));

  /** Sections with each row's position in the flat list, so keyboard movement
   * crosses group boundaries without the render pass having to count. */
  const indexed = createMemo(() => {
    let at = 0;
    return sections().map((section) => ({
      group: section.group,
      items: section.items.map((item) => ({ item, index: at++ })),
    }));
  });

  /** Keep the cursor inside the list without sending it home.
   *
   * This used to reset to 0 whenever `rows()` changed *identity* — and
   * `flatten` returns a fresh array every time anything upstream re-runs, so
   * unrelated store activity threw the selection back mid-navigation. Only
   * typing moves it home now; a list that shrinks just clamps. */
  createEffect(() => {
    const total = rows().length;
    if (total > 0 && cursor() >= total) setCursor(total - 1);
  });

  /** Whether the pointer, rather than the keyboard, is driving the selection.
   *
   * `scrollIntoView` slides a row under a stationary mouse, which fires
   * `pointerenter` and drags the selection straight back — arrow keys
   * fighting a cursor nobody touched. Hover only counts once the pointer has
   * actually moved. */
  const [pointerDriving, setPointerDriving] = createSignal(false);
  const onPointerMove = () => setPointerDriving(true);
  window.addEventListener('pointermove', onPointerMove, { passive: true });
  onCleanup(() => window.removeEventListener('pointermove', onPointerMove));

  createEffect(() => {
    if (paletteOpen()) {
      batch(() => {
        setQuery('');
        setCursor(0);
      });
      queueMicrotask(() => input?.focus());
    }
  });

  /** Keep the selected row visible without yanking the list around it. */
  createEffect(() => {
    const index = cursor();
    if (!paletteOpen() || !list) return;
    const el = list.querySelector<HTMLElement>(`[data-row="${index}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  });

  const runAt = (index: number) => {
    const row = rows()[index];
    if (!row || !row.enabled) return;
    recordUse(row.key);
    closePalette();
    row.run();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const total = rows().length;
    if (e.key === 'Escape') {
      e.preventDefault();
      closePalette();
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      runAt(cursor());
      return;
    }
    if (total === 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End') {
      setPointerDriving(false);
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setCursor((c) => (c + 1) % total);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setCursor((c) => (c - 1 + total) % total);
    } else if (e.key === 'Home') {
      e.preventDefault();
      setCursor(0);
    } else if (e.key === 'End') {
      e.preventDefault();
      setCursor(total - 1);
    }
  };

  return (
    <Show when={paletteOpen()}>
      <div
        class="modal-backdrop palette-backdrop"
        onPointerDown={(e) => {
          if (e.target === e.currentTarget) closePalette();
        }}
      >
        <div
          class="palette"
          role="dialog"
          aria-modal="true"
          aria-label="Command palette"
          onKeyDown={onKeyDown}
        >
          <input
            ref={input}
            class="palette-input"
            type="text"
            role="combobox"
            aria-expanded={rows().length > 0}
            aria-controls="palette-list"
            aria-activedescendant={rows().length > 0 ? `palette-row-${cursor()}` : undefined}
            placeholder="Search commands, tabs, pages…"
            spellcheck={false}
            autocomplete="off"
            value={query()}
            onInput={(e) =>
              batch(() => {
                setQuery(e.currentTarget.value);
                setCursor(0);
              })
            }
          />
          <div class="palette-list" id="palette-list" role="listbox" ref={list} tabindex="-1">
            <Show
              when={rows().length > 0}
              fallback={<div class="palette-empty">No matching commands</div>}
            >
              <For each={indexed()}>
                {(section) => (
                  <>
                    <div class="palette-group">{section.group}</div>
                    <For each={section.items}>
                      {({ item, index: at }) => (
                        <button
                          type="button"
                          data-row={at}
                          id={`palette-row-${at}`}
                          role="option"
                          aria-selected={cursor() === at}
                          class="palette-row"
                          classList={{
                            'is-active': cursor() === at,
                            'is-disabled': !item.enabled,
                          }}
                          disabled={!item.enabled}
                          onPointerEnter={() => pointerDriving() && setCursor(at)}
                          onClick={() => runAt(at)}
                        >
                          <span class="palette-title">{item.title}</span>
                          <Show when={item.context}>
                            <span class="palette-context">{item.context}</span>
                          </Show>
                          <Show
                            when={item.enabled}
                            fallback={<span class="palette-reason">{item.reason}</span>}
                          >
                            <Show when={item.shortcut}>
                              <span class="palette-keys">{item.shortcut}</span>
                            </Show>
                          </Show>
                        </button>
                      )}
                    </For>
                  </>
                )}
              </For>
            </Show>
          </div>
          <div class="palette-foot" aria-hidden="true">
            <span>
              <b>↑↓</b> navigate
            </span>
            <span>
              <b>↵</b> run
            </span>
            <span>
              <b>esc</b> close
            </span>
          </div>
        </div>
      </div>
    </Show>
  );
}
