/** Redaction and hidden information in the command palette. */
import type { Command, CommandCtx } from '../../../app/commands';
import { hasSelection } from '../../../stores/selectionStore';
import { setActiveTool, toolState } from '../../annotations/toolStore';
import { openSanitize } from '../sanitize/sanitizeStore';
import { markCount } from './redactMarks';
import { applyRedactions, cancelApply, markSelection, openRedactPanel } from './redactStore';

const docOpen = (ctx: CommandCtx) => ctx.tab?.documentStore.state.loaded === true;

const KEYWORDS = ['redact', 'redaction', 'black out', 'censor', 'remove', 'privacy'];

export const REDACT_COMMANDS: Command[] = [
  {
    id: 'redact.tool',
    title: 'Redact: Mark for Redaction',
    group: 'Tools',
    keywords: ['tool', ...KEYWORDS],
    enabled: docOpen,
    run: () => setActiveTool(toolState.active === 'redact' ? 'select' : 'redact'),
  },
  {
    id: 'redact.selection',
    title: 'Redact: Mark Selected Text',
    group: 'Tools',
    keywords: ['selection', ...KEYWORDS],
    enabled: (ctx) => docOpen(ctx) && hasSelection(),
    disabledReason: 'select some text first',
    run: (ctx) => void markSelection(ctx.tab ?? null),
  },
  {
    id: 'redact.find',
    title: 'Redact: Find and Redact...',
    group: 'Tools',
    keywords: ['search', 'email', 'phone', 'ssn', 'credit card', 'date', ...KEYWORDS],
    enabled: docOpen,
    run: () => openRedactPanel(),
  },
  {
    id: 'redact.apply',
    get title() {
      return 'Redact: Apply Redactions...';
    },
    group: 'Tools',
    keywords: ['apply', ...KEYWORDS],
    enabled: (ctx) =>
      docOpen(ctx) &&
      !ctx.tab!.documentStore.state.scrub.applyRedactions &&
      markCount(ctx.tab!.documentStore) > 0,
    disabledReason: 'no redaction marks to apply',
    run: (ctx) => void applyRedactions(ctx.tab ?? null),
  },
  {
    id: 'redact.cancelApply',
    title: 'Redact: Do Not Apply Redactions',
    group: 'Tools',
    keywords: ['cancel', ...KEYWORDS],
    enabled: (ctx) => ctx.tab?.documentStore.state.scrub.applyRedactions === true,
    run: (ctx) => cancelApply(ctx.tab?.documentStore),
  },
  {
    id: 'protect.removeHidden',
    title: 'Remove Hidden Information...',
    group: 'Tools',
    keywords: ['sanitize', 'metadata', 'attachments', 'javascript', 'hidden', 'clean', 'privacy'],
    enabled: docOpen,
    run: (ctx) => void openSanitize(ctx.tab ?? null),
  },
];
