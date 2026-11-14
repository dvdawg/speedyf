/** What the keyboard shortcut sheet lists, gathered from the registries
 * that actually bind keys rather than from a list kept by hand.
 *
 * Pure: the sheet hands in the commands, the keymap's chords, the chords
 * handled outside the keymap's table, and the extensions' bindings and
 * reader panes; this decides the rows, their areas, and what a filter keeps.
 */
import { formatShortcut } from '../../app/shortcutLabel';

export const SHORTCUT_AREAS = [
  'Files',
  'Navigation',
  'View',
  'Library',
  'Tools',
  'Extensions',
] as const;

export type ShortcutArea = (typeof SHORTCUT_AREAS)[number];

/** Where each command group's shortcuts are listed. */
const AREA_OF_GROUP: Record<string, ShortcutArea> = {
  File: 'Files',
  Navigate: 'Navigation',
  Search: 'Navigation',
  View: 'View',
  Panels: 'View',
  Settings: 'View',
  Library: 'Library',
  Projects: 'Library',
  Edit: 'Tools',
  Tools: 'Tools',
  Page: 'Tools',
};

export interface ShortcutSources {
  commands: readonly { id: string; title: string; group: string; shortcut?: string }[];
  /** Every chord in the keymap's table, by the command it reaches. */
  bound: readonly { id: string; spec: string }[];
  /** Chords handled outside the table. */
  unlisted: readonly { spec: string; title: string; area: ShortcutArea }[];
  /** Extension keybindings and reader pane keys, with the extension's name. */
  extensions: readonly { spec: string; title: string; extension: string }[];
}

export interface ShortcutRow {
  title: string;
  /** Formatted for the platform: `Cmd+Shift+E`. */
  keys: string[];
  /** The extension that bound it. */
  extension?: string;
}

export interface ShortcutSection {
  area: ShortcutArea;
  rows: ShortcutRow[];
}

/** Every shortcut, grouped by area in `SHORTCUT_AREAS` order, rows by title. */
export function shortcutSections(sources: ShortcutSources, mac: boolean): ShortcutSection[] {
  const rows = new Map<ShortcutArea, ShortcutRow[]>();
  const add = (area: ShortcutArea, row: ShortcutRow) => {
    const list = rows.get(area);
    if (list) list.push(row);
    else rows.set(area, [row]);
  };

  const boundById = new Map<string, string[]>();
  for (const { id, spec } of sources.bound) {
    const specs = boundById.get(id);
    if (specs) specs.push(spec);
    else boundById.set(id, [spec]);
  }
  for (const command of sources.commands) {
    const specs = [
      ...(command.shortcut ? [command.shortcut] : []),
      ...(boundById.get(command.id) ?? []),
    ];
    const keys = [...new Set(specs.map((spec) => formatShortcut(spec, mac)))];
    if (keys.length === 0) continue;
    add(AREA_OF_GROUP[command.group] ?? 'Tools', { title: command.title, keys });
  }
  for (const entry of sources.unlisted) {
    add(entry.area, { title: entry.title, keys: [formatShortcut(entry.spec, mac)] });
  }
  for (const entry of sources.extensions) {
    add('Extensions', {
      title: entry.title,
      keys: [formatShortcut(entry.spec, mac)],
      extension: entry.extension,
    });
  }

  return SHORTCUT_AREAS.flatMap((area) => {
    const list = rows.get(area);
    if (!list) return [];
    return [{ area, rows: [...list].sort((a, b) => a.title.localeCompare(b.title)) }];
  });
}

/** The sections whose rows match every word of `query`, in the title, the
 * keys, the extension's name or the area. */
export function filterShortcuts(
  sections: readonly ShortcutSection[],
  query: string
): ShortcutSection[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...sections];
  return sections.flatMap((section) => {
    const rows = section.rows.filter((row) => {
      const haystack = [section.area, row.title, row.extension ?? '', ...row.keys]
        .join(' ')
        .toLowerCase();
      return words.every((word) => haystack.includes(word));
    });
    return rows.length > 0 ? [{ area: section.area, rows }] : [];
  });
}
