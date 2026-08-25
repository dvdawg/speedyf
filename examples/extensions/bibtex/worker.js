// Reads the open document through the v2 extension API.
//
// Everything here goes through `call`, and every call is permission-checked
// against this extension's manifest before it runs. Nothing is imported and
// nothing is bundled: the API surface is the only dependency.

let nextRequestId = 1;
const pending = new Map();

self.onmessage = (event) => {
  const message = event.data;
  if (message.type === 'activate') {
    // Nothing to set up: this extension holds no state between invocations.
    return;
  }
  if (message.type === 'invoke') {
    handle(message.command)
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

/** Every citation in the document, deduped across pages. */
async function allCitations() {
  const info = await call('document.info');
  const seen = new Map();
  for (let page = 0; page < info.pageCount; page++) {
    const citations = await call('citations.list', { page });
    for (const citation of citations) {
      const key = `${citation.kind}:${citation.value}`;
      if (!seen.has(key)) seen.set(key, citation);
    }
  }
  return { info, citations: [...seen.values()] };
}

async function handle(command) {
  switch (command) {
    case 'listRefs': {
      const { info, citations } = await allCitations();
      log(
        'info',
        `${info.name}: ${citations.length} distinct citations across ${info.pageCount} pages`
      );
      for (const citation of citations.slice(0, 20)) {
        log('info', `  ${citation.kind}:${citation.value} (first seen p.${citation.page + 1})`);
      }
      return;
    }

    case 'coverage': {
      const { citations } = await allCitations();
      let have = 0;
      for (const citation of citations) {
        const resolved = await call('citations.resolve', citation);
        if (resolved.inLibrary) {
          have++;
          log('info', `have: ${resolved.title ?? resolved.fileName}`);
        } else {
          log('warn', `missing: ${citation.kind}:${citation.value}`);
        }
      }
      log('info', `${have}/${citations.length} cited works are in your library`);
      return;
    }

    case 'outline': {
      const info = await call('document.info');
      const outline = await call('document.outline');
      const figures = await call('document.figures');
      const firstPage = await call('document.pageText', { page: 0 });
      log('info', `${info.name} — ${info.pageCount} pages, on page ${info.currentPage + 1}`);
      log('info', `${count(outline)} outline entries, ${figures.length} figures`);
      log('info', `page 1 opens: ${firstPage.slice(0, 120)}…`);
      return;
    }

    default:
      throw new Error(`unknown command "${command}"`);
  }
}

function count(nodes) {
  return nodes.reduce((total, node) => total + 1 + count(node.children), 0);
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}
