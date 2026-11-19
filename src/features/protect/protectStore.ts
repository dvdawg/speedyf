/** Which tab the Protect dialog is open for, and the two things it does to
 * that tab: stage new protection for the next save, and unlock the file's
 * permissions with its permissions password. */
import { createSignal } from 'solid-js';
import type { TabRecord } from '../../stores/tabsStore';
import type { ProtectRequest } from '../../types/engine';
import { engine, isEngineError } from '../../lib/transport/engine';
import { askConfirm } from '../../stores/modalStore';
import { flashStatus } from '../../stores/statusMessage';

const [protectTab, setProtectTab] = createSignal<TabRecord | null>(null);
export { protectTab };

export function openProtect(tab: TabRecord): void {
  if (!tab.documentStore.state.loaded) return;
  setProtectTab(tab);
}

export function closeProtect(): void {
  setProtectTab(null);
}

const plural = (n: number, one: string) => (n === 1 ? `${n} ${one}` : `${n} ${one}s`);

/** Stage `request` for the next save. A signed document is asked about
 * first: changing protection rewrites the file, and its signatures stop
 * verifying. Answers whether the change was made. */
export async function applyProtection(
  tab: TabRecord,
  request: ProtectRequest | null
): Promise<boolean> {
  const store = tab.documentStore;
  const signatures = store.state.protection?.signatures ?? 0;
  if (request !== null && signatures > 0) {
    const ok = await askConfirm(
      `This document has ${plural(signatures, 'digital signature')}. Changing its protection ` +
        'rewrites the file when you save, and the signatures will no longer verify.',
      'Change protection',
      { destructive: true }
    );
    if (!ok) return false;
  }
  if (JSON.stringify(request) === JSON.stringify(store.state.protectChange)) return true;
  store.apply({ type: 'setProtection', request });
  flashStatus('Protection changes apply when you save');
  return true;
}

/** Try `password` as the file's permissions password. Answers an error to
 * show, or null once the session holds every right. */
export async function unlockPermissions(tab: TabRecord, password: string): Promise<string | null> {
  const store = tab.documentStore;
  try {
    const protection = await engine.unlockPermissions(store.state.docId, password);
    store.setProtectionInfo(protection);
    return null;
  } catch (e) {
    if (isEngineError(e) && e.code === 'password') return 'That is not the permissions password.';
    return isEngineError(e) ? e.message : String(e);
  }
}
