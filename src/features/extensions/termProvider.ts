/** The term provider: terms the open document defines, contributed by an
 * extension and previewed by SpeedyF on hover.
 *
 * Knowing what a document defines is reading, and reading belongs in an
 * extension (Notation, in practice, with its acronym extraction). Matching
 * those terms against the page text belongs to the core, which alone knows
 * where each character sits, which are mathematics and where the
 * bibliography is. So:
 *
 *   1. when a document is on screen, the first enabled extension declaring
 *      `termProvider` (and holding `document:contributeTerms`) is invoked
 *      with `{ document }`, an opaque token;
 *   2. it answers - then or later - with `terms.contribute { document,
 *      terms }`, each term `{ text, caseSensitive?, kind?, title?, body,
 *      page, box? }` on the reader's pages, `box` an `ApiBox`;
 *   3. the core keeps the terms for that document only while the token is
 *      the current one: switching documents in between voids it, because
 *      the extension's reads went to whatever document was open.
 *
 * Disabling the provider takes every term back. Pure apart from `deps`. */
import type { ApiBox } from './api';
import type { EngineTerm } from '../hovers/hoverStore';

export const CONTRIBUTE_TERMS = 'document:contributeTerms';

/** No document defines more; a provider claiming more is misreading. */
export const MAX_TERMS = 2000;
const MAX_TERM_TEXT = 64;
const MAX_TITLE = 128;
const MAX_BODY = 600;

export interface TermProviderChoice {
  extensionId: string;
  version: string;
  /** Its bare command id. */
  command: string;
}

export function termProviderTag(choice: TermProviderChoice | null): string | null {
  return choice ? `${choice.extensionId}@${choice.version}` : null;
}

/** The active provider: the first enabled, working extension that declares
 * `termProvider` and holds the permission for it. */
export function chooseTermProvider(
  extensions: readonly {
    id: string;
    version: string;
    enabled: boolean;
    error: string | null;
    permissions: readonly string[];
    termProvider: string | null;
  }[]
): TermProviderChoice | null {
  const found = extensions.find(
    (e) =>
      e.enabled && !e.error && e.termProvider !== null && e.permissions.includes(CONTRIBUTE_TERMS)
  );
  return found
    ? { extensionId: found.id, version: found.version, command: found.termProvider! }
    : null;
}

/** A term as an extension sends it, pages as the reader sees them. */
export interface ApiTerm {
  text: string;
  caseSensitive: boolean;
  kind: string | null;
  title: string;
  body: string;
  page: number;
  box: ApiBox | null;
}

function finiteBox(value: unknown): ApiBox | null {
  if (typeof value !== 'object' || value === null) return null;
  const box = value as Record<string, unknown>;
  const sides = ['x', 'y', 'w', 'h'].map((key) => box[key]);
  if (!sides.every((side) => typeof side === 'number' && Number.isFinite(side))) return null;
  const [x, y, w, h] = sides as number[];
  if (w! < 0 || h! < 0) return null;
  return { x: x!, y: y!, w: w!, h: h! };
}

/** Read `terms.contribute` arguments. A malformed term is dropped on its own
 * rather than costing the provider the whole list: a term needs a `text` of
 * two to 64 characters with no line break, a `body`, and a `page` inside the
 * document. Past 2000 terms the rest are ignored. */
export function readTermArgs(
  args: unknown,
  pageCount: number
): { document: string; terms: ApiTerm[] } {
  const { document, terms } = (args ?? {}) as { document?: unknown; terms?: unknown };
  if (typeof document !== 'string' || document.length === 0) {
    throw new Error('terms.contribute needs the "document" it was handed');
  }
  if (!Array.isArray(terms)) throw new Error('terms.contribute needs a "terms" array');
  const out: ApiTerm[] = [];
  for (const raw of terms) {
    if (out.length >= MAX_TERMS) break;
    if (typeof raw !== 'object' || raw === null) continue;
    const t = raw as Record<string, unknown>;
    if (typeof t.text !== 'string') continue;
    const text = t.text;
    const length = [...text].length;
    if (length < 2 || length > MAX_TERM_TEXT || text.trim() !== text || /[\r\n]/.test(text)) {
      continue;
    }
    if (typeof t.body !== 'string' || t.body.trim().length === 0) continue;
    if (typeof t.page !== 'number' || !Number.isInteger(t.page)) continue;
    if (t.page < 0 || t.page >= pageCount) continue;
    out.push({
      text,
      caseSensitive: t.caseSensitive !== false,
      kind: typeof t.kind === 'string' ? t.kind.slice(0, 32) : null,
      title: typeof t.title === 'string' && t.title.trim() ? t.title.slice(0, MAX_TITLE) : text,
      body: t.body.trim().slice(0, MAX_BODY),
      page: t.page,
      box: finiteBox(t.box),
    });
  }
  return { document, terms: out };
}

/** Terms on the reader's pages, as the engine takes them: pages in the file,
 * boxes as y-up `[x, bottom, w, h]`. A term on a page that has no page in
 * the file is dropped. */
export function toEngineTerms(
  terms: readonly ApiTerm[],
  srcOf: (page: number) => number | null
): EngineTerm[] {
  const out: EngineTerm[] = [];
  for (const t of terms) {
    const src = srcOf(t.page);
    if (src === null) continue;
    out.push({
      text: t.text,
      caseSensitive: t.caseSensitive,
      kind: t.kind,
      title: t.title,
      body: t.body,
      targetPage: src,
      targetRect: t.box ? [t.box.x, t.box.y - t.box.h, t.box.w, t.box.h] : null,
    });
  }
  return out;
}

export interface TermDeps {
  /** Invoke the provider's command with `{ document }`. */
  invoke(choice: TermProviderChoice, value: { document: string }): Promise<void>;
  /** Hand a document's terms to the engine. */
  store(docId: number, terms: EngineTerm[]): Promise<number>;
  /** Take every document's terms back. */
  clearAll(): Promise<void>;
  log(extensionId: string, level: 'info' | 'warn' | 'error', message: string): void;
}

/** Which documents have been asked, under which token, and whether an
 * answer still applies. */
export class TermSession {
  private provider: TermProviderChoice | null = null;
  private asked = new Set<number>();
  private tokens = new Map<string, { docId: number; epoch: number }>();
  private epoch = 0;
  private active: number | null = null;
  private seq = 0;

  constructor(private readonly deps: TermDeps) {}

  /** The active provider changed (or did not). A different one starts over;
   * none takes every term back. */
  setProvider(choice: TermProviderChoice | null): void {
    if (termProviderTag(choice) === termProviderTag(this.provider)) return;
    const had = this.provider !== null;
    this.provider = choice;
    this.asked.clear();
    this.tokens.clear();
    if (had) void this.deps.clearAll();
    if (this.active !== null) this.ensure(this.active);
  }

  /** The document on screen. Switching voids every token handed out. */
  noteActive(docId: number | null): void {
    if (docId === this.active) return;
    this.active = docId;
    this.epoch += 1;
    this.tokens.clear();
    if (docId !== null) this.ensure(docId);
  }

  /** Ask the provider about `docId`, once, if it is the document on screen. */
  ensure(docId: number): void {
    const provider = this.provider;
    if (!provider || docId !== this.active || this.asked.has(docId)) return;
    this.asked.add(docId);
    const document = `terms-${++this.seq}`;
    this.tokens.set(document, { docId, epoch: this.epoch });
    this.deps.invoke(provider, { document }).catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      this.deps.log(provider.extensionId, 'warn', `Terms: ${message}`);
    });
  }

  /** `terms.contribute` from `extensionId`. Applies only while the token is
   * current: the same provider, the same document still on screen and never
   * left since. Otherwise the document is asked again next time it shows. */
  async contribute(
    extensionId: string,
    args: unknown,
    view: { docId: number; pageCount: number; srcOf: (page: number) => number | null } | null
  ): Promise<{ stored: number }> {
    if (!this.provider || this.provider.extensionId !== extensionId) {
      throw new Error('terms.contribute is only for the active term provider');
    }
    const document = (args as { document?: unknown } | undefined)?.document;
    const token = typeof document === 'string' ? this.tokens.get(document) : undefined;
    if (!token || !view || token.epoch !== this.epoch || token.docId !== view.docId) {
      if (token) this.asked.delete(token.docId);
      return { stored: 0 };
    }
    const { terms } = readTermArgs(args, view.pageCount);
    const stored = await this.deps.store(view.docId, toEngineTerms(terms, view.srcOf));
    return { stored };
  }

  /** Forget a document that closed. */
  forget(docId: number): void {
    this.asked.delete(docId);
  }
}
