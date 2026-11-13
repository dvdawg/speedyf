/** Keep ordinary clicks and small slips from detaching a tab. Pointer capture
 * keeps the gesture alive beyond the strip (including beyond the webview). */
export function isTabDrag(start: { x: number; y: number }, x: number, y: number): boolean {
  return Math.hypot(x - start.x, y - start.y) >= 6;
}

export function isOutsideTabStrip(
  rect: { left: number; right: number; top: number; bottom: number },
  x: number,
  y: number
): boolean {
  const slop = 32;
  return (
    x < rect.left - slop || x > rect.right + slop || y < rect.top - slop || y > rect.bottom + slop
  );
}
