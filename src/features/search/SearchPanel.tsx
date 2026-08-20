/** Search panel: debounced incremental querying, match navigation, results
 * grouped by page, indexing progress, image-only messaging. */
import { createMemo, createResource, createSignal, For, onMount, Show, useContext } from 'solid-js';
import type { FlatMatch } from './searchStore';
import { engine } from '../../lib/transport/engine';
import type { FormalEntry } from '../../types/engine';
import { contextForMatch } from '../../lib/structure/context';
import { pdfRectToCssRect } from '../../lib/coordinates/coords';
import IconButton from '../../components/IconButton';
import { IconChevronDown, IconChevronUp, IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import ScriptText from '../../components/ScriptText';

export default function SearchPanel() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp, zoom, searchStore } = tab;
  const requestScrollToPage = vp.requestScrollToPage;
  const s = searchStore.state;
  let inputEl!: HTMLInputElement;

  // The structure index, used only to label results. Failure is silent: a
  // document with no LaTeX anchors simply gets unlabelled results.
  const [structure] = createResource(
    () => (documentStore.state.loaded ? documentStore.state.docId : null),
    (docId) => engine.getFormalEnvs(docId).catch(() => [] as FormalEntry[])
  );
  const contextOf = (m: FlatMatch) => contextForMatch(structure() ?? [], m.src, m.start);
  const [caseSensitive, setCase] = createSignal(false);

  onMount(() => inputEl.focus());

  const navigateTo = async (m: FlatMatch, index: number) => {
    searchStore.setCurrent(index);
    const pages = documentStore.state.pages;
    const pageIndex = pages.findIndex((p) => p.srcIndex === m.src);
    if (pageIndex < 0) return; // page was deleted in this session
    try {
      const rects = await searchStore.rectsForMatch(m);
      const first = rects[0];
      if (first) {
        const geom = zoom.pagesGeom()[pageIndex];
        if (geom) {
          const css = pdfRectToCssRect(
            { x: first[0], y: first[1], w: first[2], h: first[3] },
            geom,
            vp.state.zoom
          );
          // Never scroll above the page's own top: for a match in the first
          // few lines the framing offset goes negative, which would land the
          // view on the end of the previous page instead of on the hit.
          requestScrollToPage(pageIndex, Math.max(0, css.y - vp.state.containerH * 0.35));
          return;
        }
      }
    } catch {
      /* rects unavailable — still jump to the page */
    }
    requestScrollToPage(pageIndex);
  };

  const step = (dir: 1 | -1) => {
    if (s.flat.length === 0) return;
    const next = (((s.current + dir) % s.flat.length) + s.flat.length) % s.flat.length;
    void navigateTo(s.flat[next]!, next);
  };

  const groups = createMemo(() => {
    const map = new Map<number, { match: FlatMatch; index: number }[]>();
    s.flat.forEach((match, index) => {
      const list = map.get(match.src) ?? [];
      list.push({ match, index });
      map.set(match.src, list);
    });
    return [...map.entries()].sort((a, b) => a[0] - b[0]);
  });

  const indexingPct = () => (s.total > 0 ? Math.round((s.indexed / s.total) * 100) : 0);
  const noTextDoc = () =>
    s.indexingDone && s.query.trim().length > 0 && s.flat.length === 0 && s.indexed >= s.total;

  return (
    <div class="search-panel" role="search" aria-label="Document search">
      <div class="search-head">
        <input
          ref={inputEl}
          class="search-input"
          type="text"
          placeholder="Search document…"
          value={s.query}
          onInput={(e) => searchStore.setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') step(e.shiftKey ? -1 : 1);
            if (e.key === 'Escape') vp.setState('searchOpen', false);
          }}
          aria-label="Search query"
        />
        <IconButton
          label="Previous match (⇧Enter)"
          disabled={s.flat.length === 0}
          onClick={() => step(-1)}
        >
          <IconChevronUp />
        </IconButton>
        <IconButton
          label="Next match (Enter)"
          disabled={s.flat.length === 0}
          onClick={() => step(1)}
        >
          <IconChevronDown />
        </IconButton>
        <IconButton label="Close search (Esc)" onClick={() => vp.setState('searchOpen', false)}>
          <IconClose />
        </IconButton>
      </div>

      <div class="search-meta">
        <label class="check">
          <input
            type="checkbox"
            checked={caseSensitive()}
            onInput={(e) => {
              setCase(e.currentTarget.checked);
              searchStore.setCaseSensitive(e.currentTarget.checked);
            }}
          />
          Case sensitive
        </label>
        <span class="search-count" aria-live="polite">
          <Show when={s.query.trim()} fallback={<span />}>
            <Show when={s.flat.length > 0} fallback={s.running ? 'Searching…' : 'No matches'}>
              {s.current + 1} / {s.flat.length}
            </Show>
          </Show>
        </span>
      </div>

      <Show when={!s.indexingDone && s.total > 0}>
        <div class="search-progress" role="status">
          Indexing pages… {s.indexed}/{s.total} ({indexingPct()}%)
          <div class="progress-track" aria-hidden="true">
            <div class="progress-fill" style={{ width: `${indexingPct()}%` }} />
          </div>
        </div>
      </Show>
      <Show when={s.truncated}>
        <div class="search-warning" role="status">
          The text-index memory budget was reached — results cover the first {s.indexed} pages. Text
          selection and highlighting still work on every page.
        </div>
      </Show>
      <Show when={s.resultsTruncated}>
        <div class="search-warning" role="status">
          Showing the first {s.resultLimit} matches. Refine the query to see a narrower result set.
        </div>
      </Show>
      <Show when={noTextDoc()}>
        <div class="search-warning" role="status">
          No matches. If this is a scanned/image-only PDF it has no extractable text (OCR is not
          supported yet).
        </div>
      </Show>

      <div class="search-results">
        <For each={groups()}>
          {([src, items]) => (
            <div class="search-group">
              <div class="search-group-title">Page {src + 1}</div>
              <For each={items}>
                {({ match, index }) => {
                  const where = createMemo(() => contextOf(match));
                  return (
                    <button
                      type="button"
                      class="search-result"
                      classList={{ 'is-current': index === s.current }}
                      onClick={() => void navigateTo(match, index)}
                    >
                      <span class="search-result-text">{match.snippet.trim() || '(match)'}</span>
                      <Show when={where().section || where().environment}>
                        <span class="search-result-where">
                          <Show when={where().section}>
                            <span class="search-result-section">
                              <ScriptText value={where().section!} />
                            </span>
                          </Show>
                          <Show when={where().section && where().environment}> › </Show>
                          <Show when={where().environment}>
                            <span class="search-result-env">
                              <ScriptText value={where().environment!} />
                            </span>
                          </Show>
                        </span>
                      </Show>
                    </button>
                  );
                }}
              </For>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
