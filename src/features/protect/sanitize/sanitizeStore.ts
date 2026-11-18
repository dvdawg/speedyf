/** Remove hidden information: scan what the document carries, let the user
 * choose, and record the choice as a model edit the next save carries out
 * as a full clean rewrite. */
import { createStore } from 'solid-js/store';
import { engine } from '../../../lib/transport/engine';
import { activeTab, type TabRecord } from '../../../stores/tabsStore';
import { showError } from '../../../stores/modalStore';
import type { SanitizeOptions } from '../../document/documentStore';
import type { HiddenInfo, HiddenKey } from '../types';

export const HIDDEN_LABELS: Record<HiddenKey, string> = {
  metadata: 'Document metadata',
  attachments: 'Attached files',
  javascript: 'JavaScript',
  hiddenLayers: 'Hidden layers',
  hiddenText: 'Hidden text',
  comments: 'Comments and markup',
  forms: 'Form fields (kept as they look)',
  links: 'Links and actions to outside the document',
  bookmarks: 'Bookmarks',
  thumbnails: 'Page thumbnails',
  privateData: 'Private application data',
  revisions: 'Earlier versions of the file',
};

/** Kinds offered unticked: removing them changes how the document is used
 * rather than what it reveals. */
const OFF_BY_DEFAULT: HiddenKey[] = ['bookmarks'];

export const NO_OPTIONS: SanitizeOptions = {
  metadata: false,
  attachments: false,
  javascript: false,
  hiddenLayers: false,
  hiddenText: false,
  comments: false,
  forms: false,
  links: false,
  bookmarks: false,
  thumbnails: false,
  privateData: false,
};

interface SanitizeState {
  open: boolean;
  scanning: boolean;
  info: HiddenInfo | null;
  chosen: Partial<Record<HiddenKey, boolean>>;
}

const [sanitizeState, setSanitizeState] = createStore<SanitizeState>({
  open: false,
  scanning: false,
  info: null,
  chosen: {},
});

export { sanitizeState };

export async function openSanitize(tab: TabRecord | null = activeTab()) {
  const doc = tab?.documentStore.state;
  if (!doc?.loaded) return;
  setSanitizeState({ open: true, scanning: true, info: null, chosen: {} });
  try {
    const info = await engine.scanHiddenInfo(doc.docId);
    const chosen: Partial<Record<HiddenKey, boolean>> = {};
    for (const item of info.items) chosen[item.key] = !OFF_BY_DEFAULT.includes(item.key);
    setSanitizeState({ scanning: false, info, chosen });
  } catch (e) {
    setSanitizeState({ open: false, scanning: false });
    showError(`Could not inspect the document: ${String(e)}`);
  }
}

export function closeSanitize() {
  setSanitizeState({ open: false, info: null, chosen: {} });
}

export function choose(key: HiddenKey, on: boolean) {
  setSanitizeState('chosen', key, on);
}

/** The chosen kinds as options for the save. */
export function chosenOptions(chosen: Partial<Record<HiddenKey, boolean>>): SanitizeOptions {
  const options = { ...NO_OPTIONS };
  for (const key of Object.keys(options) as (keyof SanitizeOptions)[]) {
    options[key] = chosen[key] === true;
  }
  return options;
}

/** Record the removal; the next save performs it. */
export function confirmSanitize(tab: TabRecord | null = activeTab()) {
  const store = tab?.documentStore;
  if (!store) return;
  store.apply({
    type: 'setScrub',
    scrub: { ...store.state.scrub, removeHidden: chosenOptions(sanitizeState.chosen) },
  });
  closeSanitize();
}
