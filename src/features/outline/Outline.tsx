/** Table-of-contents panel: the PDF's bookmark tree, click to jump to a page.
 * Alternate sidebar mode to PageThumbnails, toggled from Sidebar.tsx. */
import { createResource, For, Show, useContext } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { OutlineNode } from '../../types/engine';
import { TabContext } from '../../app/TabContext';

function OutlineEntry(props: {
  node: OutlineNode;
  depth: number;
  onNavigate: (page: number) => void;
}) {
  return (
    <div class="outline-entry">
      <button
        type="button"
        class="outline-row"
        style={{ 'padding-left': `${10 + props.depth * 14}px` }}
        disabled={props.node.page === null}
        onClick={() => {
          if (props.node.page !== null) props.onNavigate(props.node.page);
        }}
      >
        {props.node.title.trim() || 'Untitled'}
      </button>
      <Show when={props.node.children.length > 0}>
        <For each={props.node.children}>
          {(child) => (
            <OutlineEntry node={child} depth={props.depth + 1} onNavigate={props.onNavigate} />
          )}
        </For>
      </Show>
    </div>
  );
}

export default function Outline() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  const [nodes] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => engine.getOutline(docId).catch(() => [] as OutlineNode[])
  );

  const navigate = (page: number) => tab.viewport.requestScrollToPage(page);

  return (
    <div class="sidebar-scroll outline-panel" aria-label="Table of contents">
      <Show when={!nodes.loading} fallback={<div class="panel-note">Loading outline…</div>}>
        <Show
          when={(nodes() ?? []).length > 0}
          fallback={<div class="panel-note">This document has no table of contents.</div>}
        >
          <For each={nodes()}>
            {(node) => <OutlineEntry node={node} depth={0} onNavigate={navigate} />}
          </For>
        </Show>
      </Show>
    </div>
  );
}
