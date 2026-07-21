/** Search orchestration: debounced querying over the incrementally built
 * Rust index, match bookkeeping, lazy rect resolution for visible pages. */
import { createStore, produce } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import type { PageMatches } from '../../types/engine';

export interface FlatMatch {
  src: number;
  start: number;
  len: number;
  snippet: string;
}

interface SearchState {
  docId: number;
  query: string;
  caseSensitive: boolean;
  results: PageMatches[];
  flat: FlatMatch[];
  current: number;
  running: boolean;
  indexed: number;
  total: number;
  truncated: boolean;
  indexingDone: boolean;
}

const [state, setState] = createStore<SearchState>({
  docId: -1,
  query: '',
  caseSensitive: false,
  results: [],
  flat: [],
  current: -1,
  running: false,
  indexed: 0,
  total: 0,
  truncated: false,
  indexingDone: false,
});

/** rects per "src:start:len", resolved lazily for mounted pages */
const rectsCache = new Map<string, [number, number, number, number][]>();
const rectsPending = new Set<string>();
const [rectsVersion, setRectsVersion] = createStore({ n: 0 });

let debounceTimer: ReturnType<typeof setTimeout> | undefined;
let querySeq = 0;

async function runQuery() {
  const seq = ++querySeq;
  const { docId, query, caseSensitive } = state;
  if (docId < 0 || query.trim().length === 0) {
    setState({ results: [], flat: [], current: -1, running: false });
    return;
  }
  setState('running', true);
  try {
    const results = await engine.searchQuery(docId, query, caseSensitive);
    if (seq !== querySeq) return; // superseded
    const flat: FlatMatch[] = [];
    for (const page of results) {
      for (const m of page.matches) {
        flat.push({ src: page.src, start: m.start, len: m.len, snippet: m.snippet });
      }
    }
    setState(
      produce((s) => {
        s.results = results;
        s.flat = flat;
        s.current = flat.length > 0 ? Math.min(Math.max(s.current, 0), flat.length - 1) : -1;
        s.running = false;
      })
    );
  } catch {
    if (seq === querySeq) setState('running', false);
  }
}

export const searchStore = {
  state,

  resetForDocument(docId: number, total: number) {
    rectsCache.clear();
    rectsPending.clear();
    setState({
      docId,
      query: '',
      results: [],
      flat: [],
      current: -1,
      running: false,
      indexed: 0,
      total,
      truncated: false,
      indexingDone: false,
    });
  },

  setQuery(query: string) {
    setState('query', query);
    rectsCache.clear();
    rectsPending.clear();
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runQuery, 250);
  },

  setCaseSensitive(v: boolean) {
    setState('caseSensitive', v);
    rectsCache.clear();
    rectsPending.clear();
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runQuery, 50);
  },

  onIndexProgress(indexed: number, total: number, truncated: boolean) {
    setState(
      produce((s) => {
        s.indexed = indexed;
        if (total > 0) s.total = total;
        s.truncated = truncated;
        s.indexingDone = total > 0 && indexed >= total;
      })
    );
    // new pages became searchable — refresh an active query (lightly debounced)
    if (state.query.trim().length > 0) {
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(runQuery, 300);
    }
  },

  setCurrent(i: number) {
    if (state.flat.length === 0) return;
    const n = state.flat.length;
    setState('current', ((i % n) + n) % n);
  },

  next() {
    this.setCurrent(state.current + 1);
  },

  prev() {
    this.setCurrent(state.current - 1);
  },

  currentMatch(): FlatMatch | null {
    return state.current >= 0 ? (state.flat[state.current] ?? null) : null;
  },

  /** Matches on a given source page with rects when resolved (mounted pages
   * call this; rect fetches are queued only for what is actually visible). */
  rectsForPage(src: number): { match: FlatMatch; index: number; rects: [number, number, number, number][] }[] {
    void rectsVersion.n; // reactive dependency for lazily arriving rects
    const out: { match: FlatMatch; index: number; rects: [number, number, number, number][] }[] = [];
    state.flat.forEach((match, index) => {
      if (match.src !== src) return;
      const key = `${match.src}:${match.start}:${match.len}`;
      const rects = rectsCache.get(key);
      if (rects) {
        out.push({ match, index, rects });
      } else if (!rectsPending.has(key) && state.docId >= 0) {
        rectsPending.add(key);
        void engine
          .getMatchRects(state.docId, match.src, match.start, match.len)
          .then((r) => {
            rectsCache.set(key, r);
            rectsPending.delete(key);
            setRectsVersion('n', (n) => n + 1);
          })
          .catch(() => rectsPending.delete(key));
      }
    });
    return out;
  },

  async rectsForMatch(m: FlatMatch): Promise<[number, number, number, number][]> {
    const key = `${m.src}:${m.start}:${m.len}`;
    const hit = rectsCache.get(key);
    if (hit) return hit;
    const r = await engine.getMatchRects(state.docId, m.src, m.start, m.len);
    rectsCache.set(key, r);
    setRectsVersion('n', (n) => n + 1);
    return r;
  },

  clear() {
    this.setQuery('');
  },
};
