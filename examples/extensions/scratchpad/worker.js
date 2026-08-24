// Collects things you pick into a list you can copy out.
//
// Worth reading for what it does *not* ask for. It has no `document:read`, no
// `citations:read`, no `library:read` — it never reads the document at all.
// Everything it holds arrived because you selected it and chose an action, and
// the extension is handed exactly that and nothing else.
//
// This is the narrow path the permission model is meant to make usable: a
// translate, summarise, or note-taking extension should look like this rather
// than asking for the whole document.

const KEY = 'items';
let items = [];
/** Resolves once stored items are loaded.
 *
 * Activation and the first invoke can arrive in the same tick, and restoring
 * is asynchronous: without this, a keep that lands mid-restore is overwritten
 * by the empty list the restore was still holding. Every handler waits.
 */
let ready = Promise.resolve();
let nextRequestId = 1;
const pending = new Map();

self.onmessage = (event) => {
  const message = event.data;
  if (message.type === 'activate') {
    ready = restore();
    return;
  }
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

async function restore() {
  try {
    const stored = await call('storage.get', { key: KEY });
    items = stored ? JSON.parse(stored) : [];
  } catch {
    items = [];
  }
  await draw();
}

async function persist() {
  await call('storage.set', { key: KEY, value: JSON.stringify(items) });
}

async function handle(command, value) {
  await ready;
  switch (command) {
    case 'keep':
      if (!value) throw new Error('nothing selected');
      items.push({ kind: 'passage', text: value });
      await persist();
      return draw();

    case 'keepCitation':
      if (!value) throw new Error('no citation');
      items.push({ kind: 'citation', text: value });
      await persist();
      return draw();

    case 'copyAll': {
      if (items.length === 0) throw new Error('the scratchpad is empty');
      // Settings are read fresh each time rather than cached at activation, so
      // a change in the Extensions panel takes effect without a restart.
      const [wrap, separator] = await Promise.all([
        call('settings.get', { key: 'wrapCitations' }),
        call('settings.get', { key: 'separator' }),
      ]);
      const joiner =
        separator === 'newline' ? '\n' : separator === 'dash rule' ? '\n\n---\n\n' : '\n\n';
      // Written first, before anything slow. A clipboard write is only
      // permitted while the user action that started this is still in effect.
      const text = items
        .map((item) =>
          item.kind === 'citation' && wrap !== 'false' ? `[${item.text}]` : item.text
        )
        .join(joiner);
      await call('clipboard.write', { text });
      log('info', `copied ${items.length} items`);
      return;
    }

    case 'clear':
      items = [];
      await persist();
      return draw();

    case 'notes':
      return draw();

    default:
      throw new Error(`unknown command "${command}"`);
  }
}

function draw() {
  if (items.length === 0) {
    return call('ui.render', {
      panel: 'notes',
      ui: { empty: 'Select text, or hover a citation, and choose "Keep in Scratchpad".' },
    });
  }
  return call('ui.render', {
    panel: 'notes',
    ui: {
      sections: [
        {
          title: `${items.length} kept`,
          rows: items.map((item, index) => ({
            id: `item-${index}`,
            title: item.kind === 'citation' ? item.text : preview(item.text),
            subtitle: item.kind === 'citation' ? 'citation' : `${item.text.length} characters`,
            badge: item.kind === 'citation' ? 'cite' : undefined,
          })),
        },
      ],
    },
  });
}

function preview(text) {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > 90 ? `${flat.slice(0, 90)}…` : flat;
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}
