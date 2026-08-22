/** Ranking for the command palette.
 *
 * Two properties this is built around.
 *
 * **Structure first, popularity second.** A usage boost applied as a plain
 * multiplier lets a much-used command outrank an exact match, so typing "set"
 * surfaces "Insert Blank Page" (i-n-s-e-r-t contains it) because you insert
 * pages a lot. Scoring the shape of the match first, then adding a log-scaled
 * boost, keeps a prefix match unbeatable however popular anything else is.
 *
 * **Optimal alignment, not greedy.** Matching each query character at its
 * first available position is the obvious implementation and it is wrong: the
 * largest bonus here is for landing on a word start, and committing early
 * throws that away. Greedily matching "tool" against "Select Tool" takes the
 * `t` inside "selec*t*" and never reaches "*T*ool", which scored Ink Tool at
 * 172 and Note Tool at 69 for the same query — an ordering with no meaning.
 * The alignment below is a small dynamic program that finds the best set of
 * positions instead of the first one it stumbles into.
 */

/** Bigger than any alignment can reach, so a prefix always wins outright. */
const PREFIX_BONUS = 1000;
/** A match at a word start is what someone typing initials means. */
const WORD_START = 60;
/** Continuing directly from the previous match. */
const CONSECUTIVE = 20;
/** Every matched character is worth something on its own. */
const CHAR = 8;
/** Opening a gap, then widening it. Together these prefer tight matches. */
const GAP_START = 6;
const GAP_EXTEND = 2;
/** Starting further into the text is slightly worse. */
const LEADING = 1;

/** How far a match may spread, as a multiple of the query length.
 *
 * Subsequence matching over long prose matches nearly anything: "tool" really
 * does occur in "B. Proof of the Leading-Order Identification and Variance
 * Collapse" (*t*he, *o*rder, identificati*o*n, co*l*lapse), and a title with
 * ten words offers ten chances at the word-start bonus, so the junk scores
 * respectably too. Command titles run 4-26 characters and section titles
 * 40-80, so the same matcher behaves completely differently across them.
 *
 * Spread is the signal that separates the two. Four characters smeared over
 * fifty are not a match however many word starts they clipped. The bound is a
 * ratio rather than a cap because the honest long-span case exists: typing
 * "proof collapse" against that title *should* span sixty characters. That is
 * a ratio near four; the junk above is nearer thirteen.
 */
const MAX_SPREAD_RATIO = 6;
/** Never reject a match that is compact in absolute terms, whatever the ratio
 * says — short titles would otherwise fall foul of it. */
const MIN_SPREAD_SPAN = 16;

const NEG = Number.NEGATIVE_INFINITY;

function isBoundary(text: string, index: number): boolean {
  if (index === 0) return true;
  const previous = text[index - 1]!;
  return previous === ' ' || previous === '-' || previous === '&' || previous === '(';
}

/** Score the best alignment of `query` within `text`, or null if it does not
 * occur as a subsequence at all.
 *
 * Case-insensitive. An empty query matches everything at zero, which lets the
 * caller keep its own ordering for the untyped state.
 */
export function subsequenceScore(text: string, query: string): number | null {
  if (query.length === 0) return 0;
  const haystack = text.toLowerCase();
  const needle = query.toLowerCase();
  if (needle.length > haystack.length) return null;

  if (haystack.startsWith(needle)) {
    // Shorter titles are the better answer for the same prefix ("Save" above
    // "Save As…"). Reciprocal rather than subtractive so it never saturates:
    // section titles from real papers run well past a hundred characters, and
    // a clamped term would tie every one of them.
    return PREFIX_BONUS + 200 / (1 + text.length);
  }

  const n = haystack.length;
  const m = needle.length;
  // best[j] = score of the best alignment of the query so far that ends
  // exactly at text position j; from[j] is where that alignment began, which
  // is what makes its spread measurable at the end.
  let best = new Array<number>(n).fill(NEG);
  let from = new Array<number>(n).fill(-1);

  for (let i = 0; i < m; i += 1) {
    const row = new Array<number>(n).fill(NEG);
    const rowFrom = new Array<number>(n).fill(-1);
    const ch = needle[i]!;
    // Running best over positions far enough back to count as a gap, decayed
    // as it travels so a wider gap is worth less than a narrow one.
    let carried = NEG;
    let carriedFrom = -1;

    for (let j = 0; j < n; j += 1) {
      if (carried !== NEG) carried -= GAP_EXTEND;
      if (j >= 2 && best[j - 2]! !== NEG) {
        const opened = best[j - 2]! - GAP_START;
        if (carried === NEG || opened > carried) {
          carried = opened;
          carriedFrom = from[j - 2]!;
        }
      }
      if (haystack[j] !== ch) continue;

      let base: number;
      let start: number;
      if (i === 0) {
        base = -j * LEADING;
        start = j;
      } else {
        const adjacent = j >= 1 && best[j - 1]! !== NEG ? best[j - 1]! + CONSECUTIVE : NEG;
        if (adjacent !== NEG && adjacent >= carried) {
          base = adjacent;
          start = from[j - 1]!;
        } else if (carried !== NEG) {
          base = carried;
          start = carriedFrom;
        } else {
          continue;
        }
      }
      row[j] = base + CHAR + (isBoundary(haystack, j) ? WORD_START : 0);
      rowFrom[j] = start;
    }
    best = row;
    from = rowFrom;
  }

  let top = NEG;
  let end = -1;
  for (let j = 0; j < n; j += 1) {
    if (best[j]! > top) {
      top = best[j]!;
      end = j;
    }
  }
  if (top === NEG) return null;

  const span = end - from[end]! + 1;
  if (span > m * MAX_SPREAD_RATIO && span > MIN_SPREAD_SPAN) return null;
  return top;
}

/** The best score across a title and its hidden keywords.
 *
 * A keyword match is worth less than a title match — "export" should find the
 * notes panel, but never above a command actually called Export.
 */
export function matchScore(
  title: string,
  keywords: readonly string[] | undefined,
  query: string
): number | null {
  let best = subsequenceScore(title, query);
  for (const keyword of keywords ?? []) {
    const scored = subsequenceScore(keyword, query);
    if (scored === null) continue;
    const discounted = Math.min(scored, PREFIX_BONUS - 1) * 0.6;
    best = best === null ? discounted : Math.max(best, discounted);
  }
  return best;
}

/** Popularity, folded in after the structural score.
 *
 * Logarithmic and capped: the twentieth use of a command should not move it
 * further than the second did, and no amount of use should overturn a prefix
 * match.
 */
export function usageBoost(uses: number): number {
  if (uses <= 0) return 0;
  return Math.min(40, Math.log2(uses + 1) * 12);
}
