/** The reference provider: where library documents' reference lists are read.
 *
 * Cutting a bibliography into entries is parsing, and parsing belongs in an
 * extension on the permissioned API rather than in the engine. So the engine
 * keeps the identifiers and the matching, and an extension that declares
 * `referenceProvider` (the built-in References, in practice) reads the lists:
 *
 *   1. this queue asks the engine for documents its provider has not read
 *      (`library_refs_pending`) — an opaque token and the stored text each,
 *      never a path;
 *   2. invokes the provider's command with `{ document, text }`, one document
 *      at a time, at idle, with a 20 s timeout of its own;
 *   3. the extension answers with `references.contribute { document, entries }`
 *      before it reports done, and the entries go back to the engine with the
 *      token (`library_refs_store`), which writes the refs sidecar.
 *
 * Only one provider is active: the first enabled extension that declares one.
 * With none, the engine reads no refs sidecars at all, so title matching is
 * off and only DOIs, arXiv ids and APS journal references link papers.
 *
 * It lives here rather than in the engine because this is where the workers
 * live. Pure apart from what `deps` does, so the whole loop is testable.
 */
import { readReferenceEntries, type ApiReferenceEntry } from './api';
import { REFERENCE_PROVIDER_TIMEOUT_MS, type ReferenceRequest } from './protocol';

export const CONTRIBUTE_REFERENCES = 'library:contributeReferences';

/** The extension reading reference lists, and how to reach it. */
export interface ProviderChoice {
  extensionId: string;
  version: string;
  /** Its bare command id. */
  command: string;
}

/** The provider as the engine knows it: `<extension id>@<version>`, so a new
 * version of the extension reads every document again. */
export function providerTag(choice: ProviderChoice | null): string | null {
  return choice ? `${choice.extensionId}@${choice.version}` : null;
}

/** The active provider: the first enabled, working extension that declares
 * `referenceProvider` and holds the permission for it. */
export function chooseProvider(
  extensions: readonly {
    id: string;
    version: string;
    enabled: boolean;
    error: string | null;
    permissions: readonly string[];
    referenceProvider: string | null;
  }[]
): ProviderChoice | null {
  const found = extensions.find(
    (e) =>
      e.enabled &&
      !e.error &&
      e.referenceProvider !== null &&
      e.permissions.includes(CONTRIBUTE_REFERENCES)
  );
  return found
    ? { extensionId: found.id, version: found.version, command: found.referenceProvider! }
    : null;
}

export interface PendingDocument {
  token: string;
  text: string;
}

export interface ReferenceQueueDeps {
  /** Tell the engine which provider is active (`library_refs_provider`). */
  setProvider(tag: string | null): Promise<void>;
  /** `library_refs_pending`. */
  pending(limit: number): Promise<PendingDocument[]>;
  /** `library_refs_store`; `null` entries mean the provider failed on it. */
  store(token: string, entries: ApiReferenceEntry[] | null): Promise<unknown>;
  /** Invoke the provider's command and wait for its `done`. */
  invoke(choice: ProviderChoice, value: ReferenceRequest, timeoutMs: number): Promise<void>;
  /** Whether the reader is in use right now; the queue waits it out. */
  busy(): boolean;
  /** Resolve after about `ms`, when the window is idle. */
  wait(ms: number): Promise<void>;
  log(extensionId: string, level: 'info' | 'warn' | 'error', message: string): void;
}

/** Documents asked for at once. */
const BATCH = 8;
/** Between two documents: enough that a library of hundreds is read over
 * minutes, in gaps, rather than in one burst. */
export const SPACING_MS = 400;
/** With nothing pending, look again this often: the text pass indexes new
 * documents in the background. */
export const RECHECK_MS = 60_000;

interface Inflight {
  extensionId: string;
  token: string;
  entries: ApiReferenceEntry[] | null;
}

export class ReferenceQueue {
  private choice: ProviderChoice | null = null;
  /** Bumped whenever the provider changes, so a loop started for the old one
   * stops at its next step. */
  private generation = 0;
  private running = false;
  private inflight: Inflight | null = null;
  /** Ends the wait for a recheck early, when the provider changes. */
  private wake: (() => void) | null = null;

  constructor(private readonly deps: ReferenceQueueDeps) {}

  get provider(): ProviderChoice | null {
    return this.choice;
  }

  /** Called whenever the extensions change: tells the engine, and starts
   * reading when there is a provider. */
  async setProvider(choice: ProviderChoice | null): Promise<void> {
    const changed = providerTag(choice) !== providerTag(this.choice);
    this.choice = choice;
    if (changed) {
      this.generation++;
      this.inflight = null;
      try {
        await this.deps.setProvider(providerTag(choice));
      } catch (err) {
        if (choice)
          this.deps.log(choice.extensionId, 'warn', `reference provider: ${describe(err)}`);
      }
    }
    this.wake?.();
    if (choice && !this.running) void this.run();
  }

  /** `references.contribute`, from an extension already allowed to call it:
   * accepted only from the provider, for the document it is reading now. */
  contribute(extensionId: string, args: unknown): { stored: number } {
    const { document, entries } = readReferenceEntries(args);
    const inflight = this.inflight;
    if (!inflight || inflight.extensionId !== extensionId || inflight.token !== document) {
      throw new Error(
        'references.contribute answers a library document SpeedyF handed this extension, while it is reading it'
      );
    }
    inflight.entries = entries;
    return { stored: entries.length };
  }

  private async run(): Promise<void> {
    this.running = true;
    try {
      while (this.choice) {
        const generation = this.generation;
        const choice = this.choice;
        if (!(await this.idle(generation))) continue;
        let batch: PendingDocument[];
        try {
          batch = await this.deps.pending(BATCH);
        } catch (err) {
          this.deps.log(choice.extensionId, 'warn', `reference provider: ${describe(err)}`);
          batch = [];
        }
        if (generation !== this.generation) continue;
        if (batch.length === 0) {
          await Promise.race([
            this.deps.wait(RECHECK_MS),
            new Promise<void>((resolve) => {
              this.wake = resolve;
            }),
          ]);
          this.wake = null;
          continue;
        }
        for (const document of batch) {
          if (!(await this.idle(generation))) break;
          await this.read(choice, document);
        }
      }
    } finally {
      this.running = false;
    }
  }

  /** Wait for a quiet moment; false when the provider changed meanwhile. */
  private async idle(generation: number): Promise<boolean> {
    do {
      await this.deps.wait(SPACING_MS);
      if (generation !== this.generation) return false;
    } while (this.deps.busy());
    return true;
  }

  /** One document: invoke, collect what the provider contributed, store it.
   * A provider that finishes without contributing found nothing; one that
   * fails or times out is reported, and the engine does not offer that
   * document again this session. */
  private async read(choice: ProviderChoice, document: PendingDocument): Promise<void> {
    const inflight: Inflight = {
      extensionId: choice.extensionId,
      token: document.token,
      entries: null,
    };
    this.inflight = inflight;
    let ok = false;
    try {
      await this.deps.invoke(
        choice,
        { document: document.token, text: document.text },
        REFERENCE_PROVIDER_TIMEOUT_MS
      );
      ok = true;
    } catch (err) {
      this.deps.log(
        choice.extensionId,
        'warn',
        `reading a library document's references: ${describe(err)}`
      );
    }
    if (this.inflight === inflight) this.inflight = null;
    const entries: ApiReferenceEntry[] | null = inflight.entries;
    try {
      await this.deps.store(document.token, ok ? (entries ?? []) : null);
    } catch {
      // The document changed, or the provider did, while it was read: the
      // engine offers it again when it is current.
    }
  }
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** When the reader was last touched, for `readerBusy`. */
let lastInput = 0;
let tracking = false;

/** Whether someone is using the window: any input in the last two seconds.
 * The provider then waits, so reading a library never competes with reading
 * a paper. */
export function readerBusy(now = Date.now()): boolean {
  if (!tracking && typeof window !== 'undefined') {
    tracking = true;
    const mark = () => {
      lastInput = Date.now();
    };
    for (const type of ['pointerdown', 'wheel', 'keydown', 'scroll']) {
      window.addEventListener(type, mark, { capture: true, passive: true });
    }
  }
  return now - lastInput < 2_000;
}

/** `ms`, then the next idle callback where the platform has one. */
export function idleWait(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(() => {
      const idle = (
        globalThis as { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => void }
      ).requestIdleCallback;
      if (idle) idle(() => resolve(), { timeout: 2_000 });
      else resolve();
    }, ms);
  });
}
