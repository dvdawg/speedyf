// References — every work this paper cites, where it is genuinely cited, and
// which ones you already have.
//
// Written as a worked example of the whole v2 surface: a declarative panel,
// reading the document's text, links and internal jumps, running SpeedyF's own
// navigation, per-extension settings, and cached results in storage.
//
// Three things here are worth copying into your own extension:
//
//   * It renders progress while it scans. A long paper takes a second or two,
//     and a panel that sits blank until it finishes looks broken.
//   * It answers every invoke promptly and does the slow work in between
//     renders, so it never trips the five-second timeout.
//   * It caches per document, so reopening the panel is instant and a rescan
//     is something you ask for.
//
// ---------------------------------------------------------------------------
//
// How a reference is found at all.
//
// The obvious reading of a bibliography is the wrong one: collect the links on
// the pages that look like a reference list, and report each one. That gives a
// list of URLs, ordered by the page they are *printed* on — which is the same
// page for all of them, so "where is this cited" answers "page 14" for every
// row, and "what is this" answers with a hostname.
//
// A typeset paper knows better than that, and says so in the file. A `[12]` in
// body text is a real link, and it points at a place on the references page:
// the entry it refers to. That single fact carries both halves of the problem.
// Follow the jumps *forward* and a reference knows every page it is cited on.
// Sort the destinations *down the page* and they cut the reference list into
// entries — and the words inside one entry are its authors, its title and its
// year, which is the difference between a row that reads
//
//     api.semanticscholar.org/CorpusID:605683      link · page 14
//
// and one that reads
//
//     Effect of batch learning in multilayer neural networks
//     Kenji Fukumizu · 1998 · cited on pages 3, 5, 9
//
// Nothing here is guesswork about the prose. The entry boundaries come from
// the document's own destinations, not from spotting where a citation seems to
// start.
//
// What this cannot do, and what happens then: a PDF that was not typeset with
// internal links — an older scan, most things exported from a word processor —
// has no jumps at all, and none of the above is available. That is common
// enough that it is a supported case rather than a failure: `scanByLinks`
// below is the previous approach, kept whole, and the panel falls back to it.

/** Cache namespace.
 *
 * The number is part of the key, and it is bumped whenever a SpeedyF release
 * changes what a scan produces. Without it a cached scan outlives the thing it
 * was cached from: fixing how titles are extracted changed nothing on screen,
 * because every reference had been resolved once already and the panel kept
 * answering from a scan made by the old code. A stale entry under an old
 * number is never read again and costs a few kilobytes until its extension is
 * removed. */
const CACHE_VERSION = 5;
const CACHE_PREFIX = `scan:v${CACHE_VERSION}:`;

/** A page needs this many distinct destinations before it is considered part
 * of a reference list. Section headings, figures and numbered equations are
 * link destinations too, so a handful on a page means nothing; a bibliography
 * page carries one per entry and there are always many. */
const MIN_ANCHORS_PER_PAGE = 4;
/** Longer than any reference, and shorter than a paragraph of prose. Used to
 * tell a page of the bibliography from a page of the appendix. */
const MAX_ENTRY_LENGTH = 600;
/** Reading positioned text is the expensive call, so the number of pages worth
 * reading it for is bounded. No bibliography is this long. */
const MAX_REFERENCE_PAGES = 12;

let nextRequestId = 1;
const pending = new Map();
/** Cached scan for the document currently shown, or null. */
let current = null;
let scanning = false;
/** Rows that are open, showing the places their work is cited. Not cached:
 * what you have open is a property of this sitting, not of the document. */
const expanded = new Set();
/** The line each citation site sits on, keyed by `page:y`. Read from the page
 * the first time a row is opened and kept, because a reference cited in five
 * places is five text reads and reopening the row should not repeat them. */
const context = new Map();

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
  if (command === 'jump') return open(value);
  if (command === 'goto') return goTo(value);
  if (command === 'rescan') return show({ force: true });
  if (command === 'refs') return show({ force: false });
  throw new Error(`unknown command "${command}"`);
}

/** Open a reference, listing every place it is cited.
 *
 * A row that jumps somewhere on activation has to choose which of the five
 * places to jump to, and any choice is wrong four times. Opening it shows all
 * five, each with the line it appears in, and the choice belongs to the
 * reader. Activating it again closes it.
 *
 * A reference with nowhere to go — a list read from bare links, which cannot
 * know where anything is cited — goes to where it is listed instead, since
 * there is nothing to open.
 */
async function open(value) {
  const key = String(value ?? '');
  const entry = current?.citations.find((citation) => citation.key === key);
  if (!entry) {
    // A row from a previous document, or a page carried by an older cache.
    const page = Number(value);
    if (!Number.isInteger(page)) throw new Error('no page to jump to');
    await call('navigate.toPage', { page });
    return;
  }
  if (!entry.sites?.length) {
    await call('navigate.toPage', { page: entry.pages[0] });
    return;
  }
  if (expanded.has(key)) expanded.delete(key);
  else {
    expanded.add(key);
    await readContext(entry);
  }
  await render();
}

/** Go to one citation of a work.
 *
 * The whole box rather than just the page, so the viewer can frame the line
 * and mark it: being put on page 6 for the third time with the `[12]` still to
 * find is most of the work the panel was meant to save.
 */
async function goTo(value) {
  const [where, ...rects] = String(value ?? '').split('|');
  const page = Number(where);
  if (!Number.isInteger(page)) throw new Error('no page to jump to');
  const boxes = rects
    .map((rect) => rect.split(',').map(Number))
    .filter((parts) => parts.length === 4 && parts.every(Number.isFinite))
    .map(([x, y, w, h]) => ({ x, y, w, h }));
  if (boxes.length === 0) {
    await call('navigate.toPage', { page });
    return;
  }
  await call('navigate.toPlace', {
    page,
    y: Math.max(...boxes.map((box) => box.y)),
    boxes,
  });
}

/** A citation site as the value its row carries back.
 *
 * "6|91.2,343.1,50.9,12.9|89,329.6,23.8,12.9" — the page, then one box per
 * piece of the citation. Compact because a row's value is capped at 500
 * characters and truncated silently if it runs over, which JSON would not
 * survive. */
function place(site) {
  const rects = site.rects
    .map((rect) => [rect.x, rect.y, rect.w, rect.h].map(round).join(','))
    .join('|');
  return `${site.page}|${rects}`;
}

function round(value) {
  return Math.round(value * 10) / 10;
}

/** The line each of a reference's citations sits on.
 *
 * One text read per page, and only for a row you opened. Read once and kept:
 * a row reopened after being closed should not cost the same work again.
 */
async function readContext(entry) {
  const wanted = entry.sites.filter((site) => !context.has(siteKey(site)));
  if (wanted.length === 0) return;
  const byPage = new Map();
  for (const site of wanted) {
    if (!byPage.has(site.page)) byPage.set(site.page, []);
    byPage.get(site.page).push(site);
  }
  for (const [page, sites] of byPage) {
    let runs = [];
    try {
      runs = await call('document.pageRuns', { page });
    } catch {
      continue;
    }
    for (const site of sites) {
      const found = lineAt(runs, site.x, site.y);
      if (found) context.set(siteKey(site), found);
    }
  }
}

/** The printed line at a place on a page.
 *
 * Read through the same column split the reference list uses: a citation in
 * the left column of a two-column page sits at the same height as text in the
 * right one, and a line built across both is two half-sentences.
 */
function lineAt(runs, x, y) {
  if (runs.length === 0) return null;
  const at = gutter(runs);
  const column =
    at === null ? runs : runs.filter((run) => (x < at ? run.box.x < at : run.box.x >= at));
  if (column.length === 0) return null;
  const gap = lineGap(column);
  const lines = toLines(column, gap);
  let best = null;
  for (const line of lines) {
    const distance = Math.abs(line.top - y);
    if (best === null || distance < best.distance) best = { distance, line };
  }
  if (!best || best.distance > gap) return null;
  return fragment(clip(joinRuns([best.line]), 160));
}

/** A line lifted off a page, marked as the fragment it is.
 *
 * What sits at a citation is a *line*, not a sentence: it starts wherever the
 * previous one ran out and stops wherever this one did. Shown bare it reads as
 * though someone wrote "form mode-wise solutions with depth-dependent
 * plateaus [5, 6, 33]; the saddle-to-saddle line, or". The ellipses say it was
 * cut, which is the difference between a quotation and a mistake.
 */
function fragment(line) {
  if (!line) return null;
  const before = /^[a-z(,;]/.test(line) ? '...' : '';
  const after = /[.!?"']$/.test(line) ? '' : '...';
  return `${before}${line}${after}`;
}

function siteKey(site) {
  return `${site.page}:${site.y}`;
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
      expanded.clear();
      return render();
    }
  }
  await scan(info);
}

// ---------- scanning ----------

async function scan(info) {
  if (scanning) return;
  scanning = true;
  expanded.clear();
  context.clear();
  try {
    const pages = [];
    let jumpCount = 0;
    for (let page = 0; page < info.pageCount; page++) {
      const [links, jumps] = await Promise.all([
        call('document.links', { page }),
        call('document.jumps', { page }),
      ]);
      pages.push({ page, links, jumps });
      jumpCount += jumps.length;
      // Progress, so a long document does not look frozen.
      //
      // A rescan keeps whatever is already on screen and puts the progress in
      // a heading above it. Replacing the list with a status line means the
      // panel you were reading empties itself the moment you refresh it.
      if (page % 5 === 0 || page === info.pageCount - 1) {
        await drawProgress(`Reading page ${page + 1} of ${info.pageCount}`);
      }
    }

    const byEntry = jumpCount > 0 ? await scanByEntries(info, pages) : null;
    const citations = byEntry ?? (await scanByLinks(info, pages));

    current = {
      name: info.name,
      pageCount: info.pageCount,
      // Recorded so the panel can say which way it read the document. The two
      // answer different questions — one knows where a work is cited, the
      // other only where it is listed — and a heading that does not
      // distinguish them makes the weaker one look like the stronger one.
      readBy: byEntry ? 'entries' : 'links',
      citations,
    };
    await writeCache(current);
    await render();
  } finally {
    scanning = false;
  }
}

// ---------- reading the bibliography as entries ----------

/** The whole reference list, read from the document's own destinations.
 *
 * Returns null when the document has internal links but none of them describe
 * a reference list — a paper whose only jumps are to its own figures — so the
 * caller can fall back rather than show an empty panel.
 */
async function scanByEntries(info, pages) {
  const anchorsByPage = new Map();
  for (const { jumps } of pages) {
    for (const jump of jumps) {
      if (jump.toY === null) continue;
      if (!anchorsByPage.has(jump.toPage)) anchorsByPage.set(jump.toPage, new Map());
      // Rounded, because the same entry is the destination of every `[12]` in
      // the paper and their positions agree only to within rounding. Kept with
      // its `x`, without which two entries printed side by side in a
      // two-column list are one destination.
      const anchor = anchorOf(jump);
      anchorsByPage.get(jump.toPage).set(anchor.key, anchor);
    }
  }

  const entries = await readReferenceList(info, pages, anchorsByPage);
  if (!entries) return null;

  // Where each entry is cited: every jump in the document that lands on it.
  // This is the whole point of reading the list this way.
  const byAnchor = new Map(entries.map((entry) => [`${entry.page}:${entry.anchor}`, entry]));

  // The height the reference list starts at, per page it occupies. A
  // bibliography often begins halfway down a page whose top half is still
  // body text, and skipping such a page whole — which is what "is this a
  // reference page" amounts to — threw away every citation printed above the
  // list. On one paper that was the last two citations of its two most-cited
  // works, and the panel looked like it had the wrong pages.
  const listTop = new Map();
  for (const entry of entries) {
    const seen = listTop.get(entry.page);
    if (seen === undefined || entry.top > seen) listTop.set(entry.page, entry.top);
  }

  const found = new Map();
  for (const { page, jumps } of pages) {
    const top = listTop.get(page);
    for (const jump of jumps) {
      if (jump.toY === null) continue;
      // A jump printed inside the reference list is a cross-reference between
      // entries ("see [4]"), not a citation of one. Judged by where it sits
      // rather than by which page it is on.
      if (top !== undefined && jump.box.y <= top + 1) continue;
      const entry = byAnchor.get(`${jump.toPage}:${anchorOf(jump).key}`);
      if (!entry) continue;
      const key = `${entry.key}@${page}`;
      if (!found.has(key)) found.set(key, { entry, page, boxes: [] });
      found.get(key).boxes.push(jump.box);
    }
  }

  for (const { entry, page, boxes } of found.values()) {
    for (const rects of citationsIn(boxes)) {
      // Where the citation is printed, not just which page it is on. "Cited on
      // page 4" is a page number; being put on the sentence that cites it is a
      // destination, and it is the whole reason the position was followed.
      entry.sites.push({
        page,
        x: Math.min(...rects.map((rect) => rect.x)),
        y: Math.round(Math.max(...rects.map((rect) => rect.y)) * 2) / 2,
        rects,
      });
      entry.citations += 1;
    }
  }
  for (const entry of entries) {
    entry.sites.sort((a, b) => a.page - b.page || b.y - a.y);
    entry.citedOn = [...new Set(entry.sites.map((site) => site.page))].sort((a, b) => a - b);
  }

  await resolveAll(entries);
  return entries;
}

/** The reference list: which pages it is on, and what is on them.
 *
 * Density alone cannot find it. An appendix of numbered theorems has
 * destinations on every page and looks identical from the outside — reading a
 * paper's bibliography as pages 12 to 17 rather than 12 to 15 added thirty
 * "references" that were paragraphs of a proof. Nor can the printed heading
 * finish the job: it says where the list starts and nothing about where it
 * ends.
 *
 * So the pages are cut up first and judged afterwards, on whether what came
 * out looks like a list of references — labelled, dated, and several to a
 * page. That is the only test that distinguishes a bibliography from anything
 * else with the same shape, and it costs one text read per page considered.
 */
async function readReferenceList(info, pages, anchorsByPage) {
  const dense = [...anchorsByPage.entries()]
    .filter(([, anchors]) => anchors.size >= MIN_ANCHORS_PER_PAGE)
    .map(([page]) => page)
    .sort((a, b) => a - b);
  if (dense.length === 0) return null;

  const from = (await findHeading(dense)) ?? dense[0];
  const entries = [];
  /** Pages taken so far. Only its length matters — whether the list has
   * started, which decides how leniently the next page is judged. */
  const referencePages = [];
  let read = 0;
  for (let page = from; page < info.pageCount && read < MAX_REFERENCE_PAGES; page++) {
    const anchors = [...(anchorsByPage.get(page)?.values() ?? [])];
    let found = [];
    if (anchors.length > 0) {
      read += 1;
      const runs = await call('document.pageRuns', { page });
      found = entriesOnPage(
        page,
        anchors,
        runs,
        pages[page]?.links ?? [],
        entries[entries.length - 1]
      );
    }
    if (!isReferenceList(found, referencePages.length > 0)) {
      // Before the list proper: the heading's own page, or a page of body text
      // between the heading and the first entry. After it: the appendix.
      if (referencePages.length > 0) break;
      continue;
    }
    referencePages.push(page);
    entries.push(...found);
  }
  return entries.length > 0 ? entries : null;
}

/** Whether a page of cut-up destinations is part of a reference list.
 *
 * A reference carries a label or a year — almost all carry both — and a page
 * of them carries several. A page of theorems cut the same way carries
 * paragraphs of prose, which is what this rejects.
 *
 * Continuing a list is judged more leniently than starting one, because the
 * last page of a bibliography holds two entries as often as twenty.
 */
function isReferenceList(entries, continuing) {
  if (entries.length < (continuing ? 1 : 3)) return false;
  const dated = entries.filter((entry) => entry.label !== null || entry.year !== null);
  if (dated.length * 2 < entries.length) return false;
  // A reference is a few lines long. A paragraph of a proof cut at its
  // theorem anchors is far longer, and carries a year often enough to pass the
  // test above — one arrived in a panel as a reference titled "Furthermore for
  // an example (x, y), we define…".
  const lengths = entries.map((entry) => entry.text.length).sort((a, b) => a - b);
  return lengths[Math.floor(lengths.length / 2)] <= MAX_ENTRY_LENGTH;
}

/** The page the printed "References" heading is on, if there is one.
 *
 * Looked for on each page that has destinations and on the page before it: a
 * list that begins halfway down a page has its heading on that page, and one
 * that begins at the top of a page has it on the page before.
 */
async function findHeading(dense) {
  const candidates = new Set();
  for (const page of dense) {
    candidates.add(page);
    if (page > 0) candidates.add(page - 1);
  }
  for (const page of [...candidates].sort((a, b) => a - b)) {
    const runs = await call('document.pageRuns', { page });
    if (!runs.some((run) => isReferencesHeading(run.text))) continue;
    // The first heading that actually has a list under it. A paper that says
    // "references" in a section title does not.
    const first = dense.find((candidate) => candidate >= page);
    if (first !== undefined && first - page <= 1) return page;
  }
  return null;
}

/** Cut one page of the reference list into entries.
 *
 * Each destination marks the entry it belongs to, so consecutive destinations
 * bound one entry between them. What a destination does *not* do is sit on the
 * entry: hyperref writes it wherever the anchor was placed relative to the
 * line, which is a whole line above the text in one paper and three points
 * above it in the next. Both are consistent within a document and neither is
 * predictable across documents, so no fixed offset works — the first attempt
 * used one and interleaved the tail of every entry with the head of the next.
 *
 * Snapping removes the need to know. Whatever the offset, the entry starts at
 * the first line at or below its destination, so the lines of the page — not a
 * guessed distance — decide where one entry ends and the next begins.
 */
function entriesOnPage(page, anchors, runs, links, previous) {
  if (anchors.length === 0) return [];
  const at = gutter(runs);
  // Left column first, then right: the order the page is read in, and the
  // order the entries are numbered in.
  const columns =
    at === null
      ? [{ runs, links }]
      : [
          {
            runs: runs.filter((run) => run.box.x < at),
            links: links.filter((link) => link.box.x < at),
          },
          {
            runs: runs.filter((run) => run.box.x >= at),
            links: links.filter((link) => link.box.x >= at),
          },
        ];
  const shared = byColumn(anchors, columns.length);
  for (let index = 0; index < columns.length; index++) columns[index].anchors = shared[index];

  const out = [];
  for (const column of columns) {
    const { entries, head } = entriesInColumn(page, column.anchors, column.runs, column.links);
    // Lines above the first destination in a column are not a reference of
    // their own: an entry that reached the foot of the previous column carries
    // on at the top of this one, and it has no second destination there
    // because it is not a second entry. Left to itself it became a row of its
    // own, titled with the middle of a sentence.
    const carry = out[out.length - 1] ?? previous;
    if (head && carry) extend(carry, head);
    out.push(...entries);
  }
  return out;
}

/** Add the tail of an entry, found in the next column or on the next page, to
 * the entry it belongs to. Re-read afterwards, since the missing half is often
 * the half with the title in it. */
function extend(entry, head) {
  const text = joinRuns(head.lines);
  if (!text) return;
  entry.text = `${entry.text} ${text}`.replace(/\s+/g, ' ').trim();
  Object.assign(entry, parseEntry(entry.text));
  if (entry.uri === null && head.links.length > 0) {
    const identified = head.links.find((link) => link.citation) ?? null;
    entry.uri = head.links[0].uri;
    entry.host = head.links[0].host;
    if (identified) {
      entry.citation = identified.citation;
      entry.kind = identified.citation.kind;
    } else if (entry.kind === 'entry') {
      entry.kind = 'link';
    }
  }
}

/** Which column each destination belongs to.
 *
 * Not decided by comparing its `x` to the gutter, which is the obvious test
 * and the wrong one. A destination's `x` is where the typesetter recorded the
 * anchor, and for a second column that is often a point in the *first* one —
 * one paper puts every right-hand entry at x=287 on a page whose right column
 * begins at x=308. Testing against the gutter read all sixteen of its
 * references as though they were in one column, and half of them came out as
 * fragments of the wrong entry.
 *
 * What is reliable is that the destinations of one column agree with each
 * other. They fall into as many tight groups as there are columns, in the same
 * left-to-right order, whatever the absolute positions are — so the groups are
 * matched to the columns by order and never by position. When the groups do
 * not come out one per column the pairing means nothing, and everything goes
 * into the first column rather than being split on a guess.
 */
function byColumn(anchors, count) {
  const empty = Array.from({ length: count }, () => []);
  if (count === 1) return [anchors];

  const groups = [];
  for (const anchor of [...anchors].sort((a, b) => a.x - b.x)) {
    const last = groups[groups.length - 1];
    if (last && anchor.x - last.x <= COLUMN_ANCHOR_SPREAD) {
      last.x = anchor.x;
      last.anchors.push(anchor);
      continue;
    }
    groups.push({ x: anchor.x, anchors: [anchor] });
  }
  if (groups.length !== count) {
    empty[0] = anchors;
    return empty;
  }
  return groups.map((group) => group.anchors);
}

/** How far apart two destinations can be and still belong to the same column.
 * Entries in a list line up; the gap to the next column is the width of a
 * column away. */
const COLUMN_ANCHOR_SPREAD = 24;

/** Where a page divides into two columns, or null if it does not.
 *
 * Found as the vertical line the fewest words cross. A one-column page has no
 * such line — every line of prose crosses the middle of it — and a two-column
 * page has a gutter that nothing crosses but the running header.
 *
 * Worth the trouble because a two-column bibliography is otherwise read
 * straight across the gutter: the first entry of the left column and the first
 * of the right become one line, and every entry on the page comes out as two
 * halves of two different references.
 */
function gutter(runs) {
  if (runs.length < 20) return null;
  const left = Math.min(...runs.map((run) => run.box.x));
  const right = Math.max(...runs.map((run) => run.box.x + run.box.w));
  const width = right - left;
  if (width <= 0) return null;

  let best = null;
  for (let at = left + width * 0.3; at <= left + width * 0.7; at += 2) {
    const crossing = runs.filter((run) => run.box.x < at && run.box.x + run.box.w > at).length;
    if (best === null || crossing < best.crossing) best = { at, crossing };
  }
  // A running header spans the gutter on some pages, so a few crossings are
  // allowed; a column of prose gives hundreds.
  if (best === null || best.crossing > runs.length * 0.04) return null;
  const before = runs.filter((run) => run.box.x + run.box.w <= best.at).length;
  const after = runs.filter((run) => run.box.x >= best.at).length;
  // Both sides must hold real text. A wide figure with a caption off to one
  // side also has an uncrossed line down it and is not a column.
  if (before < runs.length * 0.2 || after < runs.length * 0.2) return null;
  return best.at;
}

/** The entries in one column of one page. */
function entriesInColumn(page, anchors, runs, links) {
  if (anchors.length === 0 || runs.length === 0) return { entries: [], head: null };
  const ordered = [...anchors].sort((a, b) => b.y - a.y);
  const gap = lineGap(runs);
  const lines = toLines(runs, gap);
  if (lines.length === 0) return { entries: [], head: null };

  // Each destination, snapped to the line it introduces. Two that snap to the
  // same line are one entry — a paper anchors both the bibliography section
  // and its first entry at the top of the page — and one that snaps above an
  // entry already found is out of order and dropped rather than allowed to
  // reopen a range that has closed.
  const starts = [];
  for (const anchor of ordered) {
    const at = lines.findIndex((line) => line.top <= anchor.y + 1);
    if (at < 0) continue;
    const last = starts[starts.length - 1];
    if (last && at <= last.at) continue;
    starts.push({ at, anchor });
  }

  const out = [];
  for (let index = 0; index < starts.length; index++) {
    const from = starts[index].at;
    const to = index + 1 < starts.length ? starts[index + 1].at : lines.length;
    const text = joinRuns(lines.slice(from, to));
    // The section heading is a destination like any other, and on the page it
    // shares with the first entry it becomes a one-word reference.
    if (!text || isReferencesHeading(text)) continue;
    const startY = lines[from].top;
    const endY = to < lines.length ? lines[to].top : -Infinity;
    const parsed = parseEntry(text);
    const mine = links.filter((link) => link.box.y <= startY + 2 && link.box.y > endY + 2);
    const identified = mine.find((link) => link.citation) ?? null;
    out.push({
      key: `${page}:${starts[index].anchor.key}`,
      page,
      // The destination as the rest of the document spells it, so a jump can
      // be matched to its entry by equality rather than by falling inside a
      // range — the same number on both sides, no arithmetic to get wrong.
      anchor: starts[index].anchor.key,
      // Where the entry begins on the page. Used to tell a citation printed
      // above the reference list from a cross-reference inside it.
      top: startY,
      text,
      ...parsed,
      uri: mine[0]?.uri ?? null,
      host: mine[0]?.host ?? null,
      kind: identified ? identified.citation.kind : mine.length ? 'link' : 'entry',
      citation: identified ? identified.citation : null,
      citedOn: [],
      sites: [],
      citations: 0,
      // Kept under the name the link-only scan uses, so ordering, describing
      // and jumping do not each need to know which scan produced the row.
      pages: [page],
    });
  }
  const from = starts[0]?.at ?? lines.length;
  const above = lines.slice(0, from);
  const head =
    above.length > 0
      ? {
          lines: above,
          links: links.filter((link) => link.box.y > lines[from - 1].top - 1),
        }
      : null;
  return { entries: out, head };
}

/** One citation printed on a page is often several links.
 *
 * An author-year style splits every citation in two — natbib gives "Saxe et
 * al.," one link and "2014" another, with the comma between them belonging to
 * neither — and a citation that wraps is split again across the line break.
 * Left as they come, each half is its own row saying the same page twice, and
 * marking one of them draws a box around half a name.
 *
 * Grouped by reading order: pieces that follow each other with no more than a
 * space between them, or that carry on at the start of the next line, are one
 * citation. The union of a group covers the punctuation *between* its pieces,
 * because that punctuation lies between two boxes that are now one.
 *
 * What it cannot cover is punctuation outside every link — the brackets around
 * "(Saxe et al., 2014)", or the "Proposition 2.2" in "[RZ26, Proposition
 * 2.2]". The document does not link those, and the text runs are too coarse to
 * find their edges: one paper gives a whole line as a single run.
 *
 * Two separate citations of the *same* work on consecutive lines would be read
 * as one wrapped citation. That is the accepted cost: it shows one row where
 * there should be two, and still marks both places — where not grouping shows
 * two rows for one citation and marks half of it.
 */
function citationsIn(boxes) {
  const ordered = [...boxes].sort((a, b) => b.y - a.y || a.x - b.x);
  const groups = [];
  for (const box of ordered) {
    const group = groups[groups.length - 1];
    if (group && continues(group[group.length - 1], box)) group.push(box);
    else groups.push([box]);
  }
  return groups.map(perLine);
}

/** One box per line of a citation.
 *
 * The pieces on a line are united rather than kept apart, so the space and the
 * comma between "Saxe et al.," and "2014" are inside the mark instead of a
 * gap running through the middle of it. Only what is between two pieces of the
 * same citation is swallowed, and the widest that can be is the word space the
 * grouping already allowed.
 *
 * Lines stay separate, because a citation broken across one is two places on
 * the page and a box spanning both would cover everything between them. */
function perLine(group) {
  const lines = [];
  for (const box of group) {
    const line = lines[lines.length - 1];
    if (line && Math.abs(line.y - box.y) <= Math.max(line.h, box.h, 1) * 0.6) {
      const right = Math.max(line.x + line.w, box.x + box.w);
      line.x = Math.min(line.x, box.x);
      line.w = right - line.x;
      line.y = Math.max(line.y, box.y);
      line.h = Math.max(line.h, box.h);
      continue;
    }
    lines.push({ x: box.x, y: box.y, w: box.w, h: box.h });
  }
  return lines;
}

/** Whether `box` carries on from `previous` in the same citation. */
function continues(previous, box) {
  const height = Math.max(previous.h, box.h, 1);
  if (Math.abs(previous.y - box.y) <= height * 0.6) {
    // Side by side: no more than a word space between them.
    const gap = box.x - (previous.x + previous.w);
    return gap >= -1 && gap <= height * 0.6;
  }
  // Or the next line down, starting further left than the piece it follows —
  // which is what a citation broken across a line break looks like.
  const drop = previous.y - box.y;
  return drop > 0 && drop <= height * 2 && box.x < previous.x;
}

/** A destination, rounded onto a grid both sides of the match can agree on.
 *
 * The same entry is pointed at by every citation of it, and those destinations
 * are equal to within a fraction of a point rather than exactly. Rounding them
 * to the same half-point makes "is this jump this entry" an equality rather
 * than an interval test, which is one fewer thing to get wrong.
 */
function anchorOf(jump) {
  // A destination that fixes only a height has no column to belong to; nought
  // puts it in the first one, which is where a single-column page keeps
  // everything anyway.
  const x = Math.round((jump.toX ?? 0) * 2) / 2;
  const y = Math.round(jump.toY * 2) / 2;
  return { x, y, key: `${x}:${y}` };
}

/** Runs grouped into the lines they are printed on.
 *
 * A line is not one run and its runs do not share a `y`: a label set in one
 * font beside text set in another differ by a couple of points, and a line
 * broken into words is a dozen runs. Grouping by proximity puts them back
 * together, which matters because a destination is snapped to a *line* — and
 * treating the two halves of one line as two lines puts the boundary through
 * the middle of it.
 */
function toLines(runs, gap) {
  const sorted = [...runs].sort((a, b) => b.box.y - a.box.y);
  const tolerance = Math.max(3, gap * 0.4);
  const lines = [];
  for (const run of sorted) {
    const line = lines[lines.length - 1];
    if (line && line.top - run.box.y <= tolerance) {
      line.runs.push(run);
      continue;
    }
    lines.push({ top: run.box.y, runs: [run] });
  }
  for (const line of lines) line.runs.sort((a, b) => a.box.x - b.box.x);
  return lines;
}

/** The distance between one line of a page and the next.
 *
 * Taken as the commonest gap between distinct line positions rather than an
 * average, since a page mixes body text with headings and a mean lands between
 * the two and describes neither. */
function lineGap(runs) {
  const tops = [...new Set(runs.map((run) => Math.round(run.box.y)))].sort((a, b) => b - a);
  const gaps = [];
  for (let index = 1; index < tops.length; index++) {
    const gap = tops[index - 1] - tops[index];
    if (gap >= 4 && gap <= 40) gaps.push(gap);
  }
  if (gaps.length === 0) return 12;
  const counts = new Map();
  for (const gap of gaps) counts.set(gap, (counts.get(gap) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0] - b[0])[0][0];
}

/** Lines back into a paragraph of prose.
 *
 * A word the typesetter broke across a line arrives as two pieces with a
 * hyphen on the first, and joining those with a space spells "sensi- tivity".
 * Dropping the hyphen and closing the gap is right for a word that was broken
 * and wrong for a compound the author wrote that happened to break at its own
 * hyphen — "Vapnik-Chervonenkis" would become "VapnikChervonenkis". Nothing in
 * the file distinguishes the two, but a continuation that starts with a
 * capital or a digit is almost never the tail of a broken word, and that
 * recovers the proper nouns, which are the ones a reader notices. A lowercase
 * compound — "two-layer" split at its hyphen — still closes up wrongly; it is
 * the residue of a choice between two imperfect answers, and the common case
 * won.
 *
 * The hyphen itself is frequently not a hyphen. A TeX font gives the glyph no
 * Unicode value, so it arrives as a control character — but so do quotation
 * marks and braces in the same fonts, and reading every one of those as a
 * hyphen puts dashes through the middle of words. Only a trailing one, where a
 * break would put a hyphen, is read that way; the rest are dropped, having no
 * printable meaning to pass on.
 */
function joinRuns(lines) {
  let out = '';
  let broken = false;
  for (const line of lines) {
    const raw = line.runs
      .map((run) => run.text)
      .join(' ')
      .replace(/\s+/g, ' ')
      .trim();
    const ends = HYPHENATED.test(raw);
    const text = accented(raw)
      .replace(UNPRINTABLE, '')
      .replace(ends ? /[-\u2010]$/ : /(?!)/, '')
      .replace(/\s+/g, ' ')
      .trim();
    if (!text) continue;
    if (out.length === 0) {
      out = text;
      broken = ends;
      continue;
    }
    if (!broken) out += ' ';
    else if (/^[A-Z0-9]/.test(text)) out += '-';
    out += text;
    broken = ends;
  }
  return plain(out).replace(/\s+/g, ' ').trim();
}

/** Printer's punctuation as the characters a sidebar can show plainly.
 *
 * Curly quotes, en and em dashes, ligatures and non-breaking spaces are how a
 * typesetter sets a page and not how a list of references should read: they
 * vary with the font the PDF happened to use, so the same title comes out
 * looking different from two papers. Letters keep their accents — "Györfi" is
 * a name and "Gyorfi" is a different one.
 */
function plain(text) {
  return text
    .replace(/[\u2018\u2019\u201a\u2032]/g, "'")
    .replace(/[\u201c\u201d\u201e\u2033]/g, '"')
    .replace(/[\u2010\u2011\u2012\u2013\u2014\u2015\u2212]/g, '-')
    .replace(/\u2026/g, '...')
    .replace(/\ufb00/g, 'ff')
    .replace(/\ufb01/g, 'fi')
    .replace(/\ufb02/g, 'fl')
    .replace(/\ufb03/g, 'ffi')
    .replace(/\ufb04/g, 'ffl')
    .replace(/[\u00a0\u2007\u2009\u202f]/g, ' ');
}

/** Accents put back on the letters they belong to.
 *
 * TeX sets an accent as a glyph of its own, so "Györfi" arrives as "Gy¨orfi"
 * and sorts, searches and reads as though the mark were a letter. Moved after
 * the letter it precedes and composed, it becomes the letter it was always
 * meant to be. One that has drifted away from its letter entirely — "Hyvarinen,
 * ¨ A." — has nothing to attach to and is dropped rather than composed onto
 * whatever happens to follow it, which would spell a different name.
 */
function accented(text) {
  return text
    .replace(
      /([\u00a8\u00b4\u02c6\u02dc\u00af\u02d9\u02da\u02dd\u00b8])(\p{L})/gu,
      (_, mark, letter) => `${letter}${COMBINING[mark]}`.normalize('NFC')
    )
    .replace(/\s*[\u00a8\u00b4\u02c6\u02dc\u00af\u02d9\u02da\u02dd\u00b8]\s*/g, ' ');
}

/** Each spacing accent and the combining mark that does the same job. */
const COMBINING = {
  '\u00a8': '\u0308',
  '\u00b4': '\u0301',
  '\u02c6': '\u0302',
  '\u02dc': '\u0303',
  '\u00af': '\u0304',
  '\u02d9': '\u0307',
  '\u02da': '\u030a',
  '\u02dd': '\u030b',
  '\u00b8': '\u0327',
};

/** A line ending where the typesetter broke a word: a hyphen, or the glyph a
 * font with no Unicode mapping puts in its place.
 *
 * Control characters are the subject here rather than an oversight — a PDF
 * really does deliver its hyphens as U+0002 — so the rule against matching
 * them is off for these two patterns and nowhere else. */
// eslint-disable-next-line no-control-regex
const HYPHENATED = /[-\u2010\u00ad\u0000-\u0008\u000b-\u001f]$/;
/** Glyphs that reached us with no Unicode value at all. They are quotation
 * marks, braces and ligatures as often as hyphens, and there is nothing useful
 * to render for any of them. */
// eslint-disable-next-line no-control-regex
const UNPRINTABLE = /[\u0000-\u0008\u000b-\u001f\u00ad]/g;

// ---------- reading one entry ----------

/** Abbreviations that end in a period without ending a sentence. A reference
 * list is made of them, and splitting on every period turns one entry into
 * eleven fragments of which none is the title. */
const ABBREVIATIONS = new Set([
  'ed',
  'eds',
  'et al',
  'etc',
  'ex',
  'fig',
  'inc',
  'jr',
  'no',
  'pp',
  'proc',
  'sr',
  'st',
  'univ',
  'vol',
]);

/** A run of names written surname-first with initials — "Bortoli, V. D.,
 * Mathieu, E., ... and Doucet, A." — anchored at the start of an entry.
 *
 * Deliberately narrow. It has to end exactly where the title begins, so it
 * only accepts the shape it is sure of: a capitalised surname, a comma, and
 * one or more initials, repeated. A title that happens to start with a
 * capitalised word cannot extend it, because a title does not have a comma and
 * an initial after its first word. */
const AUTHOR_LIST = new RegExp(
  '^(?:' +
    "[A-Z][\\w'\u2019-]+(?:\\s+[A-Z][\\w'\u2019-]+)?" + // surname, sometimes two words
    ',\\s*' +
    '[A-Z]\\.(?:\\s*-?\\s*[A-Z]\\.)*' + // one or more initials, "C.-W." included
    '(?:,\\s*and\\s+|,\\s*&\\s*|\\s+and\\s+|\\s*&\\s*|,\\s*|\\s+)' +
    ')+'
);

/** Words that begin the part of a reference that is not its title. */
const VENUE = new RegExp(
  '^\\s*(in\\b|proceedings\\b|proc\\.|journal\\b|transactions\\b|conference\\b|advances in\\b' +
    '|arxiv\\b|corr\\b|preprint\\b|url\\b|doi\\b|https?:|pages?\\b|pp\\.|volume\\b|vol\\.' +
    '|edition\\b|springer\\b|elsevier\\b|wiley\\b|mit press\\b|cambridge\\b|oxford\\b|acm\\b' +
    '|ieee\\b|pmlr\\b|technical report\\b|master.s thesis\\b|ph\\.?d\\b|available at\\b)',
  'i'
);

/** Split an entry into sentences, keeping initials and abbreviations whole. */
function sentences(text) {
  const out = [];
  let at = 0;
  for (let index = 0; index < text.length; index++) {
    if (text[index] !== '.') continue;
    const next = text[index + 1];
    if (next !== undefined && next !== ' ') continue;
    const before = text.slice(at, index);
    // "V." and "A. C." are initials; so is a lone letter after a space.
    if (/(^|[\s.(])[A-Z]$/.test(before)) continue;
    const word = before
      .match(/([A-Za-z. ]+)$/)?.[1]
      ?.trim()
      .toLowerCase();
    if (word && ABBREVIATIONS.has(word)) continue;
    const piece = text.slice(at, index + 1).trim();
    if (piece) out.push(piece);
    at = index + 1;
  }
  const tail = text.slice(at).trim();
  if (tail) out.push(tail);
  return out;
}

/** A reference's label, authors, title and year, as far as they can be told
 * apart.
 *
 * Every field is allowed to come back null. A reference list is not a data
 * format and half of one is genuinely better than a wrong whole: a row that
 * shows a title and no year is useful, and a row that shows a venue where the
 * title should be is worse than one that shows the raw entry.
 */
function parseEntry(text) {
  let rest = text;
  let label = null;
  const bracketed = rest.match(/^\[([^\]]{1,24})\]\s*/);
  const numbered = rest.match(/^(\d{1,3})[.)]\s+/);
  if (bracketed) {
    label = bracketed[1];
    rest = rest.slice(bracketed[0].length);
  } else if (numbered) {
    label = numbered[1];
    rest = rest.slice(numbered[0].length);
  }

  const year = findYear(rest);

  // A title in quotation marks needs no working out: "Author & Author
  // \u201cTitle\u201d. In: Venue" is a whole citation style, and it has no full stop
  // between the authors and the title for a sentence split to find.
  const quoted = rest.match(/["\u201c\u201d]\s*([^"\u201c\u201d]{8,300}?)\s*["\u201c\u201d]/);
  if (quoted) {
    return {
      label,
      authors: tidy(stripYear(rest.slice(0, quoted.index))) || null,
      title: tidy(withoutTrailingYear(quoted[1], year)) || null,
      year,
    };
  }

  // "Efron, B. Tweedie's formula and selection bias. Journal of the American
  // Statistical Association, 2011." — a whole family of styles ends the author
  // list on an initial, and the full stop that closes the initial is the one
  // that opens the title. Sentence splitting cannot find it, because that same
  // full stop is the one it must *not* split on in "Vapnik, V. N. and
  // Chervonenkis, A." Recognising the author list for what it is settles both:
  // whatever follows the last "Surname, I." is the title.
  const named = rest.match(AUTHOR_LIST);
  if (named) {
    const after = rest.slice(named[0].length);
    const title = sentences(after)[0];
    if (title && !VENUE.test(title)) {
      return {
        label,
        authors: tidy(named[0].replace(/[,\s]+(and|&)?[,\s]*$/i, '')) || null,
        title: tidy(withoutTrailingYear(title, year)) || null,
        year,
      };
    }
  }

  const parts = sentences(rest);

  // Author-year style puts the date in parentheses right after the authors:
  // "Fukumizu, K. (1998). Effect of batch learning."
  const parenthesised = parts.findIndex((part) => /\(\d{4}[a-z]?\)\.?$/.test(part));
  let authorsAt = 0;
  let titleAt = 1;
  if (parenthesised >= 0 && parenthesised + 1 < parts.length) {
    authorsAt = parenthesised;
    titleAt = parenthesised + 1;
  }

  const authors = parts[authorsAt] ?? null;
  let title = null;
  for (let index = titleAt; index < parts.length; index++) {
    if (VENUE.test(parts[index])) break;
    title = parts[index];
    break;
  }

  return {
    label,
    authors: authors ? tidy(stripYear(authors)) : null,
    title: title ? tidy(withoutTrailingYear(title, year)) : null,
    year,
  };
}

/** The year a work was published.
 *
 * Reaching back to the seventeenth century, because reference lists do —
 * Pearson 1894 and Bayes 1763 are cited in papers written this year, and a
 * list that dates them "unknown" looks broken beside one that does not.
 *
 * Not simply the first four digits that could be one: "pages 1903-1943, 2019"
 * offers 1903 first, and a reference dated 1903 by its page numbers is a
 * plausible enough mistake to survive review. A number inside a range is not a
 * year, and a parenthesised one is a year whatever else the entry holds.
 */
function findYear(text) {
  const parenthesised = text.match(/\((\d{4})[a-z]?\)/);
  if (parenthesised) return Number(parenthesised[1]);
  // Not after a colon or a slash, and not before a dot and more digits:
  // "arXiv:2006.11239" and "abs/2202.02763" both offer a plausible year, and
  // a paper dated by its own preprint number is a quiet, convincing error.
  const years = [
    ...text.matchAll(/(^|[^\d:/\u2013\u2014-])((?:1[6-9]|20)\d{2})(?![\d\u2013\u2014-]|\.\d)/g),
  ];
  return years.length > 0 ? Number(years[years.length - 1][2]) : null;
}

function stripYear(authors) {
  return authors.replace(/\s*\(\d{4}[a-z]?\)\s*\.?$/, '');
}

/** A title without the date that trails it.
 *
 * An entry with no venue runs the year straight onto the title — "Exact
 * solutions to the nonlinear dynamics of learning, 2014" — and the row then
 * prints it twice, once in the title and again beside the authors. Removed
 * only when it is the year already found, so a title that genuinely ends in a
 * number keeps it. */
function withoutTrailingYear(title, year) {
  if (year === null) return title;
  return title.replace(new RegExp(`[,;]?\\s*${year}[a-z]?\\.?$`), '').trim();
}

/** A title, tidied: quotes off, and the full stop that ended the sentence it
 * was cut from off too, since that belongs to the entry rather than to the
 * title. */
function tidy(text, limit = 180) {
  return clip(
    text
      .replace(/^["“”'`]+|["“”'`]+$/g, '')
      .replace(/[.,;:]+$/, '')
      .trim(),
    limit
  );
}

/** No longer than a sidebar can show, cut on a word so what is left still
 * reads as words. Keeps the punctuation, which a sentence quoted off the page
 * needs and a title does not. */
function clip(text, limit) {
  const out = text.trim();
  if (out.length <= limit) return out;
  const cut = out.slice(0, limit);
  const space = cut.lastIndexOf(' ');
  return `${(space > limit * 0.6 ? cut.slice(0, space) : cut).trim()}...`;
}

function isReferencesHeading(text) {
  return /^\s*(\d+\.?\s*|[ivx]+\.?\s*)?(references|bibliography|works cited|literature cited)\s*$/i.test(
    text
  );
}

// ---------- reading the bibliography as bare links ----------

/** The reference list of a document with no internal links.
 *
 * This is the whole of what the panel could do before destinations were
 * followed, and it is still the only thing available for a paper that was not
 * typeset with them. It knows where a work is *listed* and cannot know where
 * it is cited, so its rows say so.
 */
async function scanByLinks(info, pages) {
  const found = new Map();
  // Links with no DOI or arXiv id — a JMLR paper, a book, a Semantic Scholar
  // entry — are references too. Collected per page and kept below only for
  // pages that also carry identified citations, which is how the bibliography
  // is told apart from a licence link in the footer.
  const plainByPage = new Map();
  for (const { page, links } of pages) {
    for (const link of links) {
      if (link.citation) {
        const { kind, value } = link.citation;
        const key = `${kind}:${value}`;
        const existing = found.get(key);
        if (existing) {
          if (!existing.pages.includes(page)) existing.pages.push(page);
        } else {
          found.set(key, { kind, value, key, pages: [page] });
        }
        continue;
      }
      if (!link.host) continue;
      if (!plainByPage.has(page)) plainByPage.set(page, []);
      plainByPage.get(page).push(link);
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

  for (const citation of collapsed)
    citation.citation = { kind: citation.kind, value: citation.value };
  await resolveAll(collapsed, info);
  const references = collapsed.filter((citation) => !citation.isSelf);

  for (const link of plain.values()) {
    link.title = null;
    link.inLibrary = false;
    link.citedOn = [];
    link.sites = [];
    link.citations = 0;
  }
  for (const entry of references) {
    entry.citedOn = [];
    entry.sites = [];
    entry.citations = 0;
  }
  return [...references, ...plain.values()];
}

// ---------- the library ----------

/** Fill in what your library knows about each entry.
 *
 * One call per entry that carries an identifier, and only for distinct works,
 * so it happens after the pages have been walked rather than inside the walk.
 */
async function resolveAll(entries, info) {
  for (const entry of entries) {
    if (!entry.citation) {
      entry.inLibrary = false;
      continue;
    }
    const resolved = await call('citations.resolve', {
      kind: entry.citation.kind,
      value: entry.citation.value,
    });
    entry.inLibrary = resolved.inLibrary;
    // The library's title is better than one read off the page — it is the
    // publisher's — but only where the page gave nothing.
    if (!entry.title) entry.title = resolved.title ?? resolved.fileName ?? null;
    // A paper prints its own identifier in its header, so it resolves to
    // itself and turns up at the top of its own reference list. Compared on a
    // normalised name because a library often holds the same paper twice —
    // "2605.25567v3.pdf" beside "2605.25567v3 (1).pdf" — and resolution may
    // answer with either copy.
    entry.isSelf = !!info && !!resolved.fileName && sameDocument(resolved.fileName, info.name);
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
    if (parsed.name !== info.name || parsed.pageCount !== info.pageCount) return null;
    return usable(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/** Whether a cached scan is the shape this build knows how to draw.
 *
 * The version in the key is supposed to answer this, and it is a promise made
 * by whoever edits the scan — which is exactly the promise that gets
 * forgotten. It was forgotten here: `sites` gained the boxes of each citation
 * and lost the single box it used to carry, under a number that had already
 * been bumped for a different change, and every cached document answered
 * clicks by throwing where a row was being built. The panel simply stopped
 * responding, with the reason only in the extension log.
 *
 * So the shape is checked rather than promised. Read against what the code
 * actually needs, a stale cache costs one rescan instead of a dead panel, and
 * a missed bump stops being able to break anything.
 */
function usable(scan) {
  if (!Array.isArray(scan.citations)) return false;
  return scan.citations.every(
    (citation) =>
      typeof citation?.key === 'string' &&
      Array.isArray(citation.pages) &&
      Array.isArray(citation.citedOn) &&
      Array.isArray(citation.sites) &&
      citation.sites.every(
        (site) =>
          Number.isInteger(site?.page) &&
          Array.isArray(site.rects) &&
          site.rects.every((rect) => typeof rect?.x === 'number' && typeof rect?.h === 'number')
      )
  );
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
  const { citations, pageCount, readBy } = current;

  if (citations.length === 0) {
    return draw({ empty: 'No references found.' });
  }

  const [sort, group] = await Promise.all([
    call('settings.get', { key: 'sort' }),
    call('settings.get', { key: 'group' }),
  ]);

  const ordered = [...citations].sort(comparator(sort));
  const heading =
    readBy === 'entries'
      ? `${citations.length} references`
      : `${citations.length} cited in ${pageCount} pages`;

  if (group === 'false') {
    return draw({ sections: [{ title: heading, rows: ordered.flatMap(rowsFor) }] });
  }

  // Counted in references, not in rows. Opening one adds a row for each place
  // it is cited, and a heading that counts rows then says the paper cites five
  // more works than it did a moment ago.
  const have = ordered.filter((c) => c.inLibrary);
  const missing = ordered.filter((c) => !c.inLibrary);
  const sections = [];
  if (have.length)
    sections.push({ title: `In your library (${have.length})`, rows: have.flatMap(rowsFor) });
  if (missing.length)
    sections.push({
      title: `Not in your library (${missing.length})`,
      rows: missing.flatMap(rowsFor),
    });
  return draw({ sections });
}

/** Where a row sits in the list.
 *
 * "appearance" means the first place the work is *cited* wherever that is
 * known, which is the order you met them in while reading. Falling back to
 * where it is listed keeps a document with no internal links in bibliography
 * order, which is the only order it has.
 */
function comparator(sort) {
  const firstAt = (entry) => entry.citedOn?.[0] ?? entry.pages[0];
  if (sort === 'most cited') {
    return (a, b) => (b.citations ?? 0) - (a.citations ?? 0) || firstAt(a) - firstAt(b);
  }
  if (sort === 'title') {
    return (a, b) => (a.title ?? a.value ?? '').localeCompare(b.title ?? b.value ?? '');
  }
  return (a, b) => firstAt(a) - firstAt(b) || (b.citations ?? 0) - (a.citations ?? 0);
}

/** A reference, and — when it is open — one row for each place it is cited.
 *
 * The panel draws a flat list, so an open row is the parent followed by its
 * citations rather than anything nested. That is enough: they read as a group
 * because they sit under it and each says which page it is on.
 */
function rowsFor(citation) {
  const rows = [toRow(citation)];
  if (!expanded.has(citation.key)) return rows;
  for (const site of citation.sites ?? []) {
    const line = context.get(siteKey(site));
    rows.push({
      id: `${citation.key}@${siteKey(site)}`,
      title: line ?? `Page ${site.page + 1}`,
      subtitle: line ? `page ${site.page + 1}` : 'go to this citation',
      // Drawn under the reference rather than beside it, because a citation of
      // a work is a detail of that work and not another entry in the list.
      nested: true,
      action: { command: 'goto', value: place(site) },
    });
  }
  return rows;
}

function toRow(citation) {
  return {
    id: citation.key,
    title: headline(citation),
    subtitle: describe(citation),
    ...(citation.inLibrary ? { badge: 'have it' } : {}),
    // The row carries its own id: activating it opens the places the work is
    // cited rather than picking one of them on your behalf. A reference with
    // no known citation sites has nothing to open and goes to its listing.
    action: { command: 'jump', value: citation.key },
  };
}

/** What a row is called: the title printed in the bibliography, then whatever
 * the library knows, then the identifier, then the address it was found at. A
 * row always has a name, and the best available one. */
function headline(citation) {
  if (citation.title) return citation.title;
  if (citation.value) return citation.value;
  if (citation.uri) return readableUrl(citation.uri);
  if (citation.text) return tidy(citation.text, 120);
  return citation.label ?? 'reference';
}

/** What a row is identified by: the scheme for a recognised work, the host for
 * everything else. */
function sourceOf(citation) {
  if (citation.kind === 'doi' || citation.kind === 'arxiv') return citation.kind;
  return citation.host ?? (citation.kind === 'entry' ? null : 'link');
}

/** "Kenji Fukumizu (1998), cited on pages 4, 7, 11" — the authors and year
 * where the entry gave them, and where the work is cited where the document's
 * destinations said so.
 *
 * A list read from bare links cannot know where anything is cited and says
 * "listed on" instead, because reporting the reference page as though it were
 * a citation site is the thing this panel used to get wrong.
 */
function describe(citation) {
  let out = '';
  if (citation.authors) out = tidy(citation.authors, 70);
  if (citation.year) out = out ? `${out} (${citation.year})` : String(citation.year);
  if (!out) out = sourceOf(citation) ?? '';

  const cited = citation.citedOn ?? [];
  const where =
    cited.length > 0
      ? `cited on ${pages(cited)}`
      : `listed on ${pages([...new Set(citation.pages)])}`;
  return out ? `${out}, ${where}` : where;
}

/** "pages 4, 7, 11" — with the tail trimmed, since a work cited on twenty
 * pages does not need all twenty listed in a sidebar. Open the row for those. */
function pages(list) {
  const shown = list.slice(0, 4).map((page) => page + 1);
  const more = list.length - shown.length;
  return `${list.length === 1 ? 'page' : 'pages'} ${shown.join(', ')}${
    more > 0 ? ` +${more} more` : ''
  }`;
}

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
  return withoutScheme.length > 64 ? `${withoutScheme.slice(0, 64)}...` : withoutScheme;
}

function draw(ui) {
  return call('ui.render', { panel: 'refs', ui });
}

/** Progress that does not throw away what is already listed. */
function drawProgress(status) {
  if (!current || current.citations.length === 0) return draw({ empty: `${status}...` });
  return draw({
    sections: [{ title: status, rows: current.citations.map(toRow) }],
  });
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}
