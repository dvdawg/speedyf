import { createMemo, createResource, createSignal, For, useContext } from 'solid-js';
import type { LinkTarget, PageLink } from '../../types/engine';
import { followExternalLink } from '../../lib/links/followExternalLink';
import { toolState } from '../annotations/toolStore';
import { citationLabel } from './citationLabel';
import { TabContext } from '../../app/TabContext';

interface Props {
  docId: number;
  src: number;
  pageHeightPt: number;
  zoom: number;
}

type HoverableLink = PageLink & {
  target: Exclude<LinkTarget, { kind: 'unknown' }>;
};

function isHoverable(link: PageLink): link is HoverableLink {
  return link.target.kind !== 'unknown';
}

function ariaLabel(target: HoverableLink['target']): string {
  if (target.kind === 'internal') return `Preview section on page ${target.page + 1}`;
  if (target.citation) return `Preview citation ${citationLabel(target.citation)}`;
  return 'Preview external link';
}

export default function CitationLayer(props: Props) {
  const { citationStore } = useContext(TabContext)!;
  const [pageLinks] = createResource(
    () => [props.docId, props.src] as const,
    ([docId, src]) => citationStore.linksForPage(docId, src)
  );
  const links = createMemo(() => pageLinks()?.links.filter(isHoverable) ?? []);

  /** Which link the pointer is on, by destination rather than by rectangle.
   *
   * One link wrapped across a line is several rectangles, and highlighting only
   * the piece under the cursor makes a reference look like two separate links.
   * Keyed by destination so every rectangle of the same link lights together. */
  const [hovered, setHovered] = createSignal<string | null>(null);
  const groupKey = (target: PageLink['target']) =>
    target.kind === 'uri'
      ? `uri:${target.uri}`
      : target.kind === 'internal'
        ? `page:${target.page}:${target.x ?? ''}:${target.y ?? ''}`
        : 'unknown';

  return (
    <div
      class="citation-layer"
      classList={{ 'is-disabled': toolState.active !== 'select' }}
      aria-label="Document links"
    >
      <For each={links()}>
        {(link) => {
          const [x, y, w, h] = link.rect;
          const key = groupKey(link.target);
          let pointerFocus = false;
          const enter = (element: HTMLButtonElement, keyboard: boolean) => {
            setHovered(key);
            citationStore.enter(props.docId, props.src, link.target, element, keyboard);
          };
          const leave = () => {
            // Only the link that set it may clear it, or moving between two
            // rectangles of the same link would flicker the highlight off.
            if (hovered() === key) setHovered(null);
            citationStore.leave();
          };
          return (
            <button
              type="button"
              class="citation-hotspot"
              classList={{ 'is-linked': hovered() === key }}
              style={{
                left: `${x * props.zoom}px`,
                top: `${(props.pageHeightPt - y - h) * props.zoom}px`,
                width: `${Math.max(2, w * props.zoom)}px`,
                height: `${Math.max(2, h * props.zoom)}px`,
              }}
              aria-label={ariaLabel(link.target)}
              onPointerEnter={(event) => enter(event.currentTarget, false)}
              onPointerLeave={leave}
              onPointerDown={() => {
                pointerFocus = true;
                queueMicrotask(() => {
                  pointerFocus = false;
                });
              }}
              onFocus={(event) => {
                if (!pointerFocus) enter(event.currentTarget, true);
              }}
              onBlur={leave}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  event.preventDefault();
                  citationStore.close();
                  event.currentTarget.blur();
                }
              }}
              onClick={(event) => {
                event.preventDefault();
                if (link.target.kind === 'internal') {
                  citationStore.navigateInternalTarget(link.target);
                  citationStore.close();
                } else if (link.target.kind === 'uri') {
                  citationStore.close();
                  void followExternalLink(link.target.uri);
                }
              }}
            />
          );
        }}
      </For>
    </div>
  );
}
