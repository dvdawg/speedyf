/** Which toolbar groups fold into the trailing overflow menu.
 *
 * The toolbar is one row and never wraps. When its groups do not fit, the
 * least important go first - by priority, not by position, as an AppKit
 * toolbar does it - until the rest fit beside the overflow button. Pinned
 * groups (priority Infinity) always stay. Pure, so it can be tested without
 * a layout engine; Toolbar.tsx feeds it measured widths. */

export interface OverflowGroup {
  id: string;
  /** Border-box width when shown, in CSS pixels. */
  width: number;
  /** Higher stays longer; Infinity never folds. */
  priority: number;
  /** Leaves the row without a place in the menu (a hint, a caption), so
   * folding it alone needs no overflow button. */
  drops?: boolean | undefined;
}

export interface OverflowLayout {
  /** Room in the row for groups, in CSS pixels. */
  available: number;
  /** Space the row puts between neighbours. */
  gap: number;
  /** Width of the overflow button itself. */
  buttonWidth: number;
}

function rowWidth(groups: readonly OverflowGroup[], gap: number): number {
  if (groups.length === 0) return 0;
  return groups.reduce((sum, g) => sum + g.width, 0) + gap * (groups.length - 1);
}

/** Ids of the groups to fold, in toolbar order. */
export function foldedGroups(groups: readonly OverflowGroup[], layout: OverflowLayout): string[] {
  if (rowWidth(groups, layout.gap) <= layout.available) return [];
  // Lowest priority first; between equals, the later one folds first.
  const order = groups
    .map((g, index) => ({ g, index }))
    .filter(({ g }) => g.priority !== Infinity)
    .sort((a, b) => a.g.priority - b.g.priority || b.index - a.index);
  const folded = new Set<string>();
  const fits = () => {
    // Once anything goes to the menu, the overflow button takes a slot of
    // its own at the end of the row.
    const button = groups.some((g) => folded.has(g.id) && !g.drops);
    const budget = layout.available - (button ? layout.buttonWidth + layout.gap : 0);
    return (
      rowWidth(
        groups.filter((x) => !folded.has(x.id)),
        layout.gap
      ) <= budget
    );
  };
  for (const { g } of order) {
    if (fits()) break;
    folded.add(g.id);
  }
  return groups.filter((g) => folded.has(g.id)).map((g) => g.id);
}

/** A toolbar label split for a menu row: "Save (Cmd+S)" reads as the name
 * "Save" with the shortcut "Cmd+S" set apart. A parenthesis that is not a
 * key ("Edit mode (page reorder, ...)") stays part of the name. */
export function splitShortcut(label: string): { name: string; key: string | null } {
  const match = /^(.*\S) \(([^()]+)\)(.*)$/.exec(label);
  if (!match) return { name: label, key: null };
  const [, name, key, rest] = match;
  const isKey = /^((Cmd|Ctrl|Shift|Option|Alt)\+\S|Esc$|Escape$|Page (Up|Down)$)/.test(key!);
  if (!isKey) return { name: label, key: null };
  return { name: `${name!}${rest!}`, key: key! };
}
