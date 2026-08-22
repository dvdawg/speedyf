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

/** Commands most recently reached, most-used first. */
export function mostUsed(limit: number): string[] {
  return Object.entries(uses())
    .sort((a, b) => b[1] - a[1])
    .slice(0, limit)
    .map(([id]) => id);
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

export function openPalette(): void {
  setIsOpen(true);
}

export function closePalette(): void {
  setIsOpen(false);
}
