/** Citation library: a local folder of papers indexed so DOI and arXiv
 * citations in a document resolve to files on disk. The backend exposes only
 * aggregate scan status, not an entry list, so there is nothing to browse. */
import { Show } from 'solid-js';
import { libraryStore } from '../citations/libraryStore';

export default function LibraryPanel() {
  const status = () => libraryStore.state.library;

  return (
    <>
      <div class="home-panel-head">
        <h2>Library</h2>
        <Show when={status().root}>
          <button
            type="button"
            class="secondary-btn"
            onClick={() => void libraryStore.disableLibrary()}
          >
            Disable
          </button>
        </Show>
      </div>
      <div class="home-panel-body">
        <p class="home-panel-intro">
          Point SpeedyF at a folder of PDFs and it will resolve DOI and arXiv citations to those
          files, so cited papers open straight from a document.
        </p>

        <div class="home-library-root">
          <span class="home-setting-title">Folder</span>
          <Show
            when={status().root}
            fallback={<span class="home-library-empty">No folder selected</span>}
          >
            {(root) => (
              <span class="home-library-path" title={root()}>
                {root()}
              </span>
            )}
          </Show>
        </div>

        <Show when={status().root}>
          <div class="home-library-scan">
            <Show
              when={status().scanning}
              fallback={<span>{status().indexed} papers indexed</span>}
            >
              <span>
                Indexed {status().indexed} of {status().total}…
              </span>
              <progress
                class="home-library-progress"
                value={status().indexed}
                max={Math.max(status().total, 1)}
              />
            </Show>
          </div>
        </Show>

        <Show when={libraryStore.state.libraryError}>
          {(message) => <p class="home-library-error">{message()}</p>}
        </Show>

        <button
          type="button"
          class="primary-btn"
          onClick={() => void libraryStore.chooseLibraryFolder()}
        >
          {status().root ? 'Change folder…' : 'Choose folder…'}
        </button>
      </div>
    </>
  );
}
