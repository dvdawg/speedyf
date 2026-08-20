import { createMemo, createResource, For, useContext } from 'solid-js';
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

  return (
    <div
      class="citation-layer"
      classList={{ 'is-disabled': toolState.active !== 'select' }}
      aria-label="Document links"
    >
      <For each={links()}>
        {(link) => {
          const [x, y, w, h] = link.rect;
          let pointerFocus = false;
          const enter = (element: HTMLButtonElement, keyboard: boolean) =>
            citationStore.enter(props.docId, props.src, link.target, element, keyboard);
          return (
            <button
              type="button"
              class="citation-hotspot"
              style={{
                left: `${x * props.zoom}px`,
                top: `${(props.pageHeightPt - y - h) * props.zoom}px`,
                width: `${Math.max(2, w * props.zoom)}px`,
                height: `${Math.max(2, h * props.zoom)}px`,
              }}
              aria-label={ariaLabel(link.target)}
              onPointerEnter={(event) => enter(event.currentTarget, false)}
              onPointerLeave={() => citationStore.leave()}
              onPointerDown={() => {
                pointerFocus = true;
                queueMicrotask(() => {
                  pointerFocus = false;
                });
              }}
              onFocus={(event) => {
                if (!pointerFocus) enter(event.currentTarget, true);
              }}
              onBlur={() => citationStore.leave()}
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
