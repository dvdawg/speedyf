/** Splitting a result snippet around the phrase that matched, so the match can
 * be emphasized where it sits.
 *
 * The engine reports a match position as an index into the *document*, not into
 * the snippet it hands back, so that number cannot be reused here. Finding the
 * phrase again within the snippet is the honest way across — the snippet was
 * cut around the match, so it is in there.
 *
 * Matching is deliberately loose about case and about runs of whitespace: the
 * snippet is normalized text and the query is whatever was typed, and the
 * backend already treats a phrase broken across a line as matching. Anything
 * not found comes back whole and unemphasized rather than guessed at. An
 * unemphasized snippet reads perfectly well; one emphasized in the wrong place
 * is a lie about where the match was. */

export type SnippetPart = { text: string; match: boolean };

/** Escape a user-typed string for use inside a regular expression. */
function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

export function splitSnippet(snippet: string, query: string): SnippetPart[] {
  const needle = query.trim();
  if (!needle || !snippet) return [{ text: snippet, match: false }];

  // Each run of whitespace in the query matches any run in the text, which is
  // what lets a phrase found across a line break light up here too.
  const pattern = needle.split(/\s+/).map(escape).join('\\s+');

  let matcher: RegExp;
  try {
    matcher = new RegExp(pattern, 'gi');
  } catch {
    // A query that cannot be expressed as a pattern still deserves its snippet.
    return [{ text: snippet, match: false }];
  }

  const parts: SnippetPart[] = [];
  let at = 0;
  for (const found of snippet.matchAll(matcher)) {
    const start = found.index ?? 0;
    // A pattern that can match nothing would loop forever on the same spot.
    if (found[0].length === 0) break;
    if (start > at) parts.push({ text: snippet.slice(at, start), match: false });
    parts.push({ text: found[0], match: true });
    at = start + found[0].length;
  }
  if (at < snippet.length) parts.push({ text: snippet.slice(at), match: false });
  return parts.length > 0 ? parts : [{ text: snippet, match: false }];
}
