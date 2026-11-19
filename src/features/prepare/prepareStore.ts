/** Prepare form mode: which tab is being prepared, the tool in hand, the
 * selection, auto-detect's proposals, and the edits, each one a
 * `setFormDesign` in the document store (so undo, dirty and save all work
 * as for any other edit).
 *
 * The mode is entered from the palette, the Form panel or the Sign menu,
 * never a toolbar button of its own: most PDFs opened are papers, not forms.
 * While it is on, the Form panel shows the field properties and the page
 * shows every field as a box to select, move and resize. */
import { batch, createSignal } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import type { ProposedField } from '../../types/engine';
import type { PageId } from '../../types/model';
import type { TabRecord } from '../../stores/tabsStore';
import { activeTab } from '../../stores/tabsStore';
import { engine } from '../../lib/transport/engine';
import { flashStatus } from '../../stores/statusMessage';
import { formInfo } from '../forms/formStore';
import { setActiveTool } from '../annotations/toolStore';
import { allows } from '../protect/permissions';
import { askConfirm } from '../../stores/modalStore';
import { documentSignatures } from '../signatures/signatureStore';
import {
  addFields,
  designFromForm,
  EMPTY_DESIGN,
  fieldFromProposal,
  namesOf,
  type FormDesign,
  type PrepareTool,
} from './model';

export interface Proposal extends ProposedField {
  id: string;
  pageId: PageId;
}

interface PrepareState {
  tool: PrepareTool;
  selected: string[];
  /** Show each field's place in the Tab order on the page. */
  showTabs: boolean;
  proposals: Proposal[];
  detecting: boolean;
  /** The form as the file has it, per `${tab}:${doc}`. */
  bases: Record<string, FormDesign>;
}

const [state, setState] = createStore<PrepareState>({
  tool: 'select',
  selected: [],
  showTabs: false,
  proposals: [],
  detecting: false,
  bases: {},
});
export { state as prepareState };

const [tabId, setTabId] = createSignal<string | null>(null);

const baseKey = (tab: TabRecord) =>
  `${tab.id}:${tab.documentStore.state.docId}:${tab.documentStore.state.generation}`;

/** Whether `tab` (else the active tab) is in Prepare form mode. */
export function preparing(tab?: TabRecord | null): boolean {
  const t = tab === undefined ? activeTab() : tab;
  return !!t && tabId() === t.id && t.documentStore.state.loaded;
}

/** The form as it is being designed. */
export function designOf(tab: TabRecord): FormDesign {
  return tab.documentStore.state.formDesign ?? state.bases[baseKey(tab)] ?? EMPTY_DESIGN;
}

/** Record `next` as one edit (coalesced with others of the same `group`). */
export function commitDesign(tab: TabRecord, next: FormDesign, group?: symbol) {
  tab.documentStore.apply({ type: 'setFormDesign', design: next }, group);
}

/** Turn flatten-on-save on or off. Flattening removes the whole form,
 * signature fields included, so on a signed document it asks first. */
export async function setFlatten(tab: TabRecord, on: boolean): Promise<boolean> {
  const doc = tab.documentStore.state;
  if (on) {
    const signed = (await documentSignatures(doc.docId, doc.path)).length > 0;
    if (
      signed &&
      !(await askConfirm(
        'This document is digitally signed. Flattening the form removes its signature ' +
          'fields, and the saved file will show its signatures as broken. Flatten anyway?',
        'Flatten form',
        { destructive: true }
      ))
    ) {
      return false;
    }
  }
  commitDesign(tab, { ...designOf(tab), flatten: on });
  return true;
}

export async function startPreparing(tab: TabRecord | null | undefined = activeTab()) {
  if (!tab || !tab.documentStore.state.loaded) return;
  if (!allows(tab.documentStore.state.protection, 'edit')) {
    flashStatus("This document's permissions do not allow changing its form");
    return;
  }
  const info = await formInfo(tab.documentStore.state.docId);
  const key = baseKey(tab);
  batch(() => {
    if (!state.bases[key]) {
      setState('bases', key, designFromForm(info, tab.documentStore.state.pages));
    }
    setState(
      produce((s) => {
        s.tool = 'select';
        s.selected = [];
        s.proposals = [];
      })
    );
    setTabId(tab.id);
    setActiveTool('select');
    tab.viewport.setState('formPanelOpen', true);
  });
}

/** Read the form in again for a tab whose document was reopened (a save
 * reopens it) while the mode stayed on. */
export async function ensureBase(tab: TabRecord) {
  const key = baseKey(tab);
  if (state.bases[key] || !tab.documentStore.state.loaded) return;
  const info = await formInfo(tab.documentStore.state.docId);
  if (baseKey(tab) !== key || state.bases[key]) return;
  setState('bases', key, designFromForm(info, tab.documentStore.state.pages));
}

export function stopPreparing() {
  batch(() => {
    setTabId(null);
    setState(
      produce((s) => {
        s.tool = 'select';
        s.selected = [];
        s.proposals = [];
        s.showTabs = false;
      })
    );
  });
}

export function togglePreparing(tab: TabRecord | null | undefined = activeTab()) {
  if (preparing(tab ?? null)) stopPreparing();
  else void startPreparing(tab);
}

export function setTool(tool: PrepareTool) {
  setState('tool', tool);
}

export function select(ids: string[]) {
  setState('selected', ids);
}

export function setShowTabs(on: boolean) {
  setState('showTabs', on);
}

/** The selected fields that still exist (an undo can take one away). */
export function selectedIds(tab: TabRecord): string[] {
  const ids = new Set(designOf(tab).fields.map((f) => f.id));
  return state.selected.filter((id) => ids.has(id));
}

// ---------------------------------------------------------------------------
// Auto-detect
// ---------------------------------------------------------------------------

let proposalSeq = 0;

/** Propose fields from the printed blanks on every page. Areas that hold
 * a field already - from the file or made here - are skipped. */
export async function detectFields(tab: TabRecord | null | undefined = activeTab()) {
  if (!tab) return;
  if (!preparing(tab)) await startPreparing(tab);
  if (!preparing(tab)) return;
  setState('detecting', true);
  try {
    const found = await engine.getFormProposals(tab.documentStore.state.docId);
    const pages = tab.documentStore.state.pages;
    const design = designOf(tab);
    const proposals: Proposal[] = [];
    for (const p of found) {
      const page = pages.find((pg) => pg.srcIndex === p.page);
      if (!page) continue;
      const covered = design.fields.some(
        (f) => f.pageId === page.id && overlapShare(f.rect, p.rect) > 0.3
      );
      if (covered) continue;
      proposalSeq += 1;
      proposals.push({ ...p, id: `prop-${proposalSeq}`, pageId: page.id });
    }
    setState('proposals', proposals);
    flashStatus(
      proposals.length === 0
        ? 'No blanks found to make fields from'
        : `${proposals.length} field${proposals.length === 1 ? '' : 's'} proposed: accept or reject them`
    );
  } catch (err) {
    flashStatus(`Could not detect fields: ${err instanceof Error ? err.message : String(err)}`);
  } finally {
    setState('detecting', false);
  }
}

function overlapShare(
  a: { x: number; y: number; w: number; h: number },
  b: { x: number; y: number; w: number; h: number }
): number {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  if (w <= 0 || h <= 0) return 0;
  return (w * h) / Math.max(1e-6, Math.min(a.w * a.h, b.w * b.h));
}

/** Turn proposals into fields, as one edit. */
export function acceptProposals(tab: TabRecord, ids?: string[]) {
  const chosen = state.proposals.filter((p) => !ids || ids.includes(p.id));
  if (chosen.length === 0) return;
  const design = designOf(tab);
  const taken = namesOf(design);
  const fields = chosen.map((p) => fieldFromProposal(p, p.pageId, taken));
  commitDesign(tab, addFields(design, fields));
  const gone = new Set(chosen.map((p) => p.id));
  setState('proposals', (list) => list.filter((p) => !gone.has(p.id)));
}

export function rejectProposals(ids?: string[]) {
  if (!ids) {
    setState('proposals', []);
    return;
  }
  const gone = new Set(ids);
  setState('proposals', (list) => list.filter((p) => !gone.has(p.id)));
}

/** Test hook. */
export function resetPrepare() {
  setTabId(null);
  setState({
    tool: 'select',
    selected: [],
    showTabs: false,
    proposals: [],
    detecting: false,
    bases: {},
  });
}
