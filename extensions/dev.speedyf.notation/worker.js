// Notation - every symbol and acronym the open paper defines, the phrase that
// defines it, and a jump to where that happens.
//
// Long theory papers pile notation up: by section five you are flipping back
// to find what "NESS" stood for or which of the four lambdas was the control
// parameter. The paper already said, once, in a sentence of a recognisable
// shape - "nonequilibrium steady state (NESS)", "Let lambda denote the control
// parameter", "where m is the mass". This reads the text for those shapes.
//
// Built the same way as References, and for the same reasons:
//
//   * It reads the document's own text (`document.pageText`), so it works on
//     anything SpeedyF can select text in, with no network and no model.
//   * It renders progress while it reads, answers every invoke within the
//     five-second budget, and keeps reading in the background past that.
//   * It caches per document, so reopening the panel is instant and a rescan
//     is something you ask for.
//
// Everything here is plain text matching. It finds definitions written as
// sentences; it does not understand mathematics, and a symbol that is only
// ever defined by an equation in a display block is invisible to it.
//
// This file is ASCII on purpose: every non-ASCII character it matches is
// written as an escape, so the source reads the same in any editor.

/** Cache namespace. Bumped whenever what a scan produces changes, so a scan
 * made by older code is never drawn by newer code. */
const CACHE_VERSION = 3;
const CACHE_PREFIX = `scan:v${CACHE_VERSION}:`;

/** "Scan pages" setting values to page counts. */
const PAGE_LIMITS = { 'first 50': 50, 'first 200': 200, all: Infinity };
const DEFAULT_PAGE_LIMIT = 200;
/** Pages fetched at once. Each is a round trip to the host; a few in flight
 * at a time keeps the pipe busy without flooding it. */
const BATCH = 8;
/** How long an invoke waits for a scan before answering. The host gives five
 * seconds; the scan keeps going after this and draws when it finishes. */
const INVOKE_BUDGET_MS = 4000;
/** How often progress is drawn while reading, in pages. */
const PROGRESS_EVERY = 32;
/** The longest defining phrase shown. */
const MAX_PHRASE = 120;
/** How far a phrase may run past `MAX_PHRASE` to reach a natural end rather
 * than stop on "of" or "for which". */
const MAX_PHRASE_EXTENDED = 200;
/** Rows per section; the host caps a panel at 500 rows in total. */
const MAX_SYMBOL_ROWS = 300;
const MAX_ACRONYM_ROWS = 190;
/** What a row's action carries to find the definition on its page. */
const MAX_SNIPPET = 150;

const NO_DOCUMENT = 'Open a paper to see its notation.';

let nextRequestId = 1;
const pending = new Map();
/** The scan on screen: `{ key, name, pageCount, scanned, symbols, acronyms }`. */
let current = null;
/** The scan being read, if any: `{ key, promise }`. */
let job = null;

self.onmessage = (event) => {
  const message = event.data;

  if (message.type === 'invoke') {
    handle(message.command, message.value)
      .then(() => done(message.requestId, true))
      .catch((err) => done(message.requestId, false, String(err?.message ?? err)));
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

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

// ---------- commands ----------

async function handle(command, value) {
  if (command === 'notation') return show({ force: false });
  if (command === 'rescan') return show({ force: true });
  if (command === 'goto') return goTo(value);
  if (command === 'terms') return provideTerms(value);
  throw new Error(`unknown command "${command}"`);
}

async function show({ force }) {
  let info;
  try {
    info = await call('document.info');
  } catch {
    current = null;
    return draw({ empty: NO_DOCUMENT });
  }

  const limit = pageLimit(await setting('pages'));
  const scanned = Math.min(info.pageCount, limit);
  const key = cacheKey(info, scanned);

  if (!force) {
    if (current?.key === key && job?.key !== key) return render();
    const cached = job?.key === key ? null : await readCache(key, info);
    if (cached) {
      current = { ...cached, key };
      return render();
    }
  } else if (job?.key !== key) {
    try {
      await call('storage.remove', { key });
    } catch {
      // Nothing cached, or the store is unavailable: either way, read again.
    }
  }

  // A scan of this document already under way is waited for rather than
  // started twice: the panel asks again whenever it is shown.
  if (job?.key !== key) {
    const entry = { key };
    entry.promise = scan(info, scanned, entry).finally(() => {
      if (job === entry) job = null;
    });
    job = entry;
  }
  await withinBudget(job.promise);
}

/** Go to a definition: the page, framed on the sentence, with it marked.
 *
 * The value is `page|offset|text`, where text is the definition as printed
 * with its whitespace removed. The page's positioned runs are read and the
 * text found in them; whitespace is the one thing the runs and the page text
 * may disagree on, which is why it is not compared. */
async function goTo(value) {
  if (!value) return show({ force: false });
  const [where, , ...rest] = String(value).split('|');
  const page = Number(where);
  if (!Number.isInteger(page) || page < 0) throw new Error('no definition to go to');
  let runs = [];
  try {
    runs = await call('document.pageRuns', { page });
  } catch {
    runs = [];
  }
  const boxes = locate(Array.isArray(runs) ? runs : [], rest.join('|'));
  if (boxes.length === 0) {
    await call('navigate.toPage', { page });
    return;
  }
  await call('navigate.toPlace', { page, y: Math.max(...boxes.map((box) => box.y)), boxes });
}

// ---------- term provider ----------

/** Answer SpeedyF's request for the terms this document defines: every
 * acronym meaning, with the page and the place it is defined, so the core
 * can preview it wherever the acronym is used.
 *
 * Read the same way the panel reads - from the cache when there is one -
 * and answered with `terms.contribute` whenever the scan finishes, which
 * may be after this invoke has reported done. */
async function provideTerms(value) {
  const document = value && typeof value === 'object' ? value.document : null;
  if (typeof document !== 'string') throw new Error('no document to read terms for');
  await withinBudget(contributeTerms(document));
}

async function contributeTerms(document) {
  if ((await setting('acronyms')) === 'false') {
    await call('terms.contribute', { document, terms: [] });
    return;
  }
  let info;
  try {
    info = await call('document.info');
  } catch {
    return;
  }
  const scanned = Math.min(info.pageCount, pageLimit(await setting('pages')));
  const key = cacheKey(info, scanned);
  let result = current?.key === key ? current : await readCache(key, info);
  if (!result) {
    if (job?.key !== key) {
      const entry = { key, quiet: true };
      entry.promise = scan(info, scanned, entry).finally(() => {
        if (job === entry) job = null;
      });
      job = entry;
    }
    await job.promise;
    result = current?.key === key ? current : null;
  }
  if (!result) return;
  await call('terms.contribute', { document, terms: await acronymTerms(result.meanings ?? []) });
}

/** Acronym meanings as terms, each with the box of the sentence that
 * defines it - which is what keeps that sentence itself from previewing. */
async function acronymTerms(meanings) {
  const runsByPage = new Map();
  const terms = [];
  for (const def of meanings) {
    if (!runsByPage.has(def.page)) {
      let runs = [];
      try {
        runs = await call('document.pageRuns', { page: def.page });
      } catch {
        runs = [];
      }
      runsByPage.set(def.page, Array.isArray(runs) ? runs : []);
    }
    const boxes = locate(runsByPage.get(def.page), def.snippet);
    terms.push({
      text: def.key,
      caseSensitive: true,
      kind: 'acronym',
      title: def.key,
      body: def.long,
      page: def.page,
      ...(boxes.length > 0 ? { box: unionBox(boxes) } : {}),
    });
  }
  return terms;
}

/** One box around several, in the same top-edge space. */
function unionBox(boxes) {
  const left = Math.min(...boxes.map((b) => b.x));
  const right = Math.max(...boxes.map((b) => b.x + b.w));
  const top = Math.max(...boxes.map((b) => b.y));
  const bottom = Math.min(...boxes.map((b) => b.y - b.h));
  return { x: round(left), y: round(top), w: round(right - left), h: round(top - bottom) };
}

/** The boxes, one per printed line, that a run of text occupies on a page. */
function locate(runs, needle) {
  if (!needle || runs.length === 0) return [];
  let hay = '';
  const owner = [];
  runs.forEach((run, index) => {
    const text = typeof run?.text === 'string' ? run.text : '';
    for (let i = 0; i < text.length; i++) {
      if (/\s/.test(text[i])) continue;
      hay += text[i];
      owner.push(index);
    }
  });
  let at = hay.indexOf(needle);
  let length = needle.length;
  // The end of a long definition is the part most likely to have been cut
  // differently; its opening words are what matter for finding the line.
  if (at === -1 && needle.length > 24) {
    length = 24;
    at = hay.indexOf(needle.slice(0, length));
  }
  if (at === -1) return [];
  const used = [...new Set(owner.slice(at, at + length))].sort((a, b) => a - b);
  return mergeLines(used.map((index) => runs[index].box).filter(isBox));
}

function isBox(box) {
  return (
    box !== null &&
    typeof box === 'object' &&
    [box.x, box.y, box.w, box.h].every((value) => typeof value === 'number' && isFinite(value))
  );
}

/** Runs on the same line become one box, so a definition is marked as the
 * line or two it sits on rather than as a stack of word-sized boxes. A box's
 * `y` is its top edge, in page coordinates that grow upwards. */
function mergeLines(boxes) {
  const lines = [];
  for (const box of boxes) {
    const line = lines.find((other) => Math.abs(other.top - box.y) < Math.max(box.h, 1) / 2);
    if (!line) {
      lines.push({ left: box.x, right: box.x + box.w, top: box.y, bottom: box.y - box.h });
      continue;
    }
    line.left = Math.min(line.left, box.x);
    line.right = Math.max(line.right, box.x + box.w);
    line.top = Math.max(line.top, box.y);
    line.bottom = Math.min(line.bottom, box.y - box.h);
  }
  return lines.map((line) => ({
    x: round(line.left),
    y: round(line.top),
    w: round(line.right - line.left),
    h: round(line.top - line.bottom),
  }));
}

function round(value) {
  return Math.round(value * 10) / 10;
}

// ---------- scanning ----------

/** Read the first `scanned` pages and keep what they define.
 *
 * `entry` is this scan's place in `job`: once another scan replaces it - the
 * reader switched documents - this one stops at its next page read, because
 * the host answers page reads for whatever document is open *now*. */
async function scan(info, scanned, entry) {
  const extractor = createExtractor();
  const acronymsShown = (await setting('acronyms')) !== 'false';
  for (let start = 0; start < scanned; start += BATCH) {
    const pages = [];
    for (let page = start; page < Math.min(scanned, start + BATCH); page++) pages.push(page);
    const texts = await Promise.all(
      pages.map((page) => call('document.pageText', { page }).catch(() => ''))
    );
    if (job !== entry) return;
    texts.forEach((text, i) => extractor.addPage(pages[i], typeof text === 'string' ? text : ''));

    const read = pages[pages.length - 1] + 1;
    if (!entry.quiet && read < scanned && read % PROGRESS_EVERY < BATCH) {
      const partial = {
        name: info.name,
        pageCount: info.pageCount,
        scanned,
        ...extractor.result(),
      };
      await draw(describe(partial, acronymsShown, `Reading page ${read} of ${scanned}`));
    }
  }

  // The same document still open, or what was read is not about this one.
  let now = null;
  try {
    now = await call('document.info');
  } catch {
    now = null;
  }
  if (job !== entry || !now || now.name !== info.name || now.pageCount !== info.pageCount) return;

  const result = { name: info.name, pageCount: info.pageCount, scanned, ...extractor.result() };
  await writeCache(entry.key, result);
  current = { ...result, key: entry.key };
  await render();
}

function withinBudget(promise) {
  let timer;
  const budget = new Promise((resolve) => {
    timer = setTimeout(resolve, INVOKE_BUDGET_MS);
  });
  return Promise.race([promise, budget]).finally(() => clearTimeout(timer));
}

function pageLimit(value) {
  return PAGE_LIMITS[value] ?? DEFAULT_PAGE_LIMIT;
}

async function setting(key) {
  try {
    return await call('settings.get', { key });
  } catch {
    return null;
  }
}

// ---------- caching ----------

/** Keyed by name, length and how much was read: enough to tell two papers
 * apart, to notice a changed file, and to read again after "Scan pages"
 * changes. */
function cacheKey(info, scanned) {
  return `${CACHE_PREFIX}${scanned}:${info.pageCount}:${info.name}`.slice(0, 200);
}

async function readCache(key, info) {
  try {
    const raw = await call('storage.get', { key });
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (parsed.name !== info.name || parsed.pageCount !== info.pageCount) return null;
    return usable(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/** Whether a cached scan is the shape this build draws. Checked rather than
 * trusted: a stale cache should cost a rescan, not a broken panel. */
function usable(scan) {
  const fine = (list, extra) =>
    Array.isArray(list) &&
    list.every(
      (def) =>
        typeof def?.key === 'string' &&
        Number.isInteger(def.page) &&
        typeof def.snippet === 'string' &&
        extra(def)
    );
  return (
    fine(scan.symbols, (def) => typeof def.tex === 'string' && typeof def.phrase === 'string') &&
    fine(scan.acronyms, (def) => typeof def.long === 'string' && Number.isInteger(def.uses)) &&
    fine(scan.meanings, (def) => typeof def.long === 'string')
  );
}

async function writeCache(key, scan) {
  try {
    await call('storage.set', { key, value: JSON.stringify(scan) });
  } catch {
    // Too large for the store: it is simply not remembered.
  }
}

// ---------- drawing ----------

async function render() {
  if (!current) return draw({ empty: NO_DOCUMENT });
  const acronymsShown = (await setting('acronyms')) !== 'false';
  return draw(describe(current, acronymsShown));
}

/** The panel for a scan. `status` is a scan still in progress. */
function describe(scan, acronymsShown, status) {
  const sections = [];
  if (status) sections.push({ title: status, rows: [] });
  if (scan.symbols.length > 0) {
    const known = new Set(scan.symbols.map((def) => def.tex));
    sections.push({
      title: `Symbols (${scan.symbols.length})`,
      rows: scan.symbols.slice(0, MAX_SYMBOL_ROWS).map((def) => symbolRow(def, known)),
    });
  }
  if (acronymsShown && scan.acronyms.length > 0) {
    sections.push({
      title: `Acronyms (${scan.acronyms.length})`,
      rows: scan.acronyms.slice(0, MAX_ACRONYM_ROWS).map(acronymRow),
    });
  }
  const partial = scan.scanned < scan.pageCount;
  if (!status && partial && sections.length > 0) {
    sections.push({ title: `Read the first ${scan.scanned} of ${scan.pageCount} pages`, rows: [] });
  }
  const empty = status
    ? `${status}...`
    : partial
      ? `No notation found in the first ${scan.scanned} pages.`
      : 'No notation found';
  return { sections, empty };
}

/** A symbol's row: the symbol, the phrase that defines it, and the page in
 * the badge, where it does not wrap with the phrase. */
function symbolRow(def, known) {
  return {
    id: `s:${def.key}`.slice(0, 200),
    title: `$${def.tex}$`,
    subtitle: phraseText(def.phrase, known),
    badge: `p. ${def.page + 1}`,
    action: { command: 'goto', value: actionValue(def) },
  };
}

function acronymRow(def) {
  return {
    id: `a:${def.key}`.slice(0, 200),
    title: def.key,
    subtitle: asciiText(def.long),
    badge: def.uses > 0 ? `p. ${def.page + 1}, used ${def.uses}x` : `p. ${def.page + 1}`,
    action: { command: 'goto', value: actionValue(def) },
  };
}

function actionValue(def) {
  return `${def.page}|${def.at}|${def.snippet}`.slice(0, 480);
}

function draw(ui) {
  return call('ui.render', { panel: 'notation', ui });
}

// ---------- normalising page text ----------

// ascii-guard: data begin - Unicode recognised in page text
const SUPERSCRIPTS = {
  '⁰': '0',
  '¹': '1',
  '²': '2',
  '³': '3',
  '⁴': '4',
  '⁵': '5',
  '⁶': '6',
  '⁷': '7',
  '⁸': '8',
  '⁹': '9',
  '⁺': '+',
  '⁻': '-',
  ⁿ: 'n',
  ⁱ: 'i',
};

const SUBSCRIPTS = {
  '₀': '0',
  '₁': '1',
  '₂': '2',
  '₃': '3',
  '₄': '4',
  '₅': '5',
  '₆': '6',
  '₇': '7',
  '₈': '8',
  '₉': '9',
  '₊': '+',
  '₋': '-',
  ₐ: 'a',
  ₑ: 'e',
  ₒ: 'o',
  ₓ: 'x',
  ₕ: 'h',
  ₖ: 'k',
  ₗ: 'l',
  ₘ: 'm',
  ₙ: 'n',
  ₚ: 'p',
  ₛ: 's',
  ₜ: 't',
  ᵢ: 'i',
  ⱼ: 'j',
};

/** Typographic characters to what they stand for in running text. */
const REPLACE = {
  '−': '-',
  '‐': '-',
  '‑': '-',
  '–': '-',
  '—': '-',
  '‘': "'",
  '’': "'",
  '“': '"',
  '”': '"',
  '′': "'",
  '″': "''",
  '∗': '*',
  '≔': ':=',
  '­': '',
};
// ascii-guard: data end

const IS_SPACE = /\s/;
const IS_LOWER = /\p{Ll}/u;
/** Where a word was hyphenated across a line: kept as a soft hyphen until
 * the whole document has been read, then either closed up ("fluctuation-dis-"
 * / "sipation" is "dissipation") or turned back into the hyphen it was
 * ("mean-" / "field" is "mean-field"). See `resolveBreaks`. */
const BREAK = '\u00ad';

/** Page text in the one spelling the patterns below are written against.
 *
 * Mathematical italic and bold letters become plain ones (NFKC, a character at
 * a time), super- and subscript digits become `^2` and `_2`, typographic
 * dashes and quotes become ASCII, whitespace collapses to single spaces, and a
 * word hyphenated across a line is closed up to its hyphen. `map[i]` is where
 * the character at `text[i]` came from in `raw`, so a match can be located in
 * the page as printed. */
function normalizePage(raw) {
  const out = [];
  const map = [];
  let i = 0;
  while (i < raw.length) {
    const cp = raw.codePointAt(i);
    const ch = String.fromCodePoint(cp);
    if (IS_SPACE.test(ch)) {
      let j = i + 1;
      while (j < raw.length && IS_SPACE.test(raw[j])) j++;
      const prev = out[out.length - 1];
      const joinHyphen =
        (prev === '-' || prev === BREAK) &&
        IS_LOWER.test(out[out.length - 2] ?? '') &&
        IS_LOWER.test(raw[j] ?? '') &&
        // "x - y" is an operator, not a broken word.
        out[out.length - 2] !== ' ';
      if (joinHyphen) out[out.length - 1] = BREAK;
      if (out.length > 0 && prev !== ' ' && !joinHyphen) {
        out.push(' ');
        map.push(i);
      }
      i = j;
      continue;
    }
    let rep;
    // PDFium reports a hyphen it cannot map at a line end as U+0002.
    if (cp === 2) rep = '-';
    else if (cp < 0x80) rep = ch;
    else if (SUPERSCRIPTS[ch] !== undefined) rep = `^${SUPERSCRIPTS[ch]}`;
    else if (SUBSCRIPTS[ch] !== undefined) rep = `_${SUBSCRIPTS[ch]}`;
    else if (REPLACE[ch] !== undefined) rep = REPLACE[ch];
    else rep = ch.normalize('NFKC');
    for (let k = 0; k < rep.length; k++) {
      out.push(rep[k]);
      map.push(i);
    }
    i += ch.length;
  }
  return { text: out.join(''), map, raw };
}

/** The printed text behind `text[start, end)`, whitespace removed: what a
 * row carries so the definition can be found among the page's runs. */
function snippetOf(norm, start, end) {
  const from = norm.map[start] ?? 0;
  const to = end < norm.map.length ? norm.map[end] : norm.raw.length;
  return norm.raw.slice(from, to).replace(/\s+/g, '').slice(0, MAX_SNIPPET);
}

// ---------- acronyms ----------

/** Small words a long form may contain without the acronym spelling them:
 * "Department of Energy (DOE)". */
const STOPWORDS = new Set([
  'a',
  'an',
  'and',
  'as',
  'at',
  'by',
  'for',
  'from',
  'in',
  'into',
  'of',
  'on',
  'or',
  'over',
  'the',
  'to',
  'under',
  'via',
  'with',
]);

/** The acronym a token spells, or null. Two to ten characters, starting
 * with a letter, at least two capitals, and mostly capitals and digits; a
 * plural `s` is dropped ("DNNs"). Roman numerals are not acronyms. */
function acronymCore(token) {
  if (!/^[A-Za-z][A-Za-z0-9-]{1,10}$/.test(token)) return null;
  let core = token;
  if (/[A-Z0-9]s$/.test(core)) core = core.slice(0, -1);
  if (core.length < 2 || core.length > 10 || core.endsWith('-')) return null;
  const upper = (core.match(/[A-Z]/g) ?? []).length;
  const digits = (core.match(/[0-9]/g) ?? []).length;
  const width = core.replace(/-/g, '').length;
  if (upper < 2 || (upper + digits) / width < 0.6) return null;
  if (/^[IVX]+$/.test(core)) return null;
  return core;
}

const WORD_CHAR = /[\p{L}\p{N}]/u;

/** Where in `long` the shortest long form of `short` begins, or -1.
 *
 * Schwartz and Hearst's matcher: every letter of the acronym, right to left,
 * must appear in the long form in order, and its first letter must begin a
 * word. Letters from inside a word count, which is what reads
 * "nonequilibrium steady state" as NESS. Digits in the acronym are optional:
 * a long form rarely spells "2". */
function longFormStart(short, long) {
  let s = short.length - 1;
  let l = long.length - 1;
  while (s >= 0) {
    const c = short[s].toLowerCase();
    if (!/[a-z]/.test(c)) {
      s--;
      continue;
    }
    while (
      l >= 0 &&
      (long[l].toLowerCase() !== c || (s === 0 && l > 0 && WORD_CHAR.test(long[l - 1])))
    ) {
      l--;
    }
    if (l < 0) return -1;
    l--;
    s--;
  }
  return long.lastIndexOf(' ', l) + 1;
}

/** Words for matching initials: hyphenated words count as one per part. */
function initialWords(text) {
  return text.split(/[\s/-]+/).filter((word) => /^[\p{L}\p{N}]/u.test(word));
}

/** Whether the words' initials spell the acronym's letters, skipping small
 * words where needed and allowing one mismatched letter in an acronym of
 * three letters or more. The first letter must match. */
function initialsMatch(letters, words) {
  if (words.length === 0 || letters.length === 0) return false;
  if (words[0][0].toLowerCase() !== letters[0]) return false;
  const allowed = letters.length >= 3 ? 1 : 0;
  const memo = new Map();
  const go = (i, j, misses) => {
    if (j === words.length) return i === letters.length;
    const id = `${i}:${j}:${misses}`;
    if (memo.has(id)) return memo.get(id);
    const word = words[j];
    let ok = false;
    if (STOPWORDS.has(word.toLowerCase()) && j > 0) ok = go(i, j + 1, misses);
    if (!ok && i < letters.length) {
      const same = word[0].toLowerCase() === letters[i];
      if (same) ok = go(i + 1, j + 1, misses);
      else if (misses < allowed && i > 0) ok = go(i + 1, j + 1, misses + 1);
    }
    memo.set(id, ok);
    return ok;
  };
  return go(0, 0, 0);
}

/** The long form of `core` at the end of `before`, or null. */
function longFormBefore(core, before) {
  const letters = core.toLowerCase().replace(/[^a-z]/g, '');
  if (letters.length < 2) return null;
  const maxWords = Math.min(letters.length + 5, letters.length * 2);
  const words = before.split(' ').filter(Boolean).slice(-maxWords);
  const candidate = words.join(' ');

  // Initials first, allowing one slip: they read the whole long form, where
  // the letter match below takes the shortest - "populous estimator" for
  // "private populous estimator (PPE)". "Markov chain Monte Carlo (MCMC)"
  // passes either way; "carbon dioxide (CO2)" only this way.
  for (let k = 1; k <= words.length; k++) {
    const tail = words.slice(-k);
    if (STOPWORDS.has(tail[0].toLowerCase())) continue;
    if (initialsMatch([...letters], initialWords(tail.join(' ')))) {
      const long = tail.join(' ');
      if (plausibleLongForm(core, long, maxWords)) return long;
    }
  }
  const start = longFormStart(core, candidate);
  if (start !== -1) {
    const long = candidate.slice(start);
    // Letters found inside words reach far for a short acronym: "RE" in
    // "the resulting inequality yields". At most one word per letter, and
    // one more, for those.
    const content = long.split(' ').filter((w) => w && !STOPWORDS.has(w.toLowerCase()));
    const loose = letters.length <= 3 && content.length > letters.length + 1;
    if (!loose && plausibleLongForm(core, long, maxWords)) return long;
  }
  return null;
}

/** The long form of `core` that is the whole of `inside`, or null:
 * "NESS (nonequilibrium steady state)". */
function longFormInside(core, inside) {
  const letters = core.toLowerCase().replace(/[^a-z]/g, '');
  if (letters.length < 2) return null;
  const maxWords = Math.min(letters.length + 5, letters.length * 2);
  const text = inside.replace(/^(?:the|a|an)\s+/i, '');
  const start = longFormStart(core, text);
  if (start === 0 && plausibleLongForm(core, text, maxWords)) return text;
  if (initialsMatch([...letters], initialWords(text)) && plausibleLongForm(core, text, maxWords)) {
    return text;
  }
  return null;
}

function plausibleLongForm(core, long, maxWords) {
  const words = long.split(' ').filter(Boolean);
  if (words.length === 0 || words.length > maxWords) return false;
  if (long.length <= core.length) return false;
  if (!/^[\p{L}]/u.test(long)) return false;
  if (/[=<>[\]{}]|\d{3,}/.test(long)) return false;
  if (long.includes(core)) return false;
  return true;
}

/** "long form (ACRO)", optionally with a citation after a comma or semicolon
 * inside the parentheses. */
const ACRONYM_AFTER = /\(\s*([A-Za-z][A-Za-z0-9-]{1,10})\s*(?:[,;][^()]{0,40})?\)/g;
/** "ACRO (long form)". */
const ACRONYM_BEFORE = /(?<![\p{L}\p{N}\p{M}-])([A-Za-z][A-Za-z0-9-]{1,10}) \(([^()]{3,120})\)/gu;

/** The clause that ends at `end`: back to the nearest sentence or clause
 * punctuation, so a long form never reaches into the previous sentence. */
function clauseBefore(text, end) {
  const before = text.slice(Math.max(0, end - 160), end);
  let cut = -1;
  for (let i = before.length - 1; i >= 0; i--) {
    if ('.;:!?()[]{}"'.includes(before[i])) {
      cut = i;
      break;
    }
  }
  return before.slice(cut + 1).trim();
}

function findAcronyms(norm, page) {
  const { text } = norm;
  const found = [];

  for (const match of text.matchAll(ACRONYM_AFTER)) {
    // "long form (ACRO)" has a space before its parenthesis; "P(RE,j)" is a
    // function of something.
    if (match.index > 0 && text[match.index - 1] !== ' ') continue;
    const core = acronymCore(match[1]);
    if (!core) continue;
    const before = clauseBefore(text, match.index);
    const long = longFormBefore(core, before);
    if (!long) continue;
    // Back up to where the long form starts, for the snippet.
    const start = text.lastIndexOf(long, match.index);
    const from = start === -1 ? match.index : start;
    const end = match.index + match[0].length;
    found.push({
      key: core,
      long: tidyPhrase(long),
      page,
      at: norm.map[from] ?? 0,
      index: from,
      snippet: snippetOf(norm, from, end),
    });
  }

  for (const match of text.matchAll(ACRONYM_BEFORE)) {
    const core = acronymCore(match[1]);
    if (!core) continue;
    const inside = match[2].split(/[,;]|\s(?:see|cf\.|e\.g\.|i\.e\.)\s/)[0].trim();
    const long = longFormInside(core, inside);
    if (!long) continue;
    const end = match.index + match[0].length;
    found.push({
      key: core,
      long: tidyPhrase(long),
      page,
      at: norm.map[match.index] ?? 0,
      index: match.index,
      snippet: snippetOf(norm, match.index, end),
    });
  }

  return found.sort((a, b) => a.index - b.index);
}

// ---------- symbols ----------

const GREEK = '\\u0391-\\u03a9\\u03b1-\\u03c9\\u0127';
const MARKS = '\\u0300-\\u036f\\u20d0-\\u20ff';

/** A symbol as it comes out of a PDF's text: one letter (Latin or Greek),
 * any accents, up to three of `_i` / `^2` / `_{ij}` / `1` / `'` / `*`, a
 * subscript set off by a space the way extraction often leaves it ("x i"),
 * and a short argument ("Z(theta)", "f(x, t)"). */
const SYMBOL_CORE =
  `[A-Za-z${GREEK}][${MARKS}]*` +
  `(?:[_^](?:\\{[^{}\\s]{1,10}\\}|[A-Za-z0-9${GREEK}+*-]{1,3})|[0-9]{1,2}(?![0-9])|'{1,2}|\\*){0,3}` +
  '(?: [ijkmnt0-9](?= ))?' +
  '(?:\\([^()\\s]{1,8}(?:, ?[^()\\s]{1,8})?\\))?';
const SYM = `(?<![\\p{L}\\p{N}\\p{M}_^\\\\'])(${SYMBOL_CORE})(?![\\p{L}\\p{N}\\p{M}_^])`;

/** The defining sentence shapes. In each, group 1 is the symbol; where the
 * phrase comes before the symbol it is group 2, otherwise it is read from
 * where the match ends. */
const SYMBOL_PATTERNS = [
  // Let lambda denote the control parameter. / Let N be the number of sites.
  new RegExp(
    `\\b(?:[Ll]et|[Ll]etting)\\s+${SYM}\\s+(?:denotes?|be|represents?|stands? for|refers? to)\\s+`,
    'gu'
  ),
  // where m is the mass / where Z(theta) denotes the partition function
  new RegExp(
    `\\b[Ww]here\\s+${SYM}\\s*,?\\s+(?:is|are|denotes|represents|stands for|refers to|is defined as)\\s+(?!given\\b)`,
    'gu'
  ),
  // lambda denotes the control parameter / P_N stands for ...
  new RegExp(`${SYM}\\s+(?:denotes|represents|stands for)\\s+`, 'gu'),
  // we write x_i for the position of the ith particle
  new RegExp(
    `\\b[Ww]e\\s+(?:will\\s+)?(?:write|use)\\s+${SYM}\\s+(?:for|to denote|to represent)\\s+`,
    'gu'
  ),
  // define beta as the inverse temperature
  new RegExp(`\\b[Dd]efine\\s+${SYM}\\s+(?:as|to be|by)\\s+`, 'gu'),
  // denote by S the entropy
  new RegExp(`\\b[Dd]enote\\s+by\\s+${SYM}\\s+(?=(?:the|a|an)\\s)`, 'gu'),
  // F := -log Z
  new RegExp(`${SYM}\\s*:=\\s*`, 'gu'),
  // H is called the Hamiltonian
  new RegExp(`${SYM}\\s+is\\s+called\\s+`, 'gu'),
  // with lambda the control parameter
  new RegExp(`\\bwith\\s+${SYM}\\s+(?=(?:the|a|an|its)\\s)`, 'gu'),
];

/** Shapes where the phrase comes first. Group 1 is the phrase, 2 the symbol. */
const SYMBOL_PATTERNS_BEFORE = [
  // denote the inverse temperature by beta / write the Hamiltonian as H
  new RegExp(
    `\\b(?:[Dd]enote|[Ww]rite)\\s+([Tt]he\\s+[^.;:()]{3,80}?)\\s+(?:by|as)\\s+${SYM}(?!\\s*\\()`,
    'gu'
  ),
  // the partition function, denoted Z(theta) / is denoted by Z
  new RegExp(`([Tt]he\\s+[^.;:,()]{3,80}?)\\s*,?\\s+(?:is\\s+)?denoted\\s+(?:by\\s+)?${SYM}`, 'gu'),
];

/** What may follow a symbol for it to start a definition of its own: a
 * defining verb, or straight to "the", "a" or "an". */
const DEFINES_NEXT =
  '(?:(?:is|are|denotes|represents|stands\\s+for)\\s+(?!called\\b|given\\b)|(?=(?:the|a|an|its)\\s))';

/** "and k the spring constant", following a "where m is the mass" - or any
 * other definition: "Let F be an infinite set and C a collection of ...". */
const FOLLOW_ON = new RegExp(`^\\s*(?:[,;]\\s*)?(?:and\\s+)?${SYM}\\s+${DEFINES_NEXT}`, 'u');

/** An "and" that starts the next definition rather than continuing this
 * one: "the diameter of the set and rho is the distance ...". Any other
 * "and" belongs to the phrase ("if and only if"). */
const COORDINATED = new RegExp(`^ and\\s+${SYM}\\s+${DEFINES_NEXT}`, 'u');

/** Words that put a capital letter in prose without making it a symbol:
 * "Figure A denotes", "Appendix B represents". */
const LABEL_BEFORE =
  /\b(?:Fig|Figs|Figure|Figures|Table|Tables|Section|Sec|Secs|Eq|Eqs|Equation|Appendix|Chapter|Theorem|Lemma|Proposition|Corollary|Definition|Ref|Refs|Step|Case|Part|Panel|Type|Class|Model|Phase|Region|Point|Grade|Vitamin|Plan)\.?\s*$/;

function precededByLabel(text, at) {
  return LABEL_BEFORE.test(text.slice(Math.max(0, at - 16), at));
}

function findSymbols(norm, page) {
  const { text } = norm;
  const found = [];
  const add = (symbol, symbolAt, clean, start, end) => {
    if (precededByLabel(text, symbolAt)) return;
    if (!clean || !/\p{L}.*\p{L}/u.test(clean)) return;
    const tex = symbolTex(symbol);
    if (!tex) return;
    found.push({
      key: tex,
      tex,
      phrase: clean,
      page,
      at: norm.map[start] ?? 0,
      index: start,
      snippet: snippetOf(norm, start, end),
    });
  };

  for (const pattern of SYMBOL_PATTERNS) {
    for (const match of text.matchAll(pattern)) {
      const symbolAt = match.index + match[0].indexOf(match[1]);
      const end = match.index + match[0].length;
      const phrase = definitionPhrase(text, end);
      add(match[1], symbolAt, phrase.text, match.index, phrase.end);

      // "where m is the mass, k the spring constant, and g the ...", or
      // "Let F be an infinite set and C a collection of subsets of F".
      let from = phrase.end;
      for (let more = 0; more < 6; more++) {
        const next = FOLLOW_ON.exec(text.slice(from, from + 200));
        if (!next) break;
        const at = from + next.index;
        const nextPhrase = definitionPhrase(text, at + next[0].length);
        add(next[1], at + next[0].indexOf(next[1]), nextPhrase.text, at, nextPhrase.end);
        if (nextPhrase.end <= from) break;
        from = nextPhrase.end;
      }
    }
  }

  for (const pattern of SYMBOL_PATTERNS_BEFORE) {
    for (const match of text.matchAll(pattern)) {
      // The nearest "the" to the symbol: a lazy match still starts at the
      // first one in the sentence.
      let phrase = match[1];
      const last = phrase.lastIndexOf(' the ');
      if (last > 0) phrase = phrase.slice(last + 1);
      const symbolAt = match.index + match[0].lastIndexOf(match[2]);
      add(match[2], symbolAt, tidyPhrase(phrase), match.index, match.index + match[0].length);
    }
  }

  return found.sort((a, b) => a.index - b.index);
}

/** The clause starting at `from`: up to the end of the sentence, a semicolon,
 * a colon, a comma at the top level, a closing bracket that was never opened,
 * a "where" or "which", or an "and" that starts the next definition -
 * whichever comes first. `stop` says which kind of end it was: `hard` (the
 * sentence or clause is over), `soft` (a comma, "where" or "which", which a
 * phrase may run on past), or `next` (another definition starts here). */
function phraseAt(text, from) {
  const tail = text.slice(from, from + 240);
  let depth = 0;
  for (let i = 0; i < tail.length; i++) {
    const c = tail[i];
    const at = (stop) => ({ text: tail.slice(0, i), end: from + i, stop });
    if (c === '(' || c === '[' || c === '{') depth++;
    else if (c === ')' || c === ']' || c === '}') {
      if (depth === 0) return at('hard');
      depth--;
    } else if (depth === 0) {
      if (c === ';' || (c === ':' && tail[i + 1] !== '=')) return at('hard');
      if (c === ',' && i >= 3) return at('soft');
      if (
        c === '.' &&
        (i + 1 >= tail.length || tail[i + 1] === ' ') &&
        !/(?:^|\s)(?:e\.g|i\.e|cf|al|resp|Eq|Eqs|Fig|Figs|Ref|Refs|Sec|vs|approx)$/.test(
          tail.slice(0, i)
        )
      ) {
        return at('hard');
      }
      if (tail.startsWith(' where ', i) || tail.startsWith(' which ', i)) return at('soft');
      if (tail.startsWith(' and ', i) && COORDINATED.test(tail.slice(i))) return at('next');
    }
  }
  return { text: tail, end: from + tail.length, stop: 'hard' };
}

/** Words a phrase cannot end on: it would stop mid-thought ("events on",
 * "the value of r for which"). */
const DANGLING = new Set([
  'on',
  'of',
  'for',
  'which',
  'that',
  'to',
  'by',
  'with',
  'in',
  'at',
  'from',
  'as',
  'whose',
  'where',
  'when',
  'and',
  'or',
  'the',
  'a',
  'an',
  // Verbs left waiting for what they say: "taken to be", "at which f is".
  'be',
  'is',
  'are',
]);

function squash(phrase) {
  return phrase
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[\s,;:.-]+$/, '');
}

/** Whether a phrase ends on a word it cannot end on. A capital A is a
 * symbol, not an article. */
function dangles(phrase) {
  const last = /(?:^|\s)([A-Za-z]+)$/.exec(phrase);
  return !!last && last[1] !== 'A' && DANGLING.has(last[1].toLowerCase());
}

/** Where the last bracket opened and never closed is, or -1. */
function unclosed(phrase) {
  const open = [];
  for (let i = 0; i < phrase.length; i++) {
    if ('([{'.includes(phrase[i])) open.push(i);
    else if (')]}'.includes(phrase[i])) open.pop();
  }
  return open.length > 0 ? open[open.length - 1] : -1;
}

const complete = (phrase) => !dangles(phrase) && unclosed(phrase) === -1;

/** A phrase cut back to its last complete word: out of any open bracket and
 * off any dangling words. */
function trimBack(phrase) {
  let out = phrase;
  for (let open = unclosed(out); open !== -1; open = unclosed(out)) out = out.slice(0, open);
  out = squash(out);
  while (dangles(out)) out = squash(out.replace(/\s*[A-Za-z]+$/, ''));
  return out || squash(phrase);
}

/** At the word before `limit`. */
function cutAt(phrase, limit) {
  const cut = phrase.lastIndexOf(' ', limit);
  return squash(phrase.slice(0, cut > 40 ? cut : limit));
}

/** The defining phrase starting at `from`, as `{ text, end }`.
 *
 * The clause `phraseAt` finds, unless it would end mid-thought: then it runs
 * on past commas and "which" to the next natural end, up to
 * `MAX_PHRASE_EXTENDED` characters. A phrase that still cannot end is cut
 * back to its last complete word and marked "..."; one the sentence itself
 * leaves hanging - extraction dropped inline math, "events on ." - keeps its
 * words and gets " ..." where the math was. */
function definitionPhrase(text, from) {
  const first = phraseAt(text, from);
  const clause = squash(first.text);
  if (clause.length <= MAX_PHRASE && complete(clause)) return { text: clause, end: first.end };
  const short = clause.length > MAX_PHRASE ? cutAt(clause, MAX_PHRASE) : clause;
  if (clause.length > MAX_PHRASE && complete(short)) return { text: `${short}...`, end: first.end };

  // Run on to the next natural end.
  if (clause.length <= MAX_PHRASE_EXTENDED && complete(clause)) {
    return { text: clause, end: first.end };
  }
  let last = first;
  let longer = clause;
  while (last.stop === 'soft' && longer.length <= MAX_PHRASE_EXTENDED) {
    const next = phraseAt(text, last.end + 1);
    if (next.end <= last.end) break;
    last = next;
    longer = squash(text.slice(from, next.end));
    if (longer.length <= MAX_PHRASE_EXTENDED && complete(longer)) {
      return { text: longer, end: next.end };
    }
  }

  if (last.stop === 'hard' && longer.length <= MAX_PHRASE && unclosed(longer) === -1) {
    return { text: `${longer} ...`, end: last.end };
  }
  return { text: `${trimBack(short)}...`, end: first.end };
}

/** A phrase cut to length at a word, with surrounding punctuation dropped.
 * For acronyms' long forms and phrases that come before their symbol. */
function tidyPhrase(phrase) {
  let out = phrase
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[\s,;:.-]+$/, '');
  if (out.length > MAX_PHRASE) {
    const cut = out.lastIndexOf(' ', MAX_PHRASE);
    out = `${out.slice(0, cut > 40 ? cut : MAX_PHRASE)}...`;
  }
  return out;
}

// ---------- TeX ----------

const GREEK_UPPER = [
  'A',
  'B',
  '\\Gamma',
  '\\Delta',
  'E',
  'Z',
  'H',
  '\\Theta',
  'I',
  'K',
  '\\Lambda',
  'M',
  'N',
  '\\Xi',
  'O',
  '\\Pi',
  'P',
  null,
  '\\Sigma',
  'T',
  '\\Upsilon',
  '\\Phi',
  'X',
  '\\Psi',
  '\\Omega',
];
const GREEK_LOWER = [
  '\\alpha',
  '\\beta',
  '\\gamma',
  '\\delta',
  '\\varepsilon',
  '\\zeta',
  '\\eta',
  '\\theta',
  '\\iota',
  '\\kappa',
  '\\lambda',
  '\\mu',
  '\\nu',
  '\\xi',
  'o',
  '\\pi',
  '\\rho',
  '\\varsigma',
  '\\sigma',
  '\\tau',
  '\\upsilon',
  '\\varphi',
  '\\chi',
  '\\psi',
  '\\omega',
];

/** TeX for one Greek letter (or hbar), or null. */
function greekTex(ch) {
  const cp = ch.codePointAt(0);
  if (cp >= 0x391 && cp <= 0x3a9) return GREEK_UPPER[cp - 0x391];
  if (cp >= 0x3b1 && cp <= 0x3c9) return GREEK_LOWER[cp - 0x3b1];
  if (cp === 0x127) return '\\hbar';
  return null;
}

// ascii-guard: data begin - Unicode recognised in page text
const ACCENTS = {
  '̂': '\\hat',
  '̃': '\\tilde',
  '̄': '\\bar',
  '̅': '\\bar',
  '̆': '\\breve',
  '̇': '\\dot',
  '̈': '\\ddot',
  '̌': '\\check',
  '⃗': '\\vec',
};

/** Symbols and operators that turn up in phrases, as TeX. */
const MATH_TEX = {
  '∈': '\\in',
  '∉': '\\notin',
  '→': '\\to',
  '↦': '\\mapsto',
  '∞': '\\infty',
  '≈': '\\approx',
  '±': '\\pm',
  '∂': '\\partial',
  '∇': '\\nabla',
  '∑': '\\sum',
  '∏': '\\prod',
  '∫': '\\int',
  '≠': '\\neq',
  '≤': '\\leq',
  '≥': '\\geq',
  '∼': '\\sim',
  '⟨': '\\langle',
  '⟩': '\\rangle',
  '×': '\\times',
  '·': '\\cdot',
  '⋅': '\\cdot',
  '∝': '\\propto',
  '≡': '\\equiv',
  '⊂': '\\subset',
  '⊆': '\\subseteq',
  '∪': '\\cup',
  '∩': '\\cap',
  '∀': '\\forall',
  '∃': '\\exists',
  '⊗': '\\otimes',
  '⊕': '\\oplus',
  '†': '\\dagger',
  '⊥': '\\perp',
  '∅': '\\emptyset',
  '∘': '\\circ',
};
// ascii-guard: data end

/** Characters TeX-escaped or converted, one at a time. */
function texChars(text) {
  let out = '';
  for (const ch of text) {
    const greek = greekTex(ch);
    if (greek) out += `${greek} `;
    else if (MATH_TEX[ch]) out += `${MATH_TEX[ch]} `;
    else if (ch === '{' || ch === '}' || ch === '%' || ch === '#' || ch === '&') out += `\\${ch}`;
    else if (ch === '$' || ch === '\\') continue;
    else if (ch.charCodeAt(0) < 0x80) out += ch;
  }
  return out;
}

/** Drop the spaces `texChars` leaves after a command where TeX does not need
 * them, so `$...$` never ends in a space. */
function tightTex(tex) {
  return tex.replace(/ +(?=[\s}_^(),'*]|$)/g, '').trim();
}

/** A symbol as TeX: `x_i`, `\lambda`, `Z(\theta)`, `\hat{x}`, `x_{1}`. */
function symbolTex(symbol) {
  const s = symbol.replace(/^([^ (]+) ([ijkmnt0-9])/, '$1_$2').replace(/\s+/g, '');
  const chars = [...s];
  if (chars.length === 0) return null;
  let i = 0;
  let out = texChars(chars[i++]);
  if (!out) return null;
  while (i < chars.length && ACCENTS[chars[i]] !== undefined) {
    out = `${ACCENTS[chars[i]]}{${tightTex(out)}}`;
    i++;
  }
  while (i < chars.length && /[\u0300-\u036f\u20d0-\u20ff]/.test(chars[i])) i++;
  while (i < chars.length) {
    const c = chars[i];
    if (c === '_' || c === '^') {
      i++;
      let arg = '';
      if (chars[i] === '{') {
        i++;
        while (i < chars.length && chars[i] !== '}') arg += chars[i++];
        i++;
      } else {
        while (
          i < chars.length &&
          arg.length < 3 &&
          /[A-Za-z0-9+*\u0391-\u03a9\u03b1-\u03c9-]/.test(chars[i])
        ) {
          arg += chars[i++];
        }
      }
      if (arg) out += `${c}{${tightTex(texChars(arg))}}`;
    } else if (/[0-9]/.test(c)) {
      let digits = '';
      while (i < chars.length && /[0-9]/.test(chars[i])) digits += chars[i++];
      out += `_{${digits}}`;
    } else if (c === "'") {
      out += "'";
      i++;
    } else if (c === '*') {
      out += '^{*}';
      i++;
    } else {
      out += texChars(chars.slice(i).join(''));
      break;
    }
  }
  return tightTex(out);
}
// ascii-guard: data begin - Unicode recognised in page text

/** Typographic characters that read as ASCII. */
const PLAIN = {
  '…': '...',
  '‘': "'",
  '’': "'",
  '‚': "'",
  '“': '"',
  '”': '"',
  '„': '"',
  '«': '"',
  '»': '"',
  '‐': '-',
  '‑': '-',
  '–': '-',
  '—': ' - ',
  '−': '-',
  '•': '-',
  '′': "'",
  '″': "''",
  '\u00a0': ' ',
  '\u2009': ' ',
  '\u00ad': '',
};
// ascii-guard: data end

/** Text from the page as something the panel can show: ASCII, with Greek
 * letters, mathematical symbols and super- and subscripts as `$...$` TeX the
 * panel typesets, and any `$` already in the text escaped so it is not read
 * as math. Ligatures and styled letters get their ASCII spelling; a letter
 * with an accent, or from another alphabet, is kept as written, since a name
 * is spelled the way its owner spells it. The References extension has a
 * copy of this as `displayText`. */
function asciiText(text) {
  return asciiParts(text).replace(/\s+/g, ' ').trim();
}

/** `asciiText` without the closing tidy, for text that is one piece of a
 * longer line: its edge spaces are kept. */
function asciiParts(text) {
  const parts = [];
  const push = (kind, value) => {
    const last = parts[parts.length - 1];
    if (last && last.kind === kind) last.value += kind === 'math' ? ` ${value}` : value;
    else parts.push({ kind, value });
  };
  /** A run of raised or lowered characters, onto the math before it, or the
   * letter or number it sits on. */
  const script = (mark, run) => {
    const last = parts[parts.length - 1];
    if (last?.kind === 'math') {
      last.value += `${mark}{${run}}`;
      return;
    }
    const base = last
      ? /(?:^|[^A-Za-z0-9.])([A-Za-z]|[0-9]+(?:\.[0-9]+)?)$/.exec(last.value)
      : null;
    if (base) {
      last.value = last.value.slice(0, last.value.length - base[1].length);
      if (!last.value) parts.pop();
      parts.push({ kind: 'math', value: `${base[1]}${mark}{${run}}` });
    } else {
      parts.push({ kind: 'math', value: `${mark}{${run}}` });
    }
  };
  const chars = Array.from(text.normalize('NFC'));
  for (let i = 0; i < chars.length; i++) {
    const ch = chars[i];
    if (SUPERSCRIPTS[ch] !== undefined || SUBSCRIPTS[ch] !== undefined) {
      const table = SUPERSCRIPTS[ch] !== undefined ? SUPERSCRIPTS : SUBSCRIPTS;
      let run = '';
      while (i < chars.length && table[chars[i]] !== undefined) run += table[chars[i++]];
      i--;
      script(table === SUPERSCRIPTS ? '^' : '_', run);
      continue;
    }
    const code = ch.charCodeAt(0);
    const greek = greekTex(ch);
    if (ch === '$') push('text', '\\$');
    else if (ch === '\\') continue;
    else if (code < 0x20) push('text', ' ');
    else if (code < 0x80) {
      // A digit straight after math belongs to it: "$\lambda$2" would not
      // close as math.
      const last = parts[parts.length - 1];
      if (last?.kind === 'math' && /[0-9]/.test(ch)) last.value += ch;
      else push('text', ch);
    } else if (greek) push('math', greek);
    else if (MATH_TEX[ch]) push('math', MATH_TEX[ch]);
    else if (PLAIN[ch] !== undefined) push('text', PLAIN[ch]);
    else {
      const plain = ch.normalize('NFKC');
      if (/^[\x20-\x7e]+$/.test(plain)) push('text', plain);
      else if (/^[\p{L}\p{M}]+$/u.test(ch)) push('text', ch);
    }
  }
  return parts
    .map((part) => (part.kind === 'math' ? `$${part.value.trim()}$` : part.value))
    .join('');
}

/** A symbol-shaped token in running text: `SYMBOL_CORE` without the
 * space-set subscript (in prose "x i" is more often two words), standing
 * alone - not part of a word, an abbreviation ("e.g.", "J. Smith") or a
 * hyphenated compound ("n-dimensional"). */
const PHRASE_SYMBOL = new RegExp(
  `(?<![\\p{L}\\p{N}\\p{M}_^\\\\'.-])` +
    `([A-Za-z${GREEK}][${MARKS}]*` +
    `(?:[_^](?:\\{[^{}\\s]{1,10}\\}|[A-Za-z0-9${GREEK}+*-]{1,3})|[0-9]{1,2}(?![0-9])|'{1,2}|\\*){0,3}` +
    '(?:\\([^()\\s]{1,8}(?:, ?[^()\\s]{1,8})?\\))?)' +
    `(?![\\p{L}\\p{N}\\p{M}_^']|\\.\\s?\\p{L}|-\\p{L})`,
  'gu'
);

/** A comparison with a number, after a formula: "x < 1". */
const TRAILING_NUMBER = /^\s*[<>=]\s*-?[0-9]+(?:\.[0-9]+)?(?![\p{L}\p{N}])/u;

/** What follows "I" when it is the pronoun. */
const PRONOUN_VERB =
  /^\s+(?:am|have|had|will|would|can|could|shall|should|may|might|must|do|did|think|mean|use|used|write|wrote|call|denote|define|assume|recall|note)\b/;

/** Unicode math, one character at a time. */
const MATH_CHAR = new RegExp(`[${Object.keys(MATH_TEX).join('')}]`, 'gu');

/** Relations, spaced in the TeX the panel typesets. */
const RELATION =
  /\s*(<|>|=|\\(?:leq|geq|neq|in|notin|approx|equiv|sim|subset|subseteq|to|mapsto|propto)(?![A-Za-z]))\s*/g;

/** Text allowed between two pieces of math for them to be one formula:
 * spaces, numbers and operators. */
const MATH_GLUE = /^[\s0-9+\-*/<>=|]*$/;

/** Whether the token at `at` in `text` is math: Greek, a decorated letter
 * (`x_i`, `x^2`, `f'`, `S(C)`), a symbol the document defines, or a Latin
 * letter standing alone as a variable. "a" or "A" followed by a word is the
 * article, and "I" followed by a verb the pronoun. */
function isMathToken(token, text, at, known) {
  if (greekTex(token[0])) return true;
  if (/[_^('*]/.test(token)) return true;
  const after = text.slice(at + token.length);
  if ((token === 'a' || token === 'A') && /^\s+\p{L}{2,}/u.test(after)) return false;
  if (token === 'I' && PRONOUN_VERB.test(after)) return false;
  if (/^[A-Za-z]$/.test(token)) return true;
  return known.has(symbolTex(token));
}

/** A defining phrase for the panel: words as ASCII text, math as `$...$`.
 *
 * Symbols the document defines, single letters used as variables, Greek
 * letters and Unicode math are typeset, and neighbouring math with only
 * spaces, numbers and operators between is one formula: "S(C)< +inf" reads
 * as `$S(C) < +\infty$`. `$` in the words is escaped. */
function phraseText(phrase, known) {
  /** Math pieces, in order: `{ start, end, tex }`. */
  const pieces = [];
  for (const match of phrase.matchAll(PHRASE_SYMBOL)) {
    if (!isMathToken(match[1], phrase, match.index, known)) continue;
    const tex = symbolTex(match[1]);
    if (tex) pieces.push({ start: match.index, end: match.index + match[0].length, tex });
  }
  for (const match of phrase.matchAll(MATH_CHAR)) {
    pieces.push({
      start: match.index,
      end: match.index + match[0].length,
      tex: MATH_TEX[match[0]],
    });
  }
  pieces.sort((a, b) => a.start - b.start);

  // Neighbours joined into formulas, each taking a sign written straight
  // before it and a number compared with it.
  const spans = [];
  for (const piece of pieces) {
    const prev = spans[spans.length - 1];
    const between = prev ? phrase.slice(prev.end, piece.start) : '';
    if (prev && MATH_GLUE.test(between)) {
      prev.parts.push({ raw: between }, { tex: piece.tex });
      prev.end = piece.end;
    } else {
      spans.push({ start: piece.start, end: piece.end, parts: [{ tex: piece.tex }] });
    }
  }
  for (const span of spans) {
    const sign = /(?:^|\s)([-+]|-?[0-9]+(?:\.[0-9]+)?\s*[<>=]\s*)$/.exec(
      phrase.slice(0, span.start)
    );
    if (sign) {
      span.start -= sign[1].length;
      span.parts.unshift({ raw: sign[1] });
    }
    // After a relation already in the formula ("<= 1") the number alone.
    const relation = /^(?:\\(?:leq|geq|neq|approx|equiv|sim|propto|pm|times|cdot))$/.test(
      span.parts[span.parts.length - 1].tex ?? ''
    );
    const number = (relation ? /^\s*-?[0-9]+(?:\.[0-9]+)?(?![\p{L}\p{N}])/u : TRAILING_NUMBER).exec(
      phrase.slice(span.end)
    );
    if (number) {
      span.end += number[0].length;
      span.parts.push({ raw: number[0] });
    }
  }

  let out = '';
  let from = 0;
  for (const span of spans) {
    if (span.start < from) continue;
    out += asciiParts(phrase.slice(from, span.start));
    out += `$${formulaTex(span.parts)}$`;
    from = span.end;
  }
  out += asciiParts(phrase.slice(from));
  return out.replace(/\s+/g, ' ').trim();
}

/** Pieces of one formula - `{ tex }` for the math, `{ raw }` for the
 * operators and numbers joining it - as one TeX string. */
function formulaTex(parts) {
  let out = '';
  parts.forEach((part) => {
    const tex = part.tex ?? texChars(part.raw);
    if (/\\[A-Za-z]+$/.test(out) && /^[A-Za-z]/.test(tex)) out += ' ';
    out += tex;
  });
  return out.replace(RELATION, ' $1 ').replace(/\s+/g, ' ').trim();
}

// ---------- putting it together ----------

/** A phrase reduced to what it says, for comparing two. */
function gist(phrase) {
  return phrase
    .toLowerCase()
    .replace(/\.\.\.$/, '')
    .replace(/[^a-z0-9]+/g, ' ')
    .replace(/\b(?:the|a|an)\b/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Symbols without the second of a pair where one is the other with an
 * argument (`X` and `X(l)`) and both are defined as the same thing. A pair
 * defined differently is two symbols and both are kept. */
function withoutRepeats(defs) {
  const byTex = new Map(defs.map((def) => [def.tex, def]));
  const dropped = new Set();
  for (const def of defs) {
    const base = /^(.+?)\(.*\)$/.exec(def.tex);
    const other = base && byTex.get(base[1]);
    if (!other || gist(other.phrase) !== gist(def.phrase)) continue;
    const otherFirst =
      other.page < def.page || (other.page === def.page && other.index <= def.index);
    dropped.add(otherFirst ? def : other);
  }
  return defs.filter((def) => !dropped.has(def));
}

/** Reads pages one at a time and keeps the first definition of each symbol
 * and acronym, with how often each acronym is used. */
/** Close up or restore the line-break hyphens in `text` (`BREAK`), using
 * the document's own words: a break closes up when the joined word occurs
 * elsewhere unbroken, or when neither half is a word of its own - "dis" and
 * "sipation" are not, so "fluctuation-dis-sipation" reads as
 * "fluctuation-dissipation". Otherwise the hyphen was real: "mean-field". */
function resolveBreaks(text, words) {
  return text
    .replace(/(\p{L}+)\u00ad(\p{L}+)/gu, (_, a, b) => {
      const joined = `${a}${b}`.toLowerCase();
      const neither = !words.has(a.toLowerCase()) && !words.has(b.toLowerCase());
      return words.has(joined) || neither ? `${a}${b}` : `${a}-${b}`;
    })
    .replace(/\u00ad/g, '-');
}

function createExtractor() {
  const symbols = new Map();
  const acronyms = new Map();
  /** Every word the document uses unbroken, for `resolveBreaks`. */
  const words = new Set();
  /** Every distinct meaning of every acronym, first definition of each:
   * what hover previews are made of, where a paper that uses "PS" for two
   * things needs both. */
  const meanings = new Map();
  const counts = new Map();
  return {
    addPage(page, raw) {
      const norm = normalizePage(raw);
      for (const def of findAcronyms(norm, page)) {
        if (!acronyms.has(def.key)) acronyms.set(def.key, def);
        const meaning = `${def.key}\u0000${def.long.toLowerCase()}`;
        if (!meanings.has(meaning)) meanings.set(meaning, def);
      }
      for (const def of findSymbols(norm, page)) {
        if (!symbols.has(def.key)) symbols.set(def.key, def);
      }
      for (const word of norm.text.match(/[\p{L}\u00ad]+/gu) ?? []) {
        if (word.length >= 2 && !word.includes(BREAK)) words.add(word.toLowerCase());
      }
      for (const token of norm.text.match(/[A-Za-z0-9]+/g) ?? []) {
        if (token.length > 11 || !/[A-Z].*[A-Z]/.test(token)) continue;
        counts.set(token, (counts.get(token) ?? 0) + 1);
      }
    },
    result() {
      const strip = ({ index: _index, ...def }) => ({
        ...def,
        ...(typeof def.long === 'string' ? { long: resolveBreaks(def.long, words) } : {}),
        ...(typeof def.phrase === 'string' ? { phrase: resolveBreaks(def.phrase, words) } : {}),
      });
      return {
        symbols: withoutRepeats([...symbols.values()]).map(strip),
        acronyms: [...acronyms.values()]
          .map((def) => ({
            ...strip(def),
            uses: Math.max(0, (counts.get(def.key) ?? 0) + (counts.get(`${def.key}s`) ?? 0) - 1),
          }))
          .sort((a, b) => a.key.localeCompare(b.key)),
        meanings: [...meanings.values()].map(strip),
      };
    },
  };
}
