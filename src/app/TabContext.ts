import { createContext } from 'solid-js';
import type { TabRecord } from '../stores/tabsStore';

/** Provided once per open tab's rendered subtree (see App.tsx's <For> over
 * tabsStore.state.tabs). Components that live inside a specific tab's view
 * for its whole mounted life read this instead of a global singleton. */
export const TabContext = createContext<TabRecord>();
