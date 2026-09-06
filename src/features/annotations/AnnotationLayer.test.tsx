// @vitest-environment happy-dom
/**
 * That a shaded bubble survives dark mode.
 *
 * `invertColor.test.ts` covers the transform and `bubbles.test.ts` covers the
 * geometry; neither can see whether the layer actually paints the inverted
 * colour. It is worth checking on the element itself, because the failure is
 * silent: a near-black shading on a near-black page is still there in the DOM,
 * still in the saved file, and completely invisible to the reader.
 */
import { render, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import type { Annotation, PageEntry } from '../../types/model';
import { updateSettings } from '../../stores/settings';
import AnnotationLayer from './AnnotationLayer';
import { shadeRect } from './bubbles';
import { TabContext } from '../../app/TabContext';
import { createDocumentStore } from '../document/documentStore';
import type { TabRecord } from '../../stores/tabsStore';

/** The glyph box PDFium reports for a LaTeX \bigcirc in CMSY10. */
const glyphBox = { x: 72.61, y: 531.1, w: 10.63, h: 11.13 };

let documentStore: ReturnType<typeof createDocumentStore>;

/** A page carrying one shaded bubble, drawn in the tool's near-black ink. */
const mount = (color = '#1a1a1a') => {
  documentStore = createDocumentStore();
  documentStore.initFromMeta({
    docId: 1,
    path: '/tmp/quiz.pdf',
    name: 'quiz.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  const page = documentStore.state.pages[0] as PageEntry;
  const annot: Annotation = {
    id: 'an-1',
    pageId: page.id,
    kind: 'bubble',
    rect: shadeRect(glyphBox),
    color,
    opacity: 1,
    bubbleCharIndex: 12,
  };
  documentStore.apply({ type: 'addAnnot', annot });
  const tab = { id: 'tab-1', documentStore } as unknown as TabRecord;
  const geom = { widthPt: page.widthPt, heightPt: page.heightPt, rotation: 0 as const };
  const view = render(() => (
    <TabContext.Provider value={tab}>
      <AnnotationLayer page={page} geom={geom} zoom={1} runs={[]} />
    </TabContext.Provider>
  ));
  return view.container.querySelector('.annot-bubble') as SVGEllipseElement;
};

afterEach(() => {
  cleanup();
  updateSettings({ theme: 'system', invertPages: false });
});

describe('shaded bubbles under inverted pages', () => {
  it('paints the authored ink on an ordinary page', () => {
    updateSettings({ theme: 'light', invertPages: true });
    expect(mount().getAttribute('fill')).toBe('#1a1a1a');
  });

  // The engine sends the printed ring back as #bababa, so the fill inside it
  // has to arrive at the same grey — anything else reads as a smudge rather
  // than a shaded bubble.
  it('paints the inverted ink once the page itself is inverted', () => {
    updateSettings({ theme: 'dark', invertPages: true });
    expect(mount().getAttribute('fill')).toBe('#bababa');
  });

  it('leaves a coloured shading its own hue', () => {
    updateSettings({ theme: 'dark', invertPages: true });
    expect(mount('#c62828').getAttribute('fill')).toBe('#c73b36');
  });

  // The setting is toggled from a keystroke while the page is on screen, so
  // the fill has to be reactive rather than read once at mount — a shading
  // frozen at its authored colour is exactly the bug this fixes.
  it('repaints an already-drawn shading when the pages are inverted', () => {
    updateSettings({ theme: 'dark', invertPages: false });
    const shading = mount();
    expect(shading.getAttribute('fill')).toBe('#1a1a1a');
    updateSettings({ invertPages: true });
    expect(shading.getAttribute('fill')).toBe('#bababa');
    updateSettings({ invertPages: false });
    expect(shading.getAttribute('fill')).toBe('#1a1a1a');
  });

  // Dark mode alone is not the trigger: the page raster is only flipped when
  // the invert-pages preference is on, and a shading has to follow the raster.
  it('leaves the ink alone in dark mode with uninverted pages', () => {
    updateSettings({ theme: 'dark', invertPages: false });
    expect(mount().getAttribute('fill')).toBe('#1a1a1a');
  });
});
