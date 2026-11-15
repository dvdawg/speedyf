/** Markup in the palette and on the keyboard. Review mode on Cmd+Shift+M
 * (Ctrl+Shift+M); while it is on, C comments on the selected text, T
 * suggests a fix for it, and R marks a region. Every command needs the
 * Markup extension switched on and a document open. */
import type { Command, CommandCtx } from '../../app/commands';
import {
  beginComment,
  beginRegion,
  liveSelection,
  markupAvailable,
  reviewing,
  toggleReviewing,
} from './markupStore';
import { copyMarkupText, exportMarkupPdf } from './exportActions';

const REASON = 'needs the Markup extension switched on and a document open';
const REVIEW_REASON = 'needs review mode: Markup: Start Reviewing';

const docOpen = (ctx: CommandCtx) => ctx.tab?.documentStore.state.loaded === true;
const usable = (ctx: CommandCtx) => docOpen(ctx) && markupAvailable();

export const MARKUP_COMMANDS: Command[] = [
  {
    id: 'markup.toggle',
    // Says what pressing it would do now.
    get title() {
      return reviewing() ? 'Markup: Stop Reviewing' : 'Markup: Start Reviewing';
    },
    group: 'Tools',
    keywords: ['review', 'referee', 'feedback', 'comment', 'annotate', 'markup'],
    shortcut: 'mod+shift+m',
    enabled: usable,
    disabledReason: REASON,
    run: () => toggleReviewing(),
  },
  {
    id: 'markup.comment',
    title: 'Markup: Comment on Selection',
    group: 'Tools',
    keywords: ['review', 'note', 'remark'],
    shortcut: 'c',
    enabled: (ctx) => usable(ctx) && reviewing() && liveSelection(),
    disabledReason: REVIEW_REASON,
    run: () => void beginComment('comment'),
  },
  {
    id: 'markup.typo',
    title: 'Markup: Suggest a Fix for Selection',
    group: 'Tools',
    keywords: ['typo', 'wording', 'replace', 'review'],
    shortcut: 't',
    enabled: (ctx) => usable(ctx) && reviewing() && liveSelection(),
    disabledReason: REVIEW_REASON,
    run: () => void beginComment('typo'),
  },
  {
    id: 'markup.region',
    title: 'Markup: Comment on a Region',
    group: 'Tools',
    keywords: ['figure', 'equation', 'box', 'review'],
    shortcut: 'r',
    enabled: (ctx) => usable(ctx) && reviewing(),
    disabledReason: REVIEW_REASON,
    run: () => void beginRegion(),
  },
  {
    id: 'markup.export',
    title: 'Markup: Export PDF...',
    group: 'File',
    keywords: ['review', 'send', 'annotated', 'feedback', 'referee'],
    enabled: usable,
    disabledReason: REASON,
    run: () => void exportMarkupPdf(),
  },
  {
    id: 'markup.copyText',
    title: 'Markup: Copy Review as Text',
    group: 'File',
    keywords: ['review', 'letter', 'email', 'clipboard', 'plain'],
    enabled: usable,
    disabledReason: REASON,
    run: () => void copyMarkupText(),
  },
];
