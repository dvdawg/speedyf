/** The math popover's state: one popover for the window, owned by SpeedyF
 * and filled by an extension through `ui.showMath`.
 *
 * An extension does not draw it and cannot put anything in it but a formula
 * (typeset by KaTeX), where it came from and how sure it is, or the error
 * its action ended with. It opens in a loading state the moment the user
 * starts an action, so a request that takes a few seconds (arXiv asks for one
 * every three) is visibly under way rather than silent.
 */
import { createSignal } from 'solid-js';
import type { MathConfidence, MathSource, ShowMathArgs } from './api';

/** Where the popover points, in client pixels. */
export interface MathAnchor {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface Common {
  extensionId: string;
  anchor: MathAnchor;
}

export type MathPopoverState =
  | (Common & { phase: 'loading'; title: string })
  | (Common & {
      phase: 'shown';
      latex: string;
      display: boolean;
      source: MathSource;
      confidence: MathConfidence;
    })
  | (Common & { phase: 'error'; message: string });

const [mathPopover, setMathPopover] = createSignal<MathPopoverState | null>(null);

export { mathPopover };

/** Somewhere sensible when nothing better is known: the upper middle of the
 * window. */
function fallbackAnchor(): MathAnchor {
  const width = typeof window === 'undefined' ? 1024 : window.innerWidth;
  const middle = width / 2;
  return { left: middle, top: 96, right: middle, bottom: 96 };
}

/** An action has started: show that it is working, where its answer will
 * appear. */
export function openMathLoading(extensionId: string, anchor: MathAnchor, title: string): void {
  setMathPopover({ phase: 'loading', extensionId, anchor, title });
}

/** `ui.showMath` from `extensionId`. Lands where that extension's action
 * opened the popover; otherwise at `anchor`, else at the fallback. */
export function showMath(
  extensionId: string,
  args: ShowMathArgs,
  anchor?: MathAnchor | null
): void {
  const current = mathPopover();
  const at =
    current && current.extensionId === extensionId ? current.anchor : (anchor ?? fallbackAnchor());
  setMathPopover({ phase: 'shown', extensionId, anchor: at, ...args });
}

/** An action failed. Shown in place of the formula, in the same popover,
 * when the popover is still waiting on that extension. */
export function showMathError(extensionId: string, message: string): void {
  const current = mathPopover();
  if (!current || current.extensionId !== extensionId || current.phase === 'shown') return;
  setMathPopover({ phase: 'error', extensionId, anchor: current.anchor, message });
}

/** An action finished. A popover still loading for it closes: the extension
 * chose to show nothing. */
export function settleMath(extensionId: string): void {
  const current = mathPopover();
  if (current?.phase === 'loading' && current.extensionId === extensionId) setMathPopover(null);
}

export function closeMath(): void {
  setMathPopover(null);
}

/** Where to put a popover of `width` x `height` pointing at `anchor` in a
 * viewport of `viewport`: under it when there is room, else above, and kept
 * 8px inside the window. */
export function placeMathPopover(
  anchor: MathAnchor,
  size: { width: number; height: number },
  viewport: { width: number; height: number }
): { left: number; top: number } {
  const margin = 8;
  const gap = 6;
  const centre = (anchor.left + anchor.right) / 2;
  const left = Math.min(
    Math.max(margin, centre - size.width / 2),
    Math.max(margin, viewport.width - size.width - margin)
  );
  const below = anchor.bottom + gap;
  const above = anchor.top - gap - size.height;
  const top =
    below + size.height <= viewport.height - margin || above < margin
      ? Math.min(below, Math.max(margin, viewport.height - size.height - margin))
      : above;
  return { left, top };
}
