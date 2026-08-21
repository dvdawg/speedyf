/** The library: the folders in it, how far indexing has got, and a search
 * across everything in it.
 *
 * Searching here rather than in the ⌘F panel is deliberate. ⌘F answers "where
 * is this in what I am reading"; this answers "which of my papers said this",
 * which is a question you ask *before* opening anything — so it belongs on the
 * screen you see when nothing is open.
 *
 * Three fixed bands: the search field, the results, the folders. Only the
 * middle one scrolls. Nesting a scrolling result list inside a scrolling panel
 * is what made this unusable before — a wheel gesture had two things it could
 * plausibly move, so it moved the wrong one.
 *
 * Results carry a file and a page, never a docId: nothing is open yet, and
 * choosing what to open is the entire point. */
import { createMemo, For, Show } from 'solid-js';
import { libraryStore } from '../../features/citations/libraryStore';
import { openInNewTabOrFocus } from '../document/tabsController';
import { tabsStore } from '../../stores/tabsStore';
import { IconPlus, IconSearch } from '../../components/icons';
import { splitSnippet } from './snippet';
import type { LibraryHit } from '../../types/engine';

/** Pages shown per document before the rest are summarized. Enough to tell
 * whether it is the right paper; past that you open it. */
const PAGES_SHOWN = 3;

export default function LibraryPanel() {
  const library = () => libraryStore.state.library;
  const hasFolders = () => library().roots.length > 0;
  const hasQuery = () => libraryStore.state.query.trim().length > 0;

  const scanPercent = createMemo(() => {
    const { indexed, total } = library();
    return total > 0 ? Math.min(100, Math.round((indexed / total) * 100)) : 0;
  });

  /** One honest line about the state of the corpus.
   *
   * Text indexing trails the metadata scan and only it decides what is
   * searchable, so it is reported separately rather than folded into one
   * number that would claim the library was ready before it was. */
  const status = createMemo(() => {
    const { indexed, scanning, textIndexed, textTotal } = library();
    if (!hasFolders()) return '';
    if (scanning) return `Scanning… ${indexed} papers found so far.`;
    const pending = Math.max(0, textTotal - textIndexed);
    if (pending > 0) {
      return `Indexing ${textIndexed} of ${textTotal} — searching what is ready so far.`;
    }
    return `${indexed} ${indexed === 1 ? 'paper' : 'papers'} in the library.`;
  });

  /** Open the paper and land on the page that matched, with the term already
   * searched so it is highlighted on arrival. The library result knows a
   * *source* page; the viewer works in layout positions, which differ once
   * pages have been reordered this session. */
  const openHit = async (hit: LibraryHit, src: number) => {
    const query = libraryStore.state.query;
    if (!(await openInNewTabOrFocus(hit.path))) return;
    const tab = tabsStore.state.tabs.find((t) => t.documentStore.state.path === hit.path);
    if (!tab) return;
    tab.searchStore.setQuery(query);
    const index = tab.documentStore.state.pages.findIndex((page) => page.srcIndex === src);
    tab.viewport.requestScrollToPage(index >= 0 ? index : 0);
  };

  return (
    <>
      <div class="home-panel-head">
        <h2>Library</h2>
      </div>
      <div class="home-panel-body library-panel">
        <div class="library-search">
          <div class="library-search-field">
            <IconSearch />
            <input
              type="text"
              placeholder={hasFolders() ? 'Search every paper…' : 'Add a folder to search it'}
              disabled={!hasFolders()}
              value={libraryStore.state.query}
              onInput={(e) => libraryStore.setQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape' && hasQuery()) {
                  e.preventDefault();
                  e.stopPropagation();
                  libraryStore.setQuery('');
                }
              }}
            />
            <Show when={hasQuery()}>
              <button
                type="button"
                class="library-search-clear"
                aria-label="Clear the search"
                onClick={() => libraryStore.setQuery('')}
              >
                ×
              </button>
            </Show>
          </div>
          <Show when={status()}>
            <p class="library-status">{status()}</p>
          </Show>
        </div>

        <div class="library-scroll">
          <Show
            when={hasQuery()}
            fallback={
              // With folders present the status line above already says what
              // is searchable, so repeating it here is noise.
              <Show when={!hasFolders()}>
                <p class="library-hint">
                  Add the folders your papers live in. They are read locally and never leave your
                  machine.
                </p>
              </Show>
            }
          >
            <Show
              when={libraryStore.state.results.length > 0}
              fallback={
                <p class="library-hint">
                  {libraryStore.state.searching ? 'Searching…' : 'No papers contain that.'}
                </p>
              }
            >
              <div class="library-results">
                <For each={libraryStore.state.results}>
                  {(hit) => (
                    <article class="library-result">
                      <button
                        type="button"
                        class="library-result-head"
                        title={hit.path}
                        onClick={() => void openHit(hit, hit.pages[0]?.src ?? 0)}
                      >
                        <span class="library-result-title">
                          <span class="library-result-name">{hit.title || hit.name}</span>
                          <Show when={hit.title && hit.title !== hit.name}>
                            <span class="library-result-file">{hit.name}</span>
                          </Show>
                        </span>
                        <span class="library-result-count">
                          {hit.totalMatches} {hit.totalMatches === 1 ? 'match' : 'matches'}
                        </span>
                      </button>
                      <For each={hit.pages.slice(0, PAGES_SHOWN)}>
                        {(page) => (
                          <button
                            type="button"
                            class="library-result-hit"
                            onClick={() => void openHit(hit, page.src)}
                          >
                            <span class="library-result-page">p{page.src + 1}</span>
                            <span class="library-result-snippet">
                              <For
                                each={splitSnippet(
                                  page.matches[0]?.snippet.trim() ?? '',
                                  libraryStore.state.query
                                )}
                              >
                                {(part) =>
                                  part.match ? <mark>{part.text}</mark> : <span>{part.text}</span>
                                }
                              </For>
                            </span>
                          </button>
                        )}
                      </For>
                      <Show when={hit.pages.length > PAGES_SHOWN}>
                        <p class="library-result-more">
                          and {hit.pages.length - PAGES_SHOWN} more{' '}
                          {hit.pages.length - PAGES_SHOWN === 1 ? 'page' : 'pages'}
                        </p>
                      </Show>
                    </article>
                  )}
                </For>
                <Show when={libraryStore.state.truncated}>
                  <p class="library-hint">
                    Showing the best matches only — narrow the search to see fewer, better ones.
                  </p>
                </Show>
              </div>
            </Show>
          </Show>
        </div>

        <footer class="library-folders">
          <div class="library-folders-head">
            <span>Folders</span>
            <button
              type="button"
              class="secondary-btn library-add-folder"
              onClick={() => void libraryStore.addLibraryFolder()}
            >
              <IconPlus /> Add folder…
            </button>
          </div>
          <Show when={hasFolders()}>
            <div class="library-folder-list">
              <For each={library().roots}>
                {(root) => (
                  <span class="library-folder" title={root}>
                    <span class="library-folder-path">{root}</span>
                    <button
                      type="button"
                      class="library-folder-remove"
                      aria-label={`Remove ${root}`}
                      title="Remove this folder"
                      onClick={() => void libraryStore.removeLibraryFolder(root)}
                    >
                      ×
                    </button>
                  </span>
                )}
              </For>
            </div>
          </Show>
          <Show when={library().scanning}>
            <progress class="library-progress" max="100" value={scanPercent()} />
          </Show>
          <Show when={libraryStore.state.libraryError}>
            <p class="library-error">{libraryStore.state.libraryError}</p>
          </Show>
        </footer>
      </div>
    </>
  );
}
