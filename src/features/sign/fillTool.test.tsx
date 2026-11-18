// @vitest-environment happy-dom
/**
 * The Fill tool on a page: blanks outlined only while it is held, a click
 * snapping a text box onto a blank, Tab and Shift-Tab between blanks (and on
 * to the next page), free text away from any blank, marks, and the date
 * stamp. Detection itself is covered in src-tauri/src/engine/blanks.rs, the
 * geometry in fill.test.ts.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Blank } from '../../types/engine';
import type * as engineModule from '../../lib/transport/engine';

const blanksBySrc = vi.hoisted(() => new Map<number, Blank[]>());

vi.mock('../../lib/transport/engine', async (importOriginal) => {
  const actual = await importOriginal<typeof engineModule>();
  return {
    ...actual,
    engine: {
      ...actual.engine,
      getPageBlanks: (_docId: number, src: number) => Promise.resolve(blanksBySrc.get(src) ?? []),
    },
  };
});

import { TabContext } from '../../app/TabContext';
import Toolbar from '../../components/Toolbar';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../../stores/tabsStore';
import AnnotationLayer from '../annotations/AnnotationLayer';
import { setActiveTool, toolState } from '../annotations/toolStore';
import { resetFill, setFillMode, toggleFill } from './fillStore';
import { formatDate } from './fill';
import { settings, updateSettings } from '../../stores/settings';
import type { Annotation } from '../../types/model';

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const blank = (
  kind: Blank['kind'],
  x: number,
  y: number,
  w: number,
  h: number,
  cells = 0
): Blank => ({
  rect: { x, y, w, h },
  kind,
  fontSizePt: 9,
  cells,
});

/** Page 1: a name line, an empty box, a checkbox and a comb row. */
const PAGE_ONE = [
  blank('line', 120, 700, 200, 14),
  blank('box', 72, 600, 200, 24),
  blank('check', 72, 560, 10, 10),
  blank('comb', 120, 520, 126, 16, 9),
];
/** Page 2: one line. */
const PAGE_TWO = [blank('line', 100, 650, 200, 14)];

beforeEach(() => {
  blanksBySrc.clear();
  blanksBySrc.set(0, PAGE_ONE);
  blanksBySrc.set(1, PAGE_TWO);
  resetFill();
});

afterEach(() => {
  cleanup();
  setActiveTool('select');
  for (const tab of [...tabsStore.state.tabs]) removeTab(tab.id);
  setActiveId(null);
  vi.restoreAllMocks();
});

/** Both pages, laid out as two layers; page coordinates equal client
 * coordinates (y down from 792) on page 1. */
function mount() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 3,
    path: '/tmp/form.pdf',
    name: 'form.pdf',
    pageCount: 2,
    sizes: [
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
    ],
    estimatedSize: [612, 792],
  });
  addTab(tab);
  setActiveId(tab.id);
  const [one, two] = tab.documentStore.state.pages;
  const runs = [{ text: 'Name', start: 0, x: 72, y: 698, w: 40, h: 9 }];
  const view = render(() => (
    <>
      <Toolbar />
      <TabContext.Provider value={tab}>
        <div data-page="1">
          <AnnotationLayer
            page={one!}
            geom={{ widthPt: 612, heightPt: 792, rotation: 0 }}
            zoom={1}
            runs={runs}
          />
        </div>
        <div data-page="2">
          <AnnotationLayer
            page={two!}
            geom={{ widthPt: 612, heightPt: 792, rotation: 0 }}
            zoom={1}
            runs={[]}
          />
        </div>
      </TabContext.Provider>
    </>
  ));
  const layers = view.container.querySelectorAll('.annot-layer');
  for (const layer of layers) {
    vi.spyOn(layer, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 612, 792));
  }
  const annots = (index = 0): Annotation[] =>
    tab.documentStore.state.annotations[tab.documentStore.state.pages[index]!.id] ?? [];
  /** Click page 1 (or 2) at page coordinates. */
  const click = (x: number, y: number, page = 0) =>
    fireEvent.pointerDown(layers[page]!, { button: 0, clientX: x, clientY: 792 - y });
  const editor = () =>
    view.queryByRole('textbox', { name: 'Text box content' }) as HTMLTextAreaElement | null;
  return { ...view, tab, annots, click, editor, layers };
}

it('outlines blanks only while the Fill tool is held', async () => {
  const view = mount();
  await tick();
  expect(view.container.querySelectorAll('.fill-blank')).toHaveLength(0);
  toggleFill();
  await tick();
  await tick();
  expect(view.layers[0]!.querySelectorAll('.fill-blank')).toHaveLength(PAGE_ONE.length);
  expect(view.layers[1]!.querySelectorAll('.fill-blank')).toHaveLength(PAGE_TWO.length);
  expect(view.getByRole('toolbar', { name: 'Fill and sign' })).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Done' }));
  expect(toolState.active).toBe('select');
  expect(view.container.querySelectorAll('.fill-blank')).toHaveLength(0);
});

it('snaps typing onto a blank and tabs through the page and on to the next', async () => {
  const view = mount();
  toggleFill();
  await tick();
  await tick();

  // On the rule of the name line, at the blank's size, ready to type.
  view.click(200, 705);
  await tick();
  const first = view.annots()[0]!;
  expect(first.kind).toBe('textbox');
  expect(first.fontSizePt).toBe(9);
  const baseline = first.rect.y + first.rect.h - 9;
  expect(baseline).toBeGreaterThan(700);
  expect(baseline).toBeLessThan(702.5);
  expect(document.activeElement).toBe(view.editor());
  fireEvent.input(view.editor()!, { target: { value: 'Ada Lovelace' } });
  expect(toolState.active).toBe('fill');

  // Tab: the box, then (skipping the checkbox) the comb.
  fireEvent.keyDown(view.editor()!, { key: 'Tab' });
  await tick();
  expect(view.annots()).toHaveLength(2);
  const box = view.annots()[1]!;
  expect(box.rect.y).toBeGreaterThan(600);
  expect(box.rect.y + box.rect.h).toBeLessThan(625);
  fireEvent.keyDown(view.editor()!, { key: 'Tab' });
  await tick();
  // The box was left empty, so it went; the comb is open.
  expect(view.annots().map((a) => a.combCells ?? 0)).toEqual([0, 9]);
  fireEvent.input(view.editor()!, { target: { value: '123456789' } });

  // Shift-Tab goes back and makes the box again; Tab returns to the comb's
  // answer instead of stacking a second one.
  fireEvent.keyDown(view.editor()!, { key: 'Tab', shiftKey: true });
  await tick();
  expect(view.annots()).toHaveLength(3);
  fireEvent.keyDown(view.editor()!, { key: 'Tab' });
  await tick();
  expect(view.annots()).toHaveLength(2);
  expect(view.editor()!.value).toBe('123456789');

  // Past the last blank: on to page 2.
  const scroll = vi.spyOn(view.tab.viewport, 'requestScrollToPage');
  fireEvent.keyDown(view.editor()!, { key: 'Tab' });
  await tick();
  await tick();
  expect(scroll).toHaveBeenCalledWith(1, expect.any(Number));
  expect(view.annots(1)).toHaveLength(1);
  expect(view.annots(1)[0]!.rect.x).toBeCloseTo(102);

  // Everything saves as ordinary text, the comb with its cells.
  const plan = view.tab.documentStore.buildEditPlan();
  expect(plan.pages[0]!.texts.map((t) => [t.text, t.combCells])).toEqual([
    ['Ada Lovelace', undefined],
    ['123456789', 9],
  ]);
});

it('types where it is clicked away from any blank, at the size of the text there', async () => {
  const view = mount();
  toggleFill();
  await tick();
  await tick();
  view.click(400, 300);
  await tick();
  const free = view.annots()[0]!;
  expect(free.kind).toBe('textbox');
  expect(free.autoSize).toBe(true);
  expect(free.fontSizePt).toBe(10);
  expect(free.rect.x).toBe(400);
  fireEvent.input(view.editor()!, { target: { value: 'note' } });
  view.click(130, 702 - 0.5);
  await tick();
  // Next to the 9 pt label line, but in the line blank: the blank's size.
  expect(view.annots()[1]!.fontSizePt).toBe(9);
});

it('places marks, centred in a checkbox, and undoes them', async () => {
  const view = mount();
  toggleFill();
  await tick();
  await tick();
  // A plain click on a checkbox ticks it.
  view.click(76, 563);
  await tick();
  const tick1 = view.annots()[0]!;
  expect(tick1).toMatchObject({ kind: 'mark', mark: 'check' });
  expect(tick1.rect.x + tick1.rect.w / 2).toBeCloseTo(77);
  expect(tick1.rect.y + tick1.rect.h / 2).toBeCloseTo(565);

  setFillMode('cross');
  view.click(300, 200);
  setFillMode('circle');
  view.click(300, 250);
  fireEvent.click(view.getByRole('radio', { name: 'Dot' }));
  view.click(300, 300);
  fireEvent.click(view.getByRole('radio', { name: 'Strike line' }));
  view.click(300, 350);
  expect(view.annots().map((a) => a.mark)).toEqual(['check', 'cross', 'circle', 'dot', 'line']);
  const plan = view.tab.documentStore.buildEditPlan();
  expect(plan.pages[0]!.annots.map((a) => [a.kind, a.mark])).toEqual([
    ['mark', 'check'],
    ['mark', 'cross'],
    ['mark', 'circle'],
    ['mark', 'dot'],
    ['mark', 'line'],
  ]);
  view.tab.documentStore.undo();
  expect(view.annots()).toHaveLength(4);
});

it('stamps today in the format the setting names', async () => {
  const before = settings.dateFormat;
  updateSettings({ dateFormat: 'iso' });
  const view = mount();
  toggleFill('date');
  await tick();
  await tick();
  view.click(200, 705);
  await tick();
  const stamp = view.annots()[0]!;
  expect(stamp.text).toBe(formatDate(new Date(), 'iso'));
  expect(stamp.text).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  // A stamp, not a mode: the next click types again.
  view.click(400, 300);
  await tick();
  expect(view.annots()[1]!.text).toBe('');
  updateSettings({ dateFormat: before });
});

it('is in the Sign menu', async () => {
  const view = mount();
  fireEvent.click(view.getByRole('button', { name: 'Sign: signatures and filling printed forms' }));
  fireEvent.click(view.getByRole('menuitemcheckbox', { name: 'Fill printed form' }));
  expect(toolState.active).toBe('fill');
});
