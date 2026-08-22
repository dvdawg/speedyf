/** Turning candidates into the grouped list the palette draws.
 *
 * Pure: the component gathers candidates from the stores, this decides what
 * survives, in what order, under which headings. Every rule below is an
 * ordinary unit test as a result.
 */
import { matchScore, usageBoost } from './fuzzyScore';

export interface Candidate {
  key: string;
  title: string;
  group: string;
  keywords?: readonly string[];
  /** Dim text on the right: a page number, a file name. */
  context?: string;
  /** Already formatted for this platform. */
  shortcut?: string;
  enabled: boolean;
  /** Shown instead of the shortcut when a disabled row is surfaced. */
  reason?: string;
  run(): void;
}

export interface Section {
  group: string;
  items: Candidate[];
}

/** Rows per heading. Typing more is the way to see past this; a palette that
 * needs scrolling is usually answering the wrong question. */
const PER_GROUP = 5;
/** A match scoring less than this share of the best one is noise, not a
 * weaker answer. */
const WEAK_MATCH_FRACTION = 0.4;
/** Headings appear in this order whenever scores do not decide it. */
const GROUP_ORDER = [
  'Recent',
  'Sections',
  'Environments',
  'Tabs',
  'Navigate',
  'File',
  'Edit',
  'Tools',
  'Page',
  'View',
  'Panels',
  'Search',
  'Settings',
];

function groupRank(group: string): number {
  const at = GROUP_ORDER.indexOf(group);
  return at < 0 ? GROUP_ORDER.length : at;
}

function sectionize(items: Candidate[], perGroup: number): Section[] {
  const byGroup = new Map<string, Candidate[]>();
  for (const item of items) {
    const bucket = byGroup.get(item.group);
    if (bucket) bucket.push(item);
    else byGroup.set(item.group, [item]);
  }
  return [...byGroup.entries()]
    .map(([group, all]) => ({ group, items: all.slice(0, perGroup) }))
    .filter((section) => section.items.length > 0);
}

/** The untyped list: what you reach for most, then everything available.
 *
 * A remembered command that is not currently runnable is dropped rather than
 * shown greyed — offering "Delete Page" on the home screen and doing nothing
 * is worse than a shorter list.
 */
export function defaultSections(
  candidates: readonly Candidate[],
  recentKeys: readonly string[],
  perGroup: number = PER_GROUP
): Section[] {
  const enabled = candidates.filter((candidate) => candidate.enabled);
  const byKey = new Map(enabled.map((candidate) => [candidate.key, candidate]));

  const recent = recentKeys
    .map((key) => byKey.get(key))
    .filter((candidate): candidate is Candidate => candidate !== undefined)
    .slice(0, 4);
  const recentKeySet = new Set(recent.map((candidate) => candidate.key));

  const rest = sectionize(
    enabled.filter((candidate) => !recentKeySet.has(candidate.key)),
    perGroup
  ).sort((a, b) => groupRank(a.group) - groupRank(b.group));

  return recent.length > 0 ? [{ group: 'Recent', items: recent }, ...rest] : rest;
}

/** The typed list.
 *
 * Disabled commands are hidden, with one exception: an exact title match is
 * surfaced greyed, carrying its reason. Something typed in full and then
 * silently absent is the confusing case; one dim row answers it.
 */
export function rankCandidates(
  candidates: readonly Candidate[],
  query: string,
  uses: (key: string) => number,
  perGroup: number = PER_GROUP
): Section[] {
  const trimmed = query.trim();
  if (trimmed.length === 0) return [];
  const exact = trimmed.toLowerCase();

  const scored: { candidate: Candidate; score: number; prefix: boolean }[] = [];
  for (const candidate of candidates) {
    const lower = candidate.title.toLowerCase();
    const isExact = lower === exact;
    if (!candidate.enabled && !isExact) continue;
    const score = matchScore(candidate.title, candidate.keywords, trimmed);
    if (score === null) continue;
    // A disabled row is only ever a footnote, never a ranked answer.
    const boosted = candidate.enabled ? score + usageBoost(uses(candidate.key)) : score - 10_000;
    scored.push({ candidate, score: boosted, prefix: lower.startsWith(exact) });
  }

  scored.sort((a, b) => b.score - a.score || a.candidate.title.localeCompare(b.candidate.title));

  // Everything is a subsequence of enough prose, so a weak match is not a
  // weaker answer — it is noise. Once the best answer is known, anything
  // scoring a small fraction of it is dropped.
  //
  // The reference is the best *non-prefix* score, because the two are not
  // comparable: a prefix match scores an order of magnitude higher by design,
  // so measuring against one would delete every alignment match beside it —
  // typing "tab" would surface "Table of Contents" and drop "Next Tab".
  const topAligned = scored.find((entry) => !entry.prefix && entry.candidate.enabled)?.score ?? 0;
  const floor = Math.max(topAligned * WEAK_MATCH_FRACTION, 1);
  const kept = scored.filter(
    // Prefix matches are never noise, and a disabled row is the exact-title
    // footnote, deliberately scored far below everything.
    (entry) => entry.prefix || !entry.candidate.enabled || entry.score >= floor
  );

  const best = new Map<string, number>();
  for (const { candidate, score } of kept) {
    const current = best.get(candidate.group);
    if (current === undefined || score > current) best.set(candidate.group, score);
  }

  return sectionize(
    kept.map((entry) => entry.candidate),
    perGroup
  ).sort((a, b) => (best.get(b.group) ?? 0) - (best.get(a.group) ?? 0));
}

/** A bare number is a page, not a search. Returns the 1-based page, or null.
 *
 * Out-of-range numbers return null rather than clamping: offering "go to page
 * 900" in a 30-page document would be a lie.
 */
export function pageJumpTarget(query: string, pageCount: number): number | null {
  const trimmed = query.trim();
  if (!/^\d{1,7}$/.test(trimmed)) return null;
  const page = Number(trimmed);
  if (page < 1 || page > pageCount) return null;
  return page;
}

/** Every row in draw order, for keyboard movement across group boundaries. */
export function flatten(sections: readonly Section[]): Candidate[] {
  return sections.flatMap((section) => section.items);
}
