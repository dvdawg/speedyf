// References — every work this paper cites, and which ones you already have.
//
// Written as a worked example of the whole v2 surface: a declarative panel,
// reading the document and its citations, running SpeedyF's own navigation,
// per-extension settings, and cached results in storage.
//
// Three things here are worth copying into your own extension:
//
//   * It renders progress while it scans. A long paper takes a second or two,
//     and a panel that sits blank until it finishes looks broken.
//   * It answers every invoke promptly and does the slow work in between
//     renders, so it never trips the five-second timeout.
//   * It caches per document, so reopening the panel is instant and a rescan
//     is something you ask for.

/** Cache namespace.
 *
 * The number is part of the key, and it is bumped whenever a SpeedyF release
 * changes what a scan produces. Without it a cached scan outlives the thing it
 * was cached from: fixing how titles are extracted changed nothing on screen,
 * because every reference had been resolved once already and the panel kept
 * answering from a scan made by the old code. A stale entry under an old
 * number is never read again and costs a few kilobytes until its extension is
 * removed. */
const CACHE_VERSION = 2;
const CACHE_PREFIX = `scan:v${CACHE_VERSION}:`;

let nextRequestId = 1;
const pending = new Map();
/** Cached scan for the document currently shown, or null. */
let current = null;
let scanning = false;

self.onmessage = (event) => {
  const message = event.data;

  if (message.type === 'invoke') {
    handle(message.command, message.value)
      .then(() => done(message.requestId, true))
      .catch((err) => done(message.requestId, false, String(err.message ?? err)));
    return;
  }

  if (message.type === 'reply') {
    const waiting = pending.get(message.requestId);
    if (!waiting) return;
    pending.delete(message.requestId);
    if (message.ok) waiting.resolve(message.value);
    else waiting.reject(new Error(message.error));
  }
};

function call(api, args) {
  const requestId = nextRequestId++;
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    self.postMessage({ type: 'call', requestId, api, args });
  });
}

// ---------- commands ----------

async function handle(command, value) {
  if (command === 'jump') {
    // A row action. `value` is the page the row carried.
    const page = Number(value);
    if (!Number.isInteger(page)) throw new Error('no page to jump to');
    await call('navigate.toPage', { page });
    return;
  }

  if (command === 'rescan') return show({ force: true });
  if (command === 'refs') return show({ force: false });
  throw new Error(`unknown command "${command}"`);
}

async function show({ force }) {
  let info;
  try {
    info = await call('document.info');
  } catch {
    current = null;
    return draw({ empty: 'Open a paper to see what it cites.' });
  }

  if (!force) {
    const cached = await readCache(info);
    if (cached) {
      current = cached;
      return render();
    }
  }
  await scan(info);
}

// ---------- scanning ----------

async function scan(info) {
  if (scanning) return;
  scanning = true;
  try {
    const found = new Map();
    // Links with no DOI or arXiv id — a JMLR paper, a book, a Semantic Scholar
    // entry — are references too. Collected per page and kept below only for
    // pages that also carry identified citations, which is how the
    // bibliography is told apart from a licence link in the footer.
    const plainByPage = new Map();
    for (let page = 0; page < info.pageCount; page++) {
      for (const link of await call('document.links', { page })) {
        if (link.citation || !link.host) continue;
        if (!plainByPage.has(page)) plainByPage.set(page, []);
        plainByPage.get(page).push(link);
      }
      for (const citation of await call('citations.list', { page })) {
        const key = `${citation.kind}:${citation.value}`;
        const existing = found.get(key);
        if (existing) existing.pages.push(page);
        else found.set(key, { ...citation, key, pages: [page] });
      }
      // Progress, so a long document does not look frozen. Cheap: the host
      // validates and redraws, it does not re-enter this worker.
      //
      // A rescan keeps whatever is already on screen and puts the progress in
      // a heading above it. Replacing the list with a status line means the
      // panel you were reading empties itself the moment you refresh it.
      if (page % 5 === 0 || page === info.pageCount - 1) {
        await drawProgress(`Reading page ${page + 1} of ${info.pageCount} — ${found.size} found`);
      }
    }

    // A page carrying identified citations is a reference page; plain links on
    // it are references too. Elsewhere they are licence notices, author
    // homepages, and the like.
    const citedPages = new Set([...found.values()].flatMap((c) => c.pages));
    const plain = new Map();
    for (const [page, links] of plainByPage) {
      if (!citedPages.has(page)) continue;
      for (const link of links) {
        if (plain.has(link.uri)) plain.get(link.uri).pages.push(page);
        else plain.set(link.uri, { ...link, key: link.uri, kind: 'link', pages: [page] });
      }
    }

    // An identifier that is a strict prefix of another of the same kind is a
    // truncation of it, not a second work: "10.1162/neco" beside
    // "10.1162/neco_a_00142" is one paper read twice, once from a line that
    // wrapped mid-suffix. Collapsed here because this is the only place with a
    // view of the whole document; a single page cannot tell the two apart.
    const collapsed = [...found.values()].filter(
      (candidate) =>
        ![...found.values()].some(
          (other) =>
            other !== candidate &&
            other.kind === candidate.kind &&
            other.value.length > candidate.value.length &&
            other.value.toLowerCase().startsWith(candidate.value.toLowerCase())
        )
    );

    const citations = collapsed;
    // Resolution is one call each and only for distinct works, so it happens
    // after the page walk rather than inside it.
    for (const citation of citations) {
      const resolved = await call('citations.resolve', {
        kind: citation.kind,
        value: citation.value,
      });
      citation.title = resolved.title ?? resolved.fileName ?? null;
      citation.inLibrary = resolved.inLibrary;
      // A paper prints its own identifier in its header, so it resolves to
      // itself and turns up at the top of its own reference list. Compared on a
      // normalised name because a library often holds the same paper twice —
      // "2605.25567v3.pdf" beside "2605.25567v3 (1).pdf" — and resolution may
      // answer with either copy.
      citation.isSelf = !!resolved.fileName && sameDocument(resolved.fileName, info.name);
    }
    const references = citations.filter((citation) => !citation.isSelf);

    for (const link of plain.values()) {
      link.title = readableUrl(link.uri);
      link.inLibrary = false;
    }
    current = {
      name: info.name,
      pageCount: info.pageCount,
      citations: [...references, ...plain.values()],
    };
    await writeCache(current);
    await render();
  } finally {
    scanning = false;
  }
}

// ---------- caching ----------

/** Keyed by name and length: enough to tell two open papers apart, and to
 * notice when the document changed underneath a stale scan. */
function cacheKey(info) {
  return `${CACHE_PREFIX}${info.pageCount}:${info.name}`.slice(0, 200);
}

async function readCache(info) {
  try {
    const raw = await call('storage.get', { key: cacheKey(info) });
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    return parsed.name === info.name && parsed.pageCount === info.pageCount ? parsed : null;
  } catch {
    return null;
  }
}

async function writeCache(scan) {
  try {
    await call('storage.set', { key: cacheKey(scan), value: JSON.stringify(scan) });
  } catch {
    // A scan too large for the store is not worth failing over; it just will
    // not be remembered.
  }
}

// ---------- drawing ----------

async function render() {
  if (!current) return draw({ empty: 'Open a paper to see what it cites.' });
  const { citations, pageCount } = current;

  if (citations.length === 0) {
    return draw({ empty: 'No references found.' });
  }

  const [sort, group] = await Promise.all([
    call('settings.get', { key: 'sort' }),
    call('settings.get', { key: 'group' }),
  ]);

  const ordered = [...citations].sort(comparator(sort));
  const rows = ordered.map(toRow);

  if (group === 'false') {
    return draw({
      sections: [{ title: `${citations.length} cited in ${pageCount} pages`, rows }],
    });
  }

  const have = ordered.filter((c) => c.inLibrary).map(toRow);
  const missing = ordered.filter((c) => !c.inLibrary).map(toRow);
  const sections = [];
  if (have.length) sections.push({ title: `In your library (${have.length})`, rows: have });
  if (missing.length)
    sections.push({ title: `Not in your library (${missing.length})`, rows: missing });
  return draw({ sections });
}

function comparator(sort) {
  if (sort === 'most cited')
    return (a, b) => b.pages.length - a.pages.length || a.pages[0] - b.pages[0];
  if (sort === 'title') {
    return (a, b) => (a.title ?? a.value ?? '').localeCompare(b.title ?? b.value ?? '');
  }
  return (a, b) => a.pages[0] - b.pages[0];
}

function toRow(citation) {
  return {
    id: citation.key,
    title: citation.title ?? citation.value,
    subtitle: describe(citation),
    ...(citation.inLibrary ? { badge: 'have it' } : {}),
    // Every row jumps to where the work is first cited.
    action: { command: 'jump', value: String(citation.pages[0]) },
  };
}

/** What a row is identified by: the scheme for a recognised work, the host for
 * everything else. */
function sourceOf(citation) {
  return citation.kind === 'link' ? (citation.host ?? 'link') : citation.kind;
}

/** "cited 3× · pages 4, 7, 11" — with the tail trimmed, since a work cited on
 * twenty pages does not need all twenty listed in a sidebar. */
/** Whether two file names are the same document. Ignores the extension and the
 * " (1)" a second download picks up. */
function sameDocument(a, b) {
  const normalise = (name) =>
    name
      .replace(/\.pdf$/i, '')
      .replace(/\s*\(\d+\)$/, '')
      .trim()
      .toLowerCase();
  return normalise(a) === normalise(b);
}

/** A URL as something to read in a list: host first, then enough of the path
 * to tell two entries from the same site apart. */
function readableUrl(uri) {
  const withoutScheme = uri.replace(/^https?:\/\//, '').replace(/\/$/, '');
  return withoutScheme.length > 64 ? `${withoutScheme.slice(0, 64)}…` : withoutScheme;
}

function describe(citation) {
  const shown = [...new Set(citation.pages)].slice(0, 4).map((p) => p + 1);
  const more = citation.pages.length - shown.length;
  const where = `${citation.pages.length === 1 ? 'page' : 'pages'} ${shown.join(', ')}${more > 0 ? ` +${more}` : ''}`;
  const times = citation.pages.length === 1 ? '' : `cited ${citation.pages.length}× · `;
  return `${sourceOf(citation)} · ${times}${where}`;
}

function draw(ui) {
  return call('ui.render', { panel: 'refs', ui });
}

/** Progress that does not throw away what is already listed. */
function drawProgress(status) {
  if (!current || current.citations.length === 0) return draw({ empty: `${status}…` });
  return draw({
    sections: [{ title: status, rows: current.citations.map(toRow) }],
  });
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}
