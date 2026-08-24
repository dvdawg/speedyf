/** The wire between SpeedyF and an extension worker.
 *
 * Every exchange is asynchronous request/response carrying a correlation id,
 * from the first release. That is not future-proofing for its own sake: a
 * fire-and-forget or synchronous protocol cannot be made async later without
 * rewriting every extension already published against it, and the shape of the
 * protocol is the one part of an extension API that is genuinely impossible to
 * revise quietly.
 *
 * Nothing here imports from the app. The host owns policy; this file owns only
 * the vocabulary, so it can be tested against a fake worker.
 */

/** Extension ids and command ids are namespaced `<extension>/<command>`. */
export type QualifiedCommandId = string;

// ---------- host → extension ----------

/** Sent once, before anything else, so the worker knows who it is. */
export interface ActivateMessage {
  type: 'activate';
  id: string;
  apiVersion: number;
  /** Exactly what the user granted. An extension is told its own limits so it
   * can degrade deliberately rather than discovering them through failures. */
  permissions: readonly string[];
}

/** Run one of the extension's contributed commands. */
export interface InvokeMessage {
  type: 'invoke';
  requestId: number;
  /** The bare id the extension declared, not the namespaced one. */
  command: string;
  /** Carried back from a row action, so one command can serve many rows. */
  value?: string;
  /** Set when the invoke came from a panel becoming visible, so an extension
   * can tell "the user opened my panel" from "the user ran my command". */
  reason?: 'command' | 'panel';
}

/** The host's answer to a `call` from the extension. */
export interface ReplyMessage {
  type: 'reply';
  requestId: number;
  ok: boolean;
  value?: unknown;
  error?: string;
}

export type HostMessage = ActivateMessage | InvokeMessage | ReplyMessage;

// ---------- extension → host ----------

/** The extension asks the host to do something. Permission-checked by name. */
export interface CallMessage {
  type: 'call';
  requestId: number;
  api: string;
  args?: unknown;
}

/** The extension answers an `invoke`. */
export interface DoneMessage {
  type: 'done';
  requestId: number;
  ok: boolean;
  error?: string;
}

/** Diagnostics, surfaced in the Extensions panel rather than the console. */
export interface LogMessage {
  type: 'log';
  level: 'info' | 'warn' | 'error';
  message: string;
}

export type ExtensionMessage = CallMessage | DoneMessage | LogMessage;

/** Anything an extension sends is untrusted input, so it is validated
 * structurally rather than cast. A malformed message is dropped and reported,
 * never allowed to reach the host as a half-typed object. */
export function parseExtensionMessage(data: unknown): ExtensionMessage | null {
  if (typeof data !== 'object' || data === null) return null;
  const msg = data as Record<string, unknown>;
  switch (msg.type) {
    case 'call':
      return typeof msg.requestId === 'number' && typeof msg.api === 'string'
        ? { type: 'call', requestId: msg.requestId, api: msg.api, args: msg.args }
        : null;
    case 'done':
      return typeof msg.requestId === 'number' && typeof msg.ok === 'boolean'
        ? {
            type: 'done',
            requestId: msg.requestId,
            ok: msg.ok,
            ...(typeof msg.error === 'string' ? { error: msg.error } : {}),
          }
        : null;
    case 'log':
      return typeof msg.message === 'string' &&
        (msg.level === 'info' || msg.level === 'warn' || msg.level === 'error')
        ? { type: 'log', level: msg.level, message: msg.message }
        : null;
    default:
      return null;
  }
}

/** How long an extension gets to answer an `invoke` before the host gives up
 * on it. The extension is not killed — a slow command is not proof of a broken
 * extension — but the host stops waiting, so nothing in the UI can hang on
 * third-party code. */
export const INVOKE_TIMEOUT_MS = 5_000;

/** How long the host waits for a worker to acknowledge activation before
 * treating it as failed to start. */
export const ACTIVATE_TIMEOUT_MS = 5_000;
