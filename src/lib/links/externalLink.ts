/** Reading a document's outbound links: what to show the user before leaving
 * the app, and what a "don't ask again" decision is allowed to cover.
 *
 * The engine re-validates every link before opening it (see external.rs).
 * This module exists so the *question* the user is asked names the same
 * destination the engine will act on. */

const WEB_SCHEMES = new Set(['http:', 'https:']);

export interface ExternalLink {
  /** normalized href — what actually gets opened */
  href: string;
  /** lowercase host, or null when there is no host to remember (mailto:) */
  host: string | null;
}

/** Parse a link the document carries, or return null if SpeedyF will not
 * follow it at all. Mirrors the engine's scheme allowlist. */
export function parseExternalLink(raw: string): ExternalLink | null {
  let parsed: URL;
  try {
    parsed = new URL(raw.trim());
  } catch {
    return null;
  }
  if (parsed.protocol === 'mailto:') return { href: parsed.href, host: null };
  if (!WEB_SCHEMES.has(parsed.protocol) || !parsed.hostname) return null;
  return { href: parsed.href, host: parsed.hostname.toLowerCase() };
}

/** Whether the user has already agreed to this destination.
 *
 * Matching is exact, never a suffix: trusting `arxiv.org` must not also trust
 * `arxiv.org.example.com`, which is precisely the shape a link pretending to
 * be somewhere familiar takes. */
export function isTrustedLink(link: ExternalLink, trusted: readonly string[]): boolean {
  return link.host !== null && trusted.includes(link.host);
}

/** Add a host to the trust list without duplicating it. */
export function withTrustedHost(trusted: readonly string[], host: string): string[] {
  return trusted.includes(host) ? [...trusted] : [...trusted, host];
}
