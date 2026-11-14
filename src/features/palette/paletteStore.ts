/** Command palette state: whether it is open, and which commands get used.
 *
 * Separate from `modalStore`, which is promise-based — every entry there
 * resolves a value back to a caller that is awaiting an answer. The palette
 * answers nobody; it is transient chrome, so it lives with the other
 * window-level UI state.
 */
import { createSignal } from 'solid-js';

const USES_KEY = 'speedyf.palette.uses';
/** Keep the tail bounded: a command used once a year is not worth ranking. */
const MAX_TRACKED = 60;

const [isOpen, setIsOpen] = createSignal(false);
export { isOpen as paletteOpen };

const [uses, setUses] = createSignal<Readonly<Record<string, number>>>(readUses());

function readUses(): Record<string, number> {
  try {
    const raw = localStorage.getItem(USES_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return {};
    const out: Record<string, number> = {};
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === 'number' && Number.isFinite(value) && value > 0) out[id] = value;
    }
    return out;
  } catch {
    // A corrupt or unavailable store costs ranking quality, never the palette.
    return {};
  }
}

export function usesOf(id: string): number {
  return uses()[id] ?? 0;
}

export function recordUse(id: string): void {
  const next = { ...uses(), [id]: (uses()[id] ?? 0) + 1 };
  const trimmed = Object.fromEntries(
    Object.entries(next)
      .sort((a, b) => b[1] - a[1])
      .slice(0, MAX_TRACKED)
  );
  setUses(trimmed);
  try {
    localStorage.setItem(USES_KEY, JSON.stringify(trimmed));
  } catch {
    // Ranking is a nicety; failing to persist it must not fail the command.
  }
}

const RECENT_KEY = 'speedyf.palette.recent';
/** Kept a little longer than shown, so a recent command that is unavailable
 * right now leaves room for the one run before it. */
const MAX_RECENT = 10;

/** Rows that are places, not commands: a tab, a section, a page. Reaching
 * one is not running a command, and its key means nothing next session. */
const NOT_COMMANDS = /^(tab|sec|env|page):/;

const [recent, setRecent] = createSignal<readonly string[]>(readRecent());

function readRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((id): id is string => typeof id === 'string').slice(0, MAX_RECENT);
  } catch {
    return [];
  }
}

/** Commands run from the palette, most recent first, by id. */
export function recentCommands(): readonly string[] {
  return recent();
}

/** Note a command run from the palette. Places are ignored. */
export function recordRecent(id: string): void {
  if (NOT_COMMANDS.test(id)) return;
  const next = [id, ...recent().filter((other) => other !== id)].slice(0, MAX_RECENT);
  setRecent(next);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Remembering is a nicety; failing to must not fail the command.
  }
}

export function openPalette(): void {
  setIsOpen(true);
}

export function closePalette(): void {
  setIsOpen(false);
}
