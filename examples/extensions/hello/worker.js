// Reference extension for the SpeedyF v1 extension API.
//
// v1 gives an extension exactly one thing: commands in the command palette.
// There is no host API yet, so this file demonstrates the whole surface —
// activation, invocation, reporting success, reporting failure, and what
// happens when an extension misbehaves.
//
// It is deliberately dependency-free and single-file. Worker source is loaded
// through a blob URL (see engine `extensions` for why), so relative
// `importScripts` will not resolve: an extension ships a bundle.

let me = null;

self.onmessage = (event) => {
  const message = event.data;

  if (message.type === 'activate') {
    me = message;
    log('info', `activated as ${me.id} against API v${me.apiVersion}`);
    if (me.permissions.length === 0) {
      log('info', 'no permissions granted, which is all v1 offers');
    }
    return;
  }

  if (message.type === 'invoke') {
    run(message.command, message.requestId);
    return;
  }

  if (message.type === 'reply') {
    // Answer to a `call` we made. See the `overreach` command below.
    log(message.ok ? 'info' : 'warn', `host replied: ${message.error ?? 'ok'}`);
  }
};

function run(command, requestId) {
  switch (command) {
    case 'greet':
      log('info', `hello from ${me?.id ?? 'an unactivated worker'}`);
      done(requestId, true);
      return;

    case 'fail':
      // A command that reports failure. The host records it against this
      // extension and shows it in the Extensions panel.
      done(requestId, false, 'this command fails on purpose');
      return;

    case 'hang':
      // Never answers. The host times out and reports it; SpeedyF itself keeps
      // running, which is the property the whole worker design exists for.
      log('warn', 'this command will never answer — the host should time out');
      return;

    case 'overreach':
      // v1 grants no APIs, so the gate refuses this. The point is that the
      // refusal is an ordinary reply rather than a crash.
      self.postMessage({ type: 'call', requestId: 9001, api: 'library.search', args: { q: 'x' } });
      done(requestId, true);
      return;

    default:
      done(requestId, false, `unknown command "${command}"`);
  }
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}
