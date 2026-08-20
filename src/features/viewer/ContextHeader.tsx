/** Sticky breadcrumb of which section the viewport currently sits in —
 * "2 Private Populous Estimator › 2.1 The PPE" — so a long paper stays
 * oriented without going to look at a sidebar, with the file it belongs to on
 * the right.
 *
 * Sections only. An environment would need an end to be shown honestly, and a
 * PDF gives no marker for where a theorem stops, so naming one here would
 * leave it standing over pages of ordinary prose. The formal-environment panel
 * is where those live.
 *
 * Reads the bookmark outline rather than the structure index: the index omits
 * headings that contain no environments, which is right for a list of theorems
 * and wrong for a breadcrumb, where a section with none would go unnamed and
 * leave the previous one showing. */
import { createMemo, createResource, Show, useContext } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { FormalEntry } from '../../types/engine';
import {
  anchorsForPosition,
  outlineAsEntries,
  pageSpaceY,
  READING_FOCUS,
  type StructureAnchors,
} from '../../lib/structure/context';
import { pageIndexAt } from '../../lib/coordinates/layout';
import { TabContext } from '../../app/TabContext';
import { jumpToAnchor } from '../outline/jumpToAnchor';
import ScriptText from '../../components/ScriptText';

const EMPTY: StructureAnchors = { section: null, subsection: null, environment: null };

export default function ContextHeader() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp, zoom } = tab;
  const doc = documentStore.state;

  const docKey = () => (doc.loaded ? doc.docId : null);
  const [outline] = createResource(docKey, (docId) => engine.getOutline(docId).catch(() => []));
  const entries = createMemo<FormalEntry[]>(() => outlineAsEntries(outline() ?? []));

  // Layout is memoized on zoom, not scroll: this recomputes on every scroll
  // frame and must not drag an O(pages) layout along with it.
  const layout = createMemo(() => zoom.layoutFor(vp.state.zoom));

  const here = createMemo<StructureAnchors>(() => {
    const list = entries();
    if (list.length === 0) return EMPTY;
    const l = layout();
    if (l.tops.length === 0) return EMPTY;

    // A selection is an explicit statement of what you are looking at, so it
    // wins over the scroll position while it lasts.
    const selected = vp.state.selectionAnchor;
    const page = selected
      ? selected.page
      : pageIndexAt(l, vp.state.scrollTop + vp.state.containerH * READING_FOCUS);
    const geom = doc.pages[page];
    const pageTop = l.tops[page];
    if (!geom || pageTop === undefined) return EMPTY;

    // Anchors are keyed by source page, which is not the position in the tab
    // once pages have been reordered; a blank page has no source at all.
    const src = geom.srcIndex;
    if (src === null) return EMPTY;

    if (selected) return anchorsForPosition(list, src, selected.y);

    // What is being read, not what is at the top edge — headings can be close
    // together, so the top edge names what you have scrolled past.
    const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
    const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
    const focus = vp.state.scrollTop + vp.state.containerH * READING_FOCUS;
    return anchorsForPosition(list, src, pageSpaceY(focus, pageTop, vp.state.zoom, heightPt));
  });

  /** Home-relative and elided in the middle, since the useful part of a path
   * is its tail. Shortened here rather than by CSS: `direction: rtl` truncates
   * from the correct end but bidi-reorders the string, which is what put the
   * leading "~/" at the far right. */
  const shownPath = createMemo(() => {
    const path = doc.path;
    if (!path) return null;
    const home = path.match(/^(\/Users\/[^/]+|\/home\/[^/]+)\//)?.[1];
    const relative = home ? `~/${path.slice(home.length + 1)}` : path;
    const parts = relative.split('/');
    if (parts.length <= 3) return relative;
    return [parts[0], '…', ...parts.slice(-2)].join('/');
  });

  /** Takes accessors, not values. A non-keyed `<Show>` re-runs its callback
   * only when the condition flips truthy/falsy, so a crumb built from a plain
   * entry captures that entry's label as a static string and never updates —
   * which is why the section used to be stuck on whichever one loaded first.
   * Reading through the accessor inside the JSX keeps the binding live. */
  const crumb = (entry: () => FormalEntry, deepest: () => boolean) => (
    <button
      type="button"
      class="context-crumb"
      classList={{ 'is-deepest': deepest() }}
      onClick={() => {
        const target = entry();
        jumpToAnchor(vp, doc, target.page, target.y);
      }}
    >
      <ScriptText value={entry().label} />
    </button>
  );

  return (
    <Show when={here().section || here().subsection || shownPath()}>
      <div class="context-header">
        <div class="context-trail">
          <Show when={here().section}>
            {(section) => crumb(section, () => here().subsection === null)}
          </Show>
          <Show when={here().section && here().subsection}>
            <span class="context-sep" aria-hidden="true">
              ›
            </span>
          </Show>
          <Show when={here().subsection}>{(subsection) => crumb(subsection, () => true)}</Show>
        </div>
        <Show when={shownPath()}>
          {(path) => (
            <span class="context-path" title={doc.path ?? undefined}>
              {path()}
            </span>
          )}
        </Show>
      </div>
    </Show>
  );
}
