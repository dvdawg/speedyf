/** Visual signatures in the command palette. */
import type { Command, CommandCtx } from '../../app/commands';
import { activeTab } from '../../stores/tabsStore';
import {
  armPlacement,
  defaultSignature,
  loadSignatures,
  openSignDialog,
  setSignMenuOpen,
  toggleFlattenSignatures,
} from './signStore';
import type { SignatureRole } from './art';
import { fillActive, toggleFill, type FillMode } from './fillStore';

const docOpen = (ctx: CommandCtx) => ctx.tab?.documentStore.state.loaded === true;

/** Place the newest saved one; with none saved yet, make one first. */
async function placeDefault(role: SignatureRole) {
  await loadSignatures();
  const saved = defaultSignature(role);
  if (saved) armPlacement(saved);
  else openSignDialog(role);
}

const KEYWORDS = ['sign', 'signature', 'autograph', 'visual signature', 'fill and sign'];

const FILL_KEYWORDS = ['fill', 'form', 'printed', 'flat', 'type on', 'fill and sign'];

const fillMode = (id: string, title: string, mode: FillMode, keywords: string[]): Command => ({
  id,
  title,
  group: 'Tools',
  keywords: [...keywords, ...FILL_KEYWORDS],
  enabled: docOpen,
  run: () => toggleFill(mode),
});

export const SIGN_COMMANDS: Command[] = [
  {
    id: 'sign.fill',
    get title() {
      return fillActive() ? 'Fill & Sign: Stop Filling' : 'Fill & Sign: Fill Printed Form';
    },
    group: 'Tools',
    keywords: ['blank', 'underline', 'type', ...FILL_KEYWORDS],
    enabled: docOpen,
    run: () => toggleFill('text'),
  },
  fillMode('sign.fillCheck', 'Fill & Sign: Place Check Mark', 'check', ['tick', 'checkbox']),
  fillMode('sign.fillCross', 'Fill & Sign: Place Cross', 'cross', ['x', 'mark']),
  fillMode('sign.fillDot', 'Fill & Sign: Place Dot', 'dot', ['bullet', 'mark']),
  fillMode('sign.fillLine', 'Fill & Sign: Place Strike Line', 'line', ['strike', 'cross out']),
  fillMode('sign.fillCircle', 'Fill & Sign: Place Circle', 'circle', ['ring', 'circle answer']),
  fillMode('sign.fillDate', "Fill & Sign: Place Today's Date", 'date', ['today', 'date stamp']),
  {
    id: 'sign.place',
    title: 'Sign: Place Signature',
    group: 'Tools',
    keywords: KEYWORDS,
    enabled: docOpen,
    run: () => void placeDefault('signature'),
  },
  {
    id: 'sign.placeInitials',
    title: 'Sign: Place Initials',
    group: 'Tools',
    keywords: ['initial', ...KEYWORDS],
    enabled: docOpen,
    run: () => void placeDefault('initials'),
  },
  {
    id: 'sign.addSignature',
    title: 'Sign: Add Signature...',
    group: 'Tools',
    keywords: ['new', 'draw', 'type', 'image', ...KEYWORDS],
    enabled: docOpen,
    run: () => openSignDialog('signature'),
  },
  {
    id: 'sign.addInitials',
    title: 'Sign: Add Initials...',
    group: 'Tools',
    keywords: ['new', 'initial', ...KEYWORDS],
    enabled: docOpen,
    run: () => openSignDialog('initials'),
  },
  {
    id: 'sign.menu',
    title: 'Sign: Show Saved Signatures',
    group: 'Tools',
    keywords: KEYWORDS,
    enabled: docOpen,
    run: () => setSignMenuOpen(true),
  },
  {
    id: 'sign.flatten',
    // Says what running it would do now.
    get title() {
      return activeTab()?.documentStore.state.flattenSignatures
        ? 'Sign: Keep Signatures Movable on Save'
        : 'Sign: Flatten Signatures on Save';
    },
    group: 'Tools',
    keywords: ['flatten', 'merge', 'lock', ...KEYWORDS],
    enabled: docOpen,
    run: (ctx) => toggleFlattenSignatures(ctx.tab?.documentStore),
  },
];
