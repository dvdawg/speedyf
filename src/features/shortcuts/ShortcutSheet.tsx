/** The keyboard shortcut sheet (Cmd+/, Ctrl+/ elsewhere): every key SpeedyF
 * and its extensions answer, read from the registries that bind them, with
 * a filter at the top. The palette's surface and type, without its cursor:
 * nothing here runs, it only says what does. */
import { createEffect, createMemo, createSignal, For, Show } from 'solid-js';
import { COMMANDS } from '../../app/commands';
import { boundShortcuts, reservedChords, UNLISTED_SHORTCUTS } from '../../app/shortcuts';
import { canonicalChord, parseChord } from '../extensions/capabilities';
import { allReaderPanes, extensionBindings, extensionState } from '../extensions/extensionStore';
import { closeShortcutSheet, shortcutSheetOpen } from './shortcutSheetStore';
import { filterShortcuts, shortcutSections, type ShortcutSources } from './shortcutRows';

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

/** Extension keybindings that survived conflict resolution, and the reader
 * pane keys the keymap would honour, each with its extension's name. */
function extensionShortcuts(): ShortcutSources['extensions'] {
  const records = new Map(extensionState.extensions.map((record) => [record.id, record]));
  const out: { spec: string; title: string; extension: string }[] = [];
  for (const binding of extensionBindings()) {
    const record = records.get(binding.extensionId);
    const qualified = `${binding.extensionId}/${binding.command}`;
    const named = [
      ...(record?.commands ?? []),
      ...(record?.selectionActions ?? []),
      ...(record?.documentTasks ?? []),
    ].find((entry) => entry.id === qualified || entry.localId === binding.command);
    out.push({
      spec: canonicalChord(binding.chord),
      title: named?.title ?? binding.command,
      extension: record?.name ?? binding.extensionId,
    });
  }
  const reserved = reservedChords();
  for (const pane of allReaderPanes()) {
    const chord = pane.key ? parseChord(pane.key) : null;
    if (!chord || reserved.has(canonicalChord(chord))) continue;
    out.push({
      spec: canonicalChord(chord),
      title: `Toggle ${pane.title}`,
      extension: records.get(pane.extensionId)?.name ?? pane.extensionId,
    });
  }
  return out;
}

export default function ShortcutSheet() {
  const [query, setQuery] = createSignal('');
  let input: HTMLInputElement | undefined;

  const sections = createMemo(() => {
    if (!shortcutSheetOpen()) return [];
    const all = shortcutSections(
      {
        commands: COMMANDS,
        bound: boundShortcuts(isMac),
        unlisted: UNLISTED_SHORTCUTS,
        extensions: extensionShortcuts(),
      },
      isMac
    );
    return filterShortcuts(all, query());
  });

  createEffect(() => {
    if (!shortcutSheetOpen()) return;
    setQuery('');
    queueMicrotask(() => input?.focus());
  });

  const onKeyDown = (e: KeyboardEvent) => {
    const toggle = e.key === '/' && (isMac ? e.metaKey : e.ctrlKey);
    if (e.key !== 'Escape' && !toggle) return;
    // Kept from the window's keymap, which would read Cmd+/ as "open".
    e.preventDefault();
    e.stopPropagation();
    closeShortcutSheet();
  };

  return (
    <Show when={shortcutSheetOpen()}>
      <div
        class="modal-backdrop palette-backdrop"
        onPointerDown={(e) => {
          if (e.target === e.currentTarget) closeShortcutSheet();
        }}
      >
        <div
          class="palette shortcut-sheet"
          role="dialog"
          aria-modal="true"
          aria-label="Keyboard shortcuts"
          onKeyDown={onKeyDown}
        >
          <input
            ref={input}
            class="palette-input"
            type="text"
            placeholder="Filter shortcuts..."
            aria-label="Filter shortcuts"
            spellcheck={false}
            autocomplete="off"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
          <div class="palette-list">
            <Show
              when={sections().length > 0}
              fallback={<div class="palette-empty">No matching shortcuts</div>}
            >
              <For each={sections()}>
                {(section) => (
                  <section aria-label={section.area}>
                    <div class="palette-group">{section.area}</div>
                    <For each={section.rows}>
                      {(row) => (
                        <div class="shortcut-row">
                          <span class="palette-title">{row.title}</span>
                          <Show when={row.extension}>
                            <span class="palette-context">{row.extension}</span>
                          </Show>
                          <For each={row.keys}>
                            {(keys) => <span class="palette-keys">{keys}</span>}
                          </For>
                        </div>
                      )}
                    </For>
                  </section>
                )}
              </For>
            </Show>
          </div>
          <div class="palette-foot" aria-hidden="true">
            <span>
              <b>esc</b> close
            </span>
          </div>
        </div>
      </div>
    </Show>
  );
}
