// Notepad - one Markdown notepad per project.
//
// SpeedyF keeps the file and draws the editor; this worker is what the
// notepad knows about papers:
//
//   * What `@key` reads as ("[AS22]", alpha.bst style), and what the `@` picker offers
//     (the project's items), told to the editor with `editor.citations`. A
//     key no item has now is looked up in the items' key history, so a cite
//     key changed after a note cited it keeps working.
//   * What a click on a citation does: open the paper, at the page and the
//     place a pinpoint `[@key, p. 12]{box=x,y,w,h}` names.
//   * What the reader's actions write: a quote, an equation (as LaTeX when an
//     equation converter can read it, else as a picture of it; upgraded in
//     place once an exact source has it), a region of a page; each with a
//     link back to its page.
//   * "Mentioned in notepad (3)" beside an item on the Projects screen.
//
// It never touches the network, and needs no document permission at all:
// the selected text, the clicked equation and the drawn rectangle are handed
// to it by SpeedyF, which is the narrow path the extension API is built for.
//
// This file is ASCII on purpose: it carries no character outside it.

/** The notepad's file in each project's folder. */
const FILE = 'notepad.md';
/** Where pictures go, beside it. */
const ASSETS = 'assets';

let nextRequestId = 1;
const pending = new Map();

function call(api, args) {
  const requestId = nextRequestId++;
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    self.postMessage({ type: 'call', requestId, api, args });
  });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}

self.onmessage = (event) => {
  const m = event.data;
  if (!m || typeof m !== 'object') return;
  if (m.type === 'invoke') {
    handle(m.command, m.value)
      .then(() => self.postMessage({ type: 'done', requestId: m.requestId, ok: true }))
      .catch((e) =>
        self.postMessage({
          type: 'done',
          requestId: m.requestId,
          ok: false,
          error: String((e && e.message) || e),
        })
      );
  } else if (m.type === 'reply') {
    const waiting = pending.get(m.requestId);
    if (!waiting) return;
    pending.delete(m.requestId);
    if (m.ok) waiting.resolve(m.value);
    else waiting.reject(new Error(m.error));
  }
};

// ---------- what a citation reads as ----------

/** The base of an item's label: SpeedyF's alpha.bst-style label ("Jar97",
 * "ZD24", "ABC+19"), else the first author and year, else the key. */
function baseLabel(item) {
  const alpha = (item.alphaLabel || '').trim();
  if (alpha) return alpha;
  const who = item.firstAuthor || (item.authors || '').split(/[,&]| et al\./)[0].trim();
  if (who) return item.year ? `${who}${String(item.year).slice(-2)}` : who;
  return item.citeKey;
}

/** 0 -> "a", 25 -> "z", 26 -> "aa". */
function suffix(n) {
  let out = '';
  let k = n;
  for (;;) {
    out = String.fromCharCode(97 + (k % 26)) + out;
    if (k < 26) return out;
    k = Math.floor(k / 26) - 1;
  }
}

/** Labels unique among `items` the way alpha.bst (and SpeedyF's graph)
 * makes them: a label that repeats gets a, b, c in order of year (no year
 * last), then title, then id. Returns id -> "[ZD24a]". */
function alphaLabels(items) {
  const groups = new Map();
  for (const item of items) {
    const base = baseLabel(item);
    if (!groups.has(base)) groups.set(base, []);
    groups.get(base).push(item);
  }
  const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
  const out = new Map();
  for (const [base, group] of groups) {
    if (group.length === 1) {
      out.set(group[0].id, `[${base}]`);
      continue;
    }
    group
      .slice()
      .sort((a, b) => {
        if (a.year !== b.year) {
          if (a.year === null || a.year === undefined) return 1;
          if (b.year === null || b.year === undefined) return -1;
          return a.year - b.year;
        }
        return cmp(a.title || '', b.title || '') || cmp(a.id, b.id);
      })
      .forEach((item, i) => out.set(item.id, `[${base}${suffix(i)}]`));
  }
  return out;
}

/** "Attias, Smith - 2022": the picker's second line. */
function detail(item) {
  return [item.authors, item.year].filter(Boolean).join(' - ');
}

/** The editor's citation entries: every item (so any key resolves), the
 * project's offered in the picker. */
function citationEntries(items, projectId) {
  // The project's items are suffixed among themselves, as the project's
  // graph does, so a paper reads the same in both; any other item a note
  // cites is suffixed among the rest.
  const inProject = items.filter((item) => item.projectIds.includes(projectId));
  const labels = new Map([
    ...alphaLabels(items.filter((item) => !item.projectIds.includes(projectId))),
    ...alphaLabels(inProject),
  ]);
  return items.map((item) => ({
    key: item.citeKey,
    label: labels.get(item.id) || `[${item.citeKey}]`,
    title: item.title || '',
    detail: detail(item),
    pick: item.projectIds.includes(projectId),
    aliases: item.keyHistory || [],
  }));
}

/** The item a key means: the one holding it now, else the first whose
 * history holds it. */
function resolveKey(items, key) {
  return (
    items.find((item) => item.citeKey === key) ||
    items.find((item) => (item.keyHistory || []).includes(key)) ||
    null
  );
}

// ---------- what goes into the note ----------

/** A pinpoint link: `[@key, p. 12]{box=72,310,240,28}`. `page` is
 * zero-based; the box is top-left page points. */
function pageLink(key, page, box) {
  const at = Number.isInteger(page) ? `, p. ${page + 1}` : '';
  const where =
    Array.isArray(box) && box.length === 4 && box.every(Number.isFinite)
      ? `{box=${box.map((v) => Math.round(v)).join(',')}}`
      : '';
  return `[@${key}${at}]${where}`;
}

/** One paragraph of quoted text, made to read as itself in Markdown:
 * whitespace (newlines included) collapses to single spaces, and every
 * character Markdown or a citation would take for markup is escaped. */
function escapeQuote(text) {
  let out = String(text).replace(/\s+/g, ' ').trim();
  out = out.replace(/[\\`*_[\]{}<>$@|~]/g, (c) => `\\${c}`);
  // A leading marker would make a heading, a list or a nested quote.
  out = out.replace(/^([#>+-]|\d+[.)])/, '\\$1');
  return out;
}

/** Characters that end a line as "this word continues below", as SpeedyF's
 * smart copy reads them: the hyphen, U+2010, U+2011 and the soft hyphen. */
const LINE_END_HYPHEN = /[-\u2010\u2011\u00ad]$/;

/** The selection's paragraphs. SpeedyF hands over its smart-copied text
 * (paragraphs apart by a blank line, wrapped lines joined); a selection
 * that still has its line breaks is rejoined here the same way: a line
 * ending in a hyphen before a lower-case letter is one word broken across
 * lines, any other break within a paragraph is a space. */
function quoteParagraphs(text) {
  return String(text)
    .replace(/\r\n?/g, '\n')
    .split(/\n[ \t]*\n\s*/)
    .map((paragraph) => {
      let out = '';
      for (const raw of paragraph.split('\n')) {
        const line = raw.trim();
        if (!line) continue;
        if (LINE_END_HYPHEN.test(out) && /^[a-z]/.test(line)) out = out.slice(0, -1) + line;
        else out = out ? `${out} ${line}` : line;
      }
      return out.replace(/\u00ad/g, '');
    })
    .filter((paragraph) => paragraph.trim() !== '');
}

/** A block quote of the selection, one `> ` line per paragraph and a bare
 * `>` between them, with the link to its page on the last line:
 * `> one ...\n>\n> two ... [@key, p. 12]{box=...}`. Quoted whole, however
 * long. */
function quoteBlock(text, key, page, box) {
  const paragraphs = quoteParagraphs(text).map(escapeQuote);
  const link = pageLink(key, page, box);
  if (paragraphs.length === 0) return `> ${link}`;
  paragraphs[paragraphs.length - 1] += ` ${link}`;
  return paragraphs.map((paragraph) => `> ${paragraph}`).join('\n>\n');
}

/** LaTeX for a `$$ ... $$` block: on one line, which is how a block that
 * stays one block in any Markdown reader is written. */
function displayMath(latex) {
  return `$$ ${String(latex)
    .replace(/\s*\n\s*/g, ' ')
    .trim()} $$`;
}

/** Every citation in a note: its key and where it is. A backslash-escaped
 * `\@` is not one. */
function citationsIn(text) {
  const key = '[A-Za-z0-9_][A-Za-z0-9_:.#$%&+?<>~/-]*[A-Za-z0-9_]|[A-Za-z0-9_]';
  const re = new RegExp(`\\[@(${key})[^\\]]*\\]|(^|[\\s([{;,"'-])@(${key})`, 'g');
  const out = [];
  let m;
  while ((m = re.exec(text)) !== null) {
    if (m[1] !== undefined) {
      if (m.index > 0 && text[m.index - 1] === '\\') continue;
      out.push({ key: m[1], offset: m.index });
    } else {
      out.push({ key: m[3], offset: m.index + m[2].length });
    }
  }
  return out;
}

/** How often a note cites an item, by its key or any key it had, and
 * where first. */
function mentions(text, item) {
  const keys = new Set([item.citeKey, ...(item.keyHistory || [])]);
  const found = citationsIn(text).filter((c) => keys.has(c.key));
  return { count: found.length, first: found.length > 0 ? found[0].offset : null };
}

/** A file name for a picture taken now. */
function assetName(kind) {
  const stamp = new Date()
    .toISOString()
    .replace(/[-:]/g, '')
    .replace(/\.\d+Z$/, '')
    .replace('T', '-');
  const salt = Math.random().toString(36).slice(2, 6);
  return `${ASSETS}/${kind}-${stamp}-${salt}.png`;
}

// ---------- the reader's actions ----------

/** The reader's item and the project to write to: the one it was opened
 * from, its only one, or the one the user picks (once a session). */
async function readerTarget() {
  const context = await call('reader.context');
  if (!context || !context.item || context.projectIds.length === 0) {
    throw new Error('this document is not an item in a project');
  }
  const projectId = context.projectId || (await call('reader.chooseProject'));
  if (!projectId) return null;
  return { context, projectId };
}

/** Hold a place at the end of the project's notepad for something on its
 * way: SpeedyF draws a placeholder of its `pending` shape there (writing
 * nothing to the file), opens the reader's drawer on the note when no view
 * of it is on screen, and here it is scrolled into view. Returns the
 * handle to fill it with. */
async function hold(projectId, pending) {
  const { handle } = await call('editor.insert', { projectId, at: 'end', pending });
  try {
    await call('editor.reveal', { projectId, handle });
  } catch (err) {
    log('info', `reveal: ${String((err && err.message) || err)}`);
  }
  return handle;
}

/** Put `text` where the placeholder is, as one undoable edit. A placeholder
 * that expired meanwhile (a very slow picture) is not lost: the text goes
 * at the end instead. Returns the handle that now tracks the text. */
async function fill(projectId, handle, text) {
  const { resolved } = await call('editor.resolve', { handle, text });
  if (resolved) return handle;
  const inserted = await call('editor.insert', { projectId, text, at: 'end' });
  return inserted.handle;
}

/** Give up a placeholder: the send failed. */
async function drop(handle) {
  try {
    await call('editor.cancel', { handle });
  } catch {
    /* it expires on its own */
  }
}

async function quote(text) {
  if (typeof text !== 'string' || !text.trim()) return;
  const target = await readerTarget();
  if (!target) return;
  const { context, projectId } = target;
  const selection = context.selection;
  const page = selection ? selection.page : context.page;
  const box = selection ? selection.box : null;
  // Filled at once: the placeholder is drawn only if this ever takes long.
  const handle = await hold(projectId, { kind: 'text' });
  await fill(projectId, handle, quoteBlock(text, context.item.citeKey, page, box));
}

/** `equations.convert` in one mode; null when there is no answer. In exact
 * mode `status` is the handle whose status line the converter names its
 * source in. */
async function convert(equation, mode, status) {
  try {
    const answer = await call('equations.convert', {
      equation,
      mode,
      ...(status ? { status } : {}),
    });
    return answer && typeof answer.latex === 'string' && answer.latex.trim() ? answer : null;
  } catch (err) {
    log('info', `no ${mode} LaTeX for this equation: ${String((err && err.message) || err)}`);
    return null;
  }
}

/** Stage times of one send, logged at info as one line. */
function stopwatch(label) {
  const started = Date.now();
  let last = started;
  const parts = [];
  return {
    lap(name) {
      const now = Date.now();
      parts.push(`${name} ${now - last}ms`);
      last = now;
    },
    log(outcome) {
      log('info', `${label}: ${parts.join(', ')}; ${outcome} in ${Date.now() - started}ms`);
    },
  };
}

/** Send an equation: a placeholder with its caption at once, filled from
 * what the page itself says (its tags or its fonts, else a picture of it),
 * then upgraded quietly to the author's own LaTeX when an exact source
 * online has it, with a status line under it meanwhile. */
async function sendEquation(value) {
  if (!value || typeof value !== 'object' || typeof value.equation !== 'string') {
    log('info', 'Send to Notepad works from the button on an equation.');
    return;
  }
  const clock = stopwatch('send equation');
  const target = await readerTarget();
  clock.lap('project');
  if (!target) return;
  const { context, projectId } = target;
  const name = value.number ? `Equation (${value.number})` : 'Equation';
  const caption = `${name}, ${pageLink(context.item.citeKey, value.page, value.bbox)}`;
  const placeholder = await hold(projectId, { kind: 'equation', caption });
  clock.lap('placeholder');
  let block;
  let asset = null;
  let local = null;
  try {
    local = await convert(value.equation, 'local');
    clock.lap('convert');
    if (local) {
      block = `${displayMath(local.latex)}\n\n${caption}`;
    } else {
      const shot = await call('equations.snapshot', { equation: value.equation });
      asset = assetName('equation');
      await call('projectFiles.write', { projectId, path: asset, bytes: shot.png });
      clock.lap('picture');
      block = `![${name}](${asset})\n\n${caption}`;
    }
  } catch (err) {
    await drop(placeholder);
    throw err;
  }
  const handle = await fill(projectId, placeholder, block);
  clock.lap('insert');
  clock.log(local ? `LaTeX from ${local.source || 'the converter'}` : 'picture');
  // The document's own tags are the author's LaTeX already.
  if (local && local.source === 'tags' && local.confidence === 'high') {
    await call('editor.unmark', { handle });
    return;
  }
  // Not awaited: the send is done; this only ever improves it.
  void upgrade({ equation: value.equation, handle, block, caption, asset, projectId });
}

/** Swap an equation just sent for an exact source's LaTeX, when there is
 * one and it differs, and only while the inserted text is still there as
 * it was: an equation the user has edited is theirs. A dim "checking ...
 * source" line sits under it meanwhile. */
async function upgrade({ equation, handle, block, caption, asset, projectId }) {
  const started = Date.now();
  const done = (outcome) => log('info', `upgrade: ${outcome} in ${Date.now() - started}ms`);
  const clear = () => call('editor.unmark', { handle }).catch(() => {});
  let marked = false;
  try {
    ({ marked } = await call('editor.mark', { handle, label: 'checking source' }));
  } catch (err) {
    log('info', `no status line: ${String((err && err.message) || err)}`);
  }
  if (!marked) return done('edited meanwhile; left alone');
  const exact = await convert(equation, 'exact', handle);
  if (!exact) {
    await clear();
    return done('no exact source');
  }
  const better = `${displayMath(exact.latex)}\n\n${caption}`;
  if (better === block) {
    await clear();
    return done('already exact');
  }
  let replaced = false;
  try {
    ({ replaced } = await call('editor.replaceMarked', { handle, text: better }));
  } catch (err) {
    await clear();
    return done(`not replaced: ${String((err && err.message) || err)}`);
  }
  if (!replaced) return done('edited meanwhile; left alone');
  if (asset) {
    try {
      await call('projectFiles.remove', { projectId, path: asset });
    } catch (err) {
      log('info', `picture kept: ${String((err && err.message) || err)}`);
    }
  }
  done(`replaced with LaTeX from ${exact.source || 'an exact source'}`);
}

async function sendRegion() {
  const target = await readerTarget();
  if (!target) return;
  const { context, projectId } = target;
  const region = await call('region.capture', {
    prompt: 'Notepad: drag a rectangle around a figure. Escape cancels.',
  });
  if (!region) return;
  const [, , w, h] = Array.isArray(region.box) ? region.box : [];
  const shape = w > 0 && h > 0 ? { w, h } : null;
  const link = pageLink(context.item.citeKey, region.page, region.box);
  const placeholder = await hold(projectId, { kind: 'image', shape, caption: link });
  const path = assetName('region');
  try {
    await call('projectFiles.write', { projectId, path, bytes: region.png });
  } catch (err) {
    await drop(placeholder);
    throw err;
  }
  const handle = await fill(
    projectId,
    placeholder,
    `![Region, p. ${region.page + 1}](${path})\n\n${link}`
  );
  await call('editor.unmark', { handle });
}

/** Open Notepad: the reader's drawer for a paper in a project, else the
 * Projects screen's view of the selected project. */
async function open() {
  const context = await call('reader.context');
  if (context && context.item && context.projectIds.length > 0) {
    await call('pane.open', { pane: 'notepad' });
    return;
  }
  const { opened } = await call('projectView.open', { view: 'notepad' });
  if (!opened) log('info', 'Select a project on the Projects screen to open its notepad.');
}

// ---------- the editor and the Projects screen ----------

async function editorEvent(value) {
  if (!value || typeof value !== 'object') return;
  const items = await call('projects.items');
  if (value.event === 'citations') {
    await call('editor.citations', {
      projectId: value.projectId,
      entries: citationEntries(items, value.projectId),
    });
    return;
  }
  if (value.event === 'activate' && value.cite) {
    const item = resolveKey(items, value.cite.key);
    if (!item) {
      log('info', `no item has the key ${value.cite.key}`);
      return;
    }
    await call('navigate.openItem', {
      itemId: item.id,
      projectId: value.projectId,
      page: Number.isInteger(value.cite.page) ? value.cite.page : null,
      box: value.cite.box || null,
    });
  }
}

/** "Mentioned in notepad (3)": in the selected project's notepad, or under
 * another scope in any of the item's projects'. */
async function itemLine(value) {
  if (!value || !value.item) return;
  const item = value.item;
  const projects = value.projectId ? [value.projectId] : item.projectIds;
  let count = 0;
  let first = null;
  for (const projectId of projects) {
    let text = null;
    try {
      text = await call('projectFiles.read', { projectId, path: FILE });
    } catch {
      text = null;
    }
    if (typeof text !== 'string') continue;
    const found = mentions(text, item);
    count += found.count;
    if (!first && found.first !== null) first = { projectId, offset: found.first };
  }
  await call('ui.itemLine', {
    itemId: item.id,
    text: count > 0 ? `Mentioned in notepad (${count})` : null,
    action: first ? { command: 'jump', value: first } : null,
  });
}

async function jump(value) {
  if (!value || typeof value.projectId !== 'string') return;
  await call('projectView.open', {
    view: 'notepad',
    projectId: value.projectId,
    offset: Number.isInteger(value.offset) ? value.offset : 0,
  });
}

async function handle(command, value) {
  switch (command) {
    case 'open':
      return open();
    case 'quote':
      return quote(value);
    case 'equation':
      return sendEquation(value);
    case 'region':
      return sendRegion();
    case 'editor':
      return editorEvent(value);
    case 'mentions':
      return itemLine(value);
    case 'jump':
      return jump(value);
  }
  throw new Error(`unknown command "${command}"`);
}
