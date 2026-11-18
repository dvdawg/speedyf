/** Digital signatures in the command palette. Only a signed document offers
 * the command, so it never clutters the palette for the common case. */
import type { Command } from '../../app/commands';
import type { TabRecord } from '../../stores/tabsStore';
import { knownSignatures } from './signatureStore';

/** Show or hide the Signatures panel. It takes the place of the Form panel
 * rather than squeezing in beside it. */
export function toggleSignaturesPanel(tab: TabRecord): void {
  const open = !tab.viewport.state.signaturesPanelOpen;
  tab.viewport.setState('signaturesPanelOpen', open);
  if (open) tab.viewport.setState('formPanelOpen', false);
}

export const SIGNATURE_COMMANDS: Command[] = [
  {
    id: 'signatures.showPanel',
    title: 'Show Digital Signatures',
    group: 'Panels',
    keywords: ['signature', 'signed', 'certificate', 'verify', 'validate', 'trust'],
    enabled: (ctx) => {
      const doc = ctx.tab?.documentStore.state;
      return doc?.loaded === true && knownSignatures(doc.docId).length > 0;
    },
    run: (ctx) => {
      if (ctx.tab) toggleSignaturesPanel(ctx.tab);
    },
  },
];
