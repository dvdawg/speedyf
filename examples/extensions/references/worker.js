// A sidebar panel built entirely out of the declarative UI vocabulary.
//
// The extension never renders anything. It sends a description — sections,
// rows, badges — and SpeedyF draws it with its own components. That is why this
// panel matches the built-in ones, follows dark mode, and inherits the inverted
// page palette without a line of CSS here.

let nextRequestId = 1;
const pending = new Map();

self.onmessage = (event) => {
  const message = event.data;
  if (message.type === 'invoke') {
    render()
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

async function render() {
  let info;
  try {
    info = await call('document.info');
  } catch {
    // No document open is an ordinary state, not a failure worth an error box.
    return draw({ empty: 'Open a paper to see what it cites.' });
  }

  const seen = new Map();
  for (let page = 0; page < info.pageCount; page++) {
    for (const citation of await call('citations.list', { page })) {
      const key = `${citation.kind}:${citation.value}`;
      const existing = seen.get(key);
      if (existing) existing.count++;
      else seen.set(key, { ...citation, count: 1 });
    }
  }

  const citations = [...seen.values()];
  if (citations.length === 0) {
    return draw({
      empty: `No linked citations in ${info.name}. Older papers often have none.`,
    });
  }

  const have = [];
  const missing = [];
  for (const citation of citations) {
    const resolved = await call('citations.resolve', citation);
    const row = {
      id: `${citation.kind}:${citation.value}`,
      title: resolved.title ?? resolved.fileName ?? citation.value,
      subtitle: `${citation.kind} · cited ${citation.count}× · first on p.${citation.page + 1}`,
      action: { command: 'refs', value: String(citation.page) },
    };
    if (resolved.inLibrary) have.push({ ...row, badge: 'in library' });
    else missing.push(row);
  }

  draw({
    sections: [
      { title: `In your library (${have.length})`, rows: have },
      { title: `Not in your library (${missing.length})`, rows: missing },
    ],
  });
}

function draw(ui) {
  return call('ui.render', { panel: 'refs', ui });
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}
