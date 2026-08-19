/** Formal-environment panel: theorems, lemmas, definitions and captions
 * recovered from a LaTeX-compiled document, click to jump.
 *
 * Deliberately built to read and behave like the table of contents next to it:
 * the same rows, the same indentation, the same navigation. Entries arrive in
 * document order under their section rather than bucketed by type — the order
 * results appear in is what you navigate by — and each label is the word the
 * document prints, not the LaTeX counter it is anchored on, so a lemma sharing
 * the theorem counter still reads "Lemma". See src-tauri/src/engine/formal.rs. */
import { createResource, For, Show, useContext } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { FormalEntry } from '../../types/engine';
import { TabContext } from '../../app/TabContext';
import { jumpToAnchor } from './jumpToAnchor';

export default function FormalEnvs() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  const [entries] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => engine.getFormalEnvs(docId).catch(() => [] as FormalEntry[])
  );

  return (
    <div class="sidebar-scroll outline-panel" aria-label="Formal environments">
      <Show when={!entries.loading} fallback={<div class="panel-note">Reading environments…</div>}>
        <Show
          when={(entries() ?? []).length > 0}
          fallback={<div class="panel-note">No formal environments extractable.</div>}
        >
          <For each={entries()}>
            {(entry) => (
              <div class="outline-entry">
                <button
                  type="button"
                  class="outline-row"
                  style={{ 'padding-left': `${10 + entry.depth * 14}px` }}
                  onClick={() => jumpToAnchor(tab.viewport, doc, entry.page, entry.y)}
                >
                  {entry.label}
                </button>
              </div>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
