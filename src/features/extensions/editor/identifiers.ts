/** DOIs and arXiv ids in pasted text, read into the forms the library
 * stores them in, so a paste can become a citation of the item holding one.
 *
 * The canonical forms are the core's (`src-tauri/src/library/papers.rs`,
 * `normalize_doi` and `normalize_arxiv`; `identifiers.rs` for what an arXiv
 * id may look like): a DOI lower-case without a `doi:` or URL prefix, an
 * arXiv id without its `arXiv:` label, URL or version. Mirrored here only as
 * far as a paste needs. */

export type Identifier = { kind: 'doi'; id: string } | { kind: 'arxiv'; id: string };

/** The first new-style arXiv id was 0704.0001. */
const FIRST_NEW_STYLE_YYMM = 704;

const DOI_PREFIX = /^(?:doi:\s*|https?:\/\/(?:dx\.)?doi\.org\/|(?:dx\.)?doi\.org\/)/i;
const DOI = /^10\.\d{4,9}\/\S+$/;

/** A DOI as the library stores it, or null when `raw` is not one. */
export function normalizeDoi(raw: string): string | null {
  let s = raw.trim();
  const prefix = DOI_PREFIX.exec(s);
  if (prefix) s = s.slice(prefix[0].length);
  if (prefix && /doi\.org\/$/i.test(prefix[0])) {
    try {
      // A URL's DOI may be percent-encoded (`10.1000%2Fabc`).
      s = decodeURIComponent(s);
    } catch {
      /* kept as written */
    }
  }
  if (!DOI.test(s)) return null;
  return s.toLowerCase();
}

const NEW_STYLE = /^(\d{2})(\d{2})\.\d{4,5}$/;
const OLD_STYLE = /^[a-z][a-z-]*(?:\.[A-Z]{2})?\/(\d{2})(\d{2})\d{3}$/;

/** An arXiv id without its version, or null when `raw` is not one. */
export function normalizeArxiv(raw: string): string | null {
  let s = raw.trim();
  const url = /^(?:https?:\/\/)?(?:www\.|export\.)?arxiv\.org\/(?:abs|pdf)\/(.+)$/i.exec(s);
  if (url) {
    s = url[1]!
      .split(/[?#]/)[0]!
      .replace(/\/+$/, '')
      .replace(/\.pdf$/i, '');
  } else if (/^arxiv:/i.test(s)) {
    s = s.slice(6).trim();
  }
  s = s.replace(/v\d+$/i, '');
  const fresh = NEW_STYLE.exec(s);
  if (fresh) {
    const yymm = Number(fresh[1]) * 100 + Number(fresh[2]);
    const month = Number(fresh[2]);
    return month >= 1 && month <= 12 && yymm >= FIRST_NEW_STYLE_YYMM ? s : null;
  }
  const old = OLD_STYLE.exec(s);
  if (old) {
    const month = Number(old[2]);
    return month >= 1 && month <= 12 ? s : null;
  }
  return null;
}

/** What `token` names, or null. A DOI is tried first: `10.48550/arXiv.x`
 * is a DOI. */
export function parseIdentifier(token: string): Identifier | null {
  const doi = normalizeDoi(token);
  if (doi) return { kind: 'doi', id: doi };
  const arxiv = normalizeArxiv(token);
  if (arxiv) return { kind: 'arxiv', id: arxiv };
  return null;
}

/** The index key of an identifier: `doi:10.1/x`, `arxiv:2401.01234`. */
export function identifierKey(id: Identifier): string {
  return `${id.kind}:${id.id.toLowerCase()}`;
}

/** Punctuation around a token that is prose, not identifier. */
const LEAD = /^[(<["']+/;
const TRAIL = /[.,;:!?)\]>"']+$/;

/** `text` with every whitespace-separated token that is exactly one
 * identifier `resolve` knows replaced by `@key`, punctuation around it
 * kept; null when nothing was replaced. */
export function citeIdentifiers(
  text: string,
  resolve: (id: Identifier) => string | null
): string | null {
  let changed = false;
  const out = text.replace(/\S+/g, (token) => {
    const lead = LEAD.exec(token)?.[0] ?? '';
    const core = token.slice(lead.length);
    const trail = TRAIL.exec(core)?.[0] ?? '';
    // The token as it is first (a DOI may end in `)`), then without the
    // prose punctuation after it.
    const tries: [string, string][] = [[core, '']];
    if (trail && trail.length < core.length) tries.push([core.slice(0, -trail.length), trail]);
    for (const [candidate, after] of tries) {
      const id = parseIdentifier(candidate);
      const key = id ? resolve(id) : null;
      if (key) {
        changed = true;
        return `${lead}@${key}${after}`;
      }
    }
    return token;
  });
  return changed ? out : null;
}
