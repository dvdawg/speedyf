/** Prepare form mode and form data in the command palette. */
import type { Command, CommandCtx } from '../../app/commands';
import type { TabRecord } from '../../stores/tabsStore';
import { exportFormData, importFormData } from '../forms/formData';
import {
  designOf,
  detectFields,
  preparing,
  setFlatten,
  startPreparing,
  togglePreparing,
} from './prepareStore';

/** Switch flatten-on-save, entering the mode first so the switch is seen. */
async function toggleFlatten(tab: TabRecord | null) {
  if (!tab) return;
  if (!preparing(tab)) await startPreparing(tab);
  if (!preparing(tab)) return;
  await setFlatten(tab, !designOf(tab).flatten);
}

const docOpen = (ctx: CommandCtx) => ctx.tab?.documentStore.state.loaded === true;

const KEYWORDS = ['form', 'fields', 'acrobat', 'fillable', 'create form', 'prepare'];

export const PREPARE_COMMANDS: Command[] = [
  {
    id: 'forms.prepare',
    // Says what running it would do now.
    get title() {
      return preparing() ? 'Form: Done Preparing' : 'Form: Prepare Form';
    },
    group: 'Tools',
    keywords: [...KEYWORDS, 'add field', 'text field', 'checkbox', 'radio', 'dropdown'],
    enabled: docOpen,
    run: (ctx) => togglePreparing(ctx.tab),
  },
  {
    id: 'forms.autodetect',
    title: 'Form: Auto-Detect Fields',
    group: 'Tools',
    keywords: [...KEYWORDS, 'detect', 'blanks', 'automatic', 'recognise'],
    enabled: docOpen,
    run: (ctx) => void detectFields(ctx.tab),
  },
  {
    id: 'forms.flatten',
    title: 'Form: Flatten Form on Save',
    group: 'Tools',
    keywords: [...KEYWORDS, 'flatten', 'merge', 'lock', 'remove fields'],
    enabled: docOpen,
    run: (ctx) => void toggleFlatten(ctx.tab),
  },
  {
    id: 'forms.exportData',
    title: 'Form: Export Data...',
    group: 'File',
    keywords: ['fdf', 'xfdf', 'json', 'csv', 'form', 'values', 'save data'],
    enabled: docOpen,
    run: (ctx) => void exportFormData(ctx.tab),
  },
  {
    id: 'forms.importData',
    title: 'Form: Import Data...',
    group: 'File',
    keywords: ['fdf', 'xfdf', 'json', 'csv', 'form', 'values', 'load data', 'fill'],
    enabled: docOpen,
    run: (ctx) => void importFormData(ctx.tab),
  },
];
