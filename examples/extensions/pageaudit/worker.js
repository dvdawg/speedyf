// A document task: batch work over every page.
//
// This is the shape a real OCR extension takes. It asks for a raster of each
// page, does something with the pixels, and contributes a text layer back for
// pages that have none. What it "recognises" here is deliberately trivial — the
// point is the flow, not the recognition.
//
// Note what is NOT happening: nothing in SpeedyF waits for this. It can take a
// minute over a long document and the reader stays responsive, because rasters
// are rendered at idle priority behind whatever the user is actually looking at.

let nextRequestId = 1;
const pending = new Map();
let report = null;
let running = false;

self.onmessage = (event) => {
  const message = event.data;
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

async function handle(command) {
  if (command === 'audit') return draw();
  if (command !== 'run') throw new Error(`unknown command "${command}"`);
  if (running) throw new Error('a scan is already in progress');

  running = true;
  try {
    const info = await call('document.info');
    const pages = [];
    let contributed = 0;

    for (let page = 0; page < info.pageCount; page++) {
      // Rendered small on purpose: this only measures the page, and a full-size
      // raster of every page would be a lot of bytes across the wire for
      // nothing. A real OCR pass would ask for ~2000 and take the cost.
      const bytes = await call('render.page', { page, scaleMilli: 600 });
      const text = await call('document.pageText', { page });
      const hasText = text.trim().length > 20;

      if (!hasText) {
        // The page has no extractable text. A real extension would recognise
        // the raster here; this records what it would have worked from, which
        // is enough to prove the contribution path end to end.
        await call('document.contributeText', {
          page,
          text: `[page audit] no embedded text; raster was ${bytes.length} bytes`,
        });
        contributed++;
      }
      pages.push({ page, bytes: bytes.length, hasText });
      // Yield between pages so a long document does not monopolise the worker.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }

    const stored = await call('document.contributedText');
    report = { info, pages, contributed, storedPages: stored ? stored.pages.length : 0 };
    await draw();
  } finally {
    running = false;
  }
}

function draw() {
  if (!report) {
    return call('ui.render', {
      panel: 'audit',
      ui: { empty: 'Run "Page Audit: Scan Document" to walk every page.' },
    });
  }
  const withText = report.pages.filter((p) => p.hasText).length;
  return call('ui.render', {
    panel: 'audit',
    ui: {
      sections: [
        {
          title: `${report.info.name} — ${report.info.pageCount} pages`,
          rows: [
            { id: 'text', title: 'Pages with embedded text', subtitle: String(withText) },
            {
              id: 'none',
              title: 'Pages without',
              subtitle: String(report.pages.length - withText),
              badge: report.contributed > 0 ? `${report.contributed} contributed` : undefined,
            },
            {
              id: 'stored',
              title: 'Pages in the sidecar',
              subtitle: String(report.storedPages),
            },
          ],
        },
      ],
    },
  });
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}
