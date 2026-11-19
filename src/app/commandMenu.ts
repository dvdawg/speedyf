/** A command from the registry as a right-click menu entry: the same title,
 * enabled state and checkmark the menu bar and the palette show. */
import { commandById, type CommandCtx } from './commands';
import type { MenuEntry } from '../lib/nativeMenu';

/** `id` as a menu entry, or nothing when no such command exists. */
export function commandEntry(id: string, ctx: CommandCtx, text?: string): MenuEntry[] {
  const command = commandById(id);
  if (!command) return [];
  const checked = command.checked?.(ctx);
  return [
    {
      text: text ?? command.title,
      enabled: command.enabled(ctx),
      ...(checked === undefined ? {} : { checked }),
      run: () => {
        if (command.enabled(ctx)) command.run(ctx);
      },
    },
  ];
}
