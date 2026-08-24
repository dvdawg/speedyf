// Vim-style navigation for SpeedyF, built on the v2 extension API.
//
// This is the extension the v1 plan originally named and could not support:
// it needs both `contributes.keybindings` (to claim j, k, g, and so on) and
// `commands.run` (to drive SpeedyF once a key fires). Neither existed in v1.
//
// Note the shape. Keys are bound to *this extension's* commands, never
// directly to SpeedyF's. Every effect on the app goes through `commands.run`,
// which means every one of them passes the permission gate — a manifest entry
// alone can never move the document.

let me = null;
let nextRequestId = 1;
const pendingCalls = new Map();

self.onmessage = (event) => {
  const message = event.data;

  if (message.type === 'activate') {
    me = message;
    if (!me.permissions.includes('commands:run')) {
      log('error', 'without commands:run this extension can bind keys but do nothing');
    }
    return;
  }

  if (message.type === 'invoke') {
    handle(message.command)
      .then(() => done(message.requestId, true))
      .catch((err) => done(message.requestId, false, String(err.message ?? err)));
    return;
  }

  if (message.type === 'reply') {
    const pending = pendingCalls.get(message.requestId);
    if (!pending) return;
    pendingCalls.delete(message.requestId);
    if (message.ok) pending.resolve(message.value);
    else pending.reject(new Error(message.error));
  }
};

/** Ask SpeedyF to run one of its own commands. */
function run(id) {
  return call('commands.run', { id });
}

function call(api, args) {
  const requestId = nextRequestId++;
  return new Promise((resolve, reject) => {
    pendingCalls.set(requestId, { resolve, reject });
    self.postMessage({ type: 'call', requestId, api, args });
  });
}

async function handle(command) {
  switch (command) {
    case 'next':
      return run('nav.nextPage');
    case 'prev':
      return run('nav.previousPage');
    case 'zoomIn':
      return run('view.zoomIn');
    case 'zoomOut':
      return run('view.zoomOut');
    case 'fit':
      return run('view.fitWidth');
    case 'outline':
      return run('panel.outline');
    case 'search':
      return run('search.findInDocument');

    case 'forbidden': {
      // Deliberate demonstration: undo is in the Edit group, which extensions
      // cannot reach however they ask. The refusal arrives as an ordinary
      // rejected call, not a crash.
      try {
        await run('edit.undo');
        log('error', 'edit.undo was allowed — the permission gate is not working');
      } catch (err) {
        log('info', `refused as expected: ${err.message}`);
      }
      return;
    }

    case 'readDoc': {
      // This extension only asked for `commands:run`. Reaching for a read API
      // is refused by permission, not by the command-group rule above — two
      // separate gates, both closed by default.
      try {
        await call('document.info');
        log('error', 'document.info was allowed — the permission gate is not working');
      } catch (err) {
        log('info', `refused as expected: ${err.message}`);
      }
      return;
    }

    default:
      throw new Error(`unknown command "${command}"`);
  }
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}
