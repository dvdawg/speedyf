/** Set while toolbar controls render inside the overflow menu: IconButton
 * then draws a labelled menu row instead of a bare icon. */
import { createContext } from 'solid-js';

export interface ToolbarMenu {
  /** Close the overflow menu and return focus to its button. */
  close(): void;
}

export const ToolbarMenuContext = createContext<ToolbarMenu | null>(null);
