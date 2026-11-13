/** The quick-open switcher (Cmd+Shift+O): any library item by title, author,
 * year, venue or cite key, opened on Enter.
 *
 * Drawn with the command palette's chrome so the two read as one family. The
 * library is read once per opening and matched client-side, so typing never
 * waits on the engine; the last row hands the query to the Library panel's
 * full-text search for when the words are in the paper rather than about it.
 */
import { batch, createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import MathText from '../../components/MathText';
import { engine } from '../../lib/transport/engine';
import { showError } from '../../stores/modalStore';
import { recentStore } from '../../stores/recentStore';
import type { Paper, PaperRow } from '../../types/engine';
import { goHome, openInNewTabOrFocus } from '../document/tabsController';
import { searchFullText } from '../projects/fullTextHandoff';
import { closeQuickOpen, quickOpenOpen } from './quickOpenStore';
import { defaultRows, detailLine, prepare, rankRows, type Prepared } from './quickOpenItems';

/** The first file still on disk, else the first file: opening a missing one
 * fails with a real error, which beats silently doing nothing. */
export function primaryPath(paper: Paper): string | null {
  const file = paper.files.find((f) => !f.missing) ?? paper.files[0];
  return file?.path ?? null;
}

type Row = { kind: 'item'; row: PaperRow } | { kind: 'fulltext'; query: string };

export default function QuickOpen() {
  const [query, setQuery] = createSignal('');
  const [cursor, setCursor] = createSignal(0);
  const [rows, setRows] = createSignal<PaperRow[]>([]);
  const [prepared, setPrepared] = createSignal<Prepared[]>([]);
  const [recentIds, setRecentIds] = createSignal<string[]>([]);
  const [loading, setLoading] = createSignal(false);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  let returnFocus: HTMLElement | null = null;
  /** Bumped per opening, so a slow answer from a previous one is dropped. */
  let generation = 0;

  const load = async () => {
    const mine = ++generation;
    setLoading(true);
    try {
      const listed = await engine.libraryPapers({});
      if (mine !== generation) return;
      const items = listed.rows.filter((row) => row.isPaper);
      batch(() => {
        setRows(items);
        setPrepared(prepare(items));
      });
    } catch (error) {
      if (mine === generation) showError(`Could not read the library: ${String(error)}`);
    } finally {
      if (mine === generation) setLoading(false);
    }
    // Recents are kept by path; the library knows which item each one is.
    const paths = recentStore.state.entries.map((entry) => entry.path);
    const found = await Promise.all(
      paths.map((path) => engine.libraryPaperForPath(path).catch(() => null))
    );
    if (mine !== generation) return;
    setRecentIds(found.filter((paper): paper is Paper => paper !== null).map((p) => p.id));
  };

  createEffect(() => {
    if (quickOpenOpen()) {
      const focused = document.activeElement;
      returnFocus = focused instanceof HTMLElement ? focused : null;
      batch(() => {
        setQuery('');
        setCursor(0);
        setRows([]);
        setPrepared([]);
        setRecentIds([]);
      });
      void load();
      queueMicrotask(() => input?.focus());
    } else {
      generation += 1;
    }
  });

  const shown = createMemo<Row[]>(() => {
    const text = query().trim();
    if (text.length === 0) {
      return defaultRows(rows(), recentIds()).map((row) => ({ kind: 'item', row }));
    }
    const items: Row[] = rankRows(prepared(), text).map((row) => ({ kind: 'item', row }));
    items.push({ kind: 'fulltext', query: text });
    return items;
  });

  createEffect(() => {
    const total = shown().length;
    if (total > 0 && cursor() >= total) setCursor(total - 1);
  });

  /** See CommandPalette: hover counts only once the pointer really moved. */
  const [pointerDriving, setPointerDriving] = createSignal(false);
  const onPointerMove = () => setPointerDriving(true);
  window.addEventListener('pointermove', onPointerMove, { passive: true });
  onCleanup(() => window.removeEventListener('pointermove', onPointerMove));

  createEffect(() => {
    const index = cursor();
    if (!quickOpenOpen() || !list) return;
    list.querySelector<HTMLElement>(`[data-row="${index}"]`)?.scrollIntoView({ block: 'nearest' });
  });

  const dismiss = () => {
    closeQuickOpen();
    const target = returnFocus;
    returnFocus = null;
    if (target?.isConnected) queueMicrotask(() => target.focus());
  };

  const openRow = async (row: PaperRow) => {
    closeQuickOpen();
    returnFocus = null;
    let path: string | null;
    try {
      path = primaryPath(await engine.libraryPaper(row.id));
    } catch (error) {
      showError(`Could not read ${row.citeKey || row.title}: ${String(error)}`);
      return;
    }
    if (!path) {
      showError('This item has no file to open.');
      return;
    }
    await openInNewTabOrFocus(path);
  };

  const runAt = (index: number) => {
    const entry = shown()[index];
    if (!entry) return;
    if (entry.kind === 'item') {
      void openRow(entry.row);
      return;
    }
    closeQuickOpen();
    returnFocus = null;
    searchFullText(entry.query);
    goHome();
  };

  const move = (delta: number) => {
    const total = shown().length;
    if (total === 0) return;
    setPointerDriving(false);
    setCursor((c) => (c + delta + total) % total);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      dismiss();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      runAt(cursor());
    } else if (e.key === 'ArrowDown' || (e.ctrlKey && !e.metaKey && e.key === 'n')) {
      e.preventDefault();
      move(1);
    } else if (e.key === 'ArrowUp' || (e.ctrlKey && !e.metaKey && e.key === 'p')) {
      e.preventDefault();
      move(-1);
    }
  };

  return (
    <Show when={quickOpenOpen()}>
      <div
        class="modal-backdrop palette-backdrop"
        onPointerDown={(e) => {
          if (e.target === e.currentTarget) dismiss();
        }}
      >
        <div
          class="palette quick-open"
          role="dialog"
          aria-modal="true"
          aria-label="Open item"
          onKeyDown={onKeyDown}
        >
          <input
            ref={input}
            class="palette-input"
            type="text"
            role="combobox"
            aria-expanded={shown().length > 0}
            aria-controls="quick-open-list"
            aria-activedescendant={shown().length > 0 ? `quick-open-row-${cursor()}` : undefined}
            placeholder="Open item by title, author, year or cite key..."
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
          <div class="palette-list" id="quick-open-list" role="listbox" ref={list} tabindex="-1">
            <Show
              when={shown().length > 0}
              fallback={
                <div class="palette-empty">
                  {loading() ? 'Reading the library...' : 'No items in the library'}
                </div>
              }
            >
              <For each={shown()}>
                {(entry, index) => (
                  <button
                    type="button"
                    data-row={index()}
                    id={`quick-open-row-${index()}`}
                    role="option"
                    aria-selected={cursor() === index()}
                    class="palette-row quick-open-row"
                    classList={{
                      'is-active': cursor() === index(),
                      'is-missing': entry.kind === 'item' && entry.row.missing,
                      'is-fulltext': entry.kind === 'fulltext',
                    }}
                    onPointerEnter={() => pointerDriving() && setCursor(index())}
                    onClick={() => runAt(index())}
                  >
                    <Show
                      when={entry.kind === 'item' ? entry.row : null}
                      fallback={
                        <span class="palette-title">
                          Search full text for "{entry.kind === 'fulltext' ? entry.query : ''}"
                        </span>
                      }
                    >
                      {(row) => (
                        <>
                          <span class="quick-open-text">
                            <MathText class="palette-title" text={row().title || row().citeKey} />
                            <span class="quick-open-detail">
                              {detailLine(row())}
                              <Show when={row().missing}>
                                <span class="quick-open-missing">file missing</span>
                              </Show>
                            </span>
                          </span>
                          <span class="quick-open-key">{row().citeKey}</span>
                        </>
                      )}
                    </Show>
                  </button>
                )}
              </For>
            </Show>
          </div>
          <div class="palette-foot" aria-hidden="true">
            <span>
              <b>Up/Down</b> navigate
            </span>
            <span>
              <b>Enter</b> open
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
