/** What an extension is allowed to reach, and how its keybindings are read.
 *
 * Pure: the store does the dispatching, this decides what is permitted. Every
 * rule below is an ordinary unit test as a result, which matters more here than
 * elsewhere — these functions are the entire security boundary between
 * third-party code and the document you have open.
 */
import type { CommandGroup } from '../../app/commands';

/** Command groups an extension may run with `commands:run`.
 *
 * The line is what a command *changes*: these four only change what you are
 * looking at. Everything else — File, Edit, Page, Tools, Settings — changes the
 * document, the file on disk, or the user's preferences, and a keybinding
 * extension has no business reaching any of it.
 *
 * Narrow on purpose. Opening a group later is a one-line change that breaks
 * nothing; discovering that every installed extension can already delete pages
 * is not recoverable in the same way.
 */
const RUNNABLE_GROUPS: readonly CommandGroup[] = ['View', 'Navigate', 'Panels', 'Search'];

export const COMMANDS_RUN = 'commands:run';

/** Whether `permissions` allow running a command in `group`. */
export function canRunGroup(
  permissions: readonly string[],
  group: CommandGroup | undefined
): boolean {
  if (!permissions.includes(COMMANDS_RUN)) return false;
  return group !== undefined && RUNNABLE_GROUPS.includes(group);
}

/** Why a command was refused, phrased for the extension author reading it in
 * the panel rather than for the user. */
export function refusalReason(
  permissions: readonly string[],
  id: string,
  group: CommandGroup | undefined
): string {
  if (!permissions.includes(COMMANDS_RUN)) {
    return `"${id}" needs the "${COMMANDS_RUN}" permission`;
  }
  if (group === undefined) return `no command "${id}" exists`;
  return `"${id}" is in the ${group} group, which extensions cannot run (allowed: ${RUNNABLE_GROUPS.join(', ')})`;
}

/** A parsed key chord. Mirrors the shape `shortcuts.ts` matches against, so an
 * extension binding and a built-in one are compared on identical terms. */
export interface Chord {
  /** `KeyboardEvent.key`; single characters are compared lower-cased. */
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

/** Parse a neutral chord spec — `j`, `mod+shift+p`, `alt+ArrowDown`.
 *
 * The same vocabulary `commands.ts` already uses for built-in shortcuts, so
 * extension authors and the app describe keys the same way. Returns null for
 * anything unparseable rather than guessing; a binding that cannot be read is
 * reported to its author, never silently dropped.
 */
export function parseChord(spec: string): Chord | null {
  const parts = spec
    .split('+')
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  if (parts.length === 0) return null;

  const chord: Chord = { key: '', mod: false, shift: false, alt: false };
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]!;
    const lower = part.toLowerCase();
    const isLast = i === parts.length - 1;
    if (!isLast) {
      // Everything before the final segment must be a modifier; a stray word
      // here means the author wrote something we would otherwise misread.
      if (lower === 'mod' || lower === 'cmd' || lower === 'ctrl') chord.mod = true;
      else if (lower === 'shift') chord.shift = true;
      else if (lower === 'alt' || lower === 'option') chord.alt = true;
      else return null;
      continue;
    }
    // A modifier in the final position leaves no key to press.
    if (['mod', 'cmd', 'ctrl', 'shift', 'alt', 'option'].includes(lower)) return null;
    chord.key = part.length === 1 ? lower : part;
  }
  return chord.key.length > 0 ? chord : null;
}

/** Whether a keyboard event matches a chord. */
export function chordMatches(chord: Chord, event: KeyboardEvent, mod: boolean): boolean {
  const key = chord.key.length === 1 ? event.key.toLowerCase() : event.key;
  return (
    key === chord.key &&
    chord.mod === mod &&
    chord.shift === event.shiftKey &&
    chord.alt === event.altKey
  );
}

export interface ExtensionBinding {
  extensionId: string;
  /** The extension's own command id, un-namespaced. */
  command: string;
  chord: Chord;
}

/** A keybinding an extension declared, and what became of it. */
export interface BindingOutcome {
  binding: ExtensionBinding | null;
  /** Set when the declaration was rejected; shown in the Extensions panel. */
  error: string | null;
}

/** Read one declared keybinding.
 *
 * `reserved` is every chord spec the app binds itself. A built-in always wins:
 * an extension must not be able to take ⌘S, and quietly losing the app's own
 * shortcut would be a far worse failure than refusing the extension's.
 */
export function resolveBinding(
  extensionId: string,
  declared: { command: string; key: string },
  reserved: ReadonlySet<string>
): BindingOutcome {
  const chord = parseChord(declared.key);
  if (!chord) {
    return { binding: null, error: `"${declared.key}" is not a key SpeedyF understands` };
  }
  const canonical = canonicalChord(chord);
  if (reserved.has(canonical)) {
    return {
      binding: null,
      error: `"${declared.key}" is already a SpeedyF shortcut and cannot be rebound`,
    };
  }
  return { binding: { extensionId, command: declared.command, chord }, error: null };
}

/** A chord's canonical string, for comparing two declarations. Modifier order
 * is fixed here so `shift+mod+p` and `mod+shift+p` are recognised as the same
 * key rather than both being accepted. */
export function canonicalChord(chord: Chord): string {
  const parts: string[] = [];
  if (chord.mod) parts.push('mod');
  if (chord.alt) parts.push('alt');
  if (chord.shift) parts.push('shift');
  parts.push(chord.key);
  return parts.join('+');
}

/** Resolve every declared binding across all extensions.
 *
 * Two extensions claiming the same chord is resolved by install order — the
 * first keeps it, the second is told why it did not get it. Silent last-wins
 * would make the behaviour depend on registry ordering nobody can see.
 */
export function resolveBindings(
  declarations: readonly {
    extensionId: string;
    bindings: readonly { command: string; key: string }[];
  }[],
  reserved: ReadonlySet<string>
): { bindings: ExtensionBinding[]; errors: { extensionId: string; message: string }[] } {
  const bindings: ExtensionBinding[] = [];
  const errors: { extensionId: string; message: string }[] = [];
  const claimed = new Map<string, string>();

  for (const declaration of declarations) {
    for (const declared of declaration.bindings) {
      const outcome = resolveBinding(declaration.extensionId, declared, reserved);
      if (outcome.error || !outcome.binding) {
        errors.push({
          extensionId: declaration.extensionId,
          message: outcome.error ?? 'binding could not be read',
        });
        continue;
      }
      const canonical = canonicalChord(outcome.binding.chord);
      const owner = claimed.get(canonical);
      if (owner) {
        errors.push({
          extensionId: declaration.extensionId,
          message: `"${declared.key}" is already bound by ${owner}`,
        });
        continue;
      }
      claimed.set(canonical, declaration.extensionId);
      bindings.push(outcome.binding);
    }
  }
  return { bindings, errors };
}
