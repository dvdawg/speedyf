/** Formal-environment panel: theorems, lemmas, definitions and captions
 * recovered from a LaTeX-compiled document, click to jump.
 *
 * Deliberately built to read and behave like the table of contents next to it:
 * the same rows, the same indentation, the same navigation. Entries arrive in
 * document order under their section rather than bucketed by type — the order
 * results appear in is what you navigate by — and each label is the word the
 * document prints, not the LaTeX counter it is anchored on, so a lemma sharing
 * the theorem counter still reads "Lemma". See src-tauri/src/engine/formal.rs. */
import { For, onMount, Show, useContext } from 'solid-js';
import { TabContext } from '../../app/TabContext';
import { jumpToAnchor } from './jumpToAnchor';
import ScriptText from '../../components/ScriptText';

export default function FormalEnvs() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  // A whole-document pass, so it runs when this panel is actually opened
  // rather than on every document open. The result lives on the tab, which is
  // what lets the palette offer environments once they have been paid for.
  const structure = tab.structureStore.state;
  onMount(() => tab.structureStore.ensureFormal());

  return (
    <div class="sidebar-scroll outline-panel" aria-label="Formal environments">
      <Show
        when={structure.formalLoaded}
        fallback={<div class="panel-note">Reading environments…</div>}
      >
        <Show
          when={structure.formal.length > 0}
          fallback={<div class="panel-note">No formal environments extractable.</div>}
        >
          <For each={structure.formal}>
            {(entry) => (
              <div class="outline-entry">
                <button
                  type="button"
                  class="outline-row"
                  style={{ 'padding-left': `${10 + entry.depth * 14}px` }}
                  onClick={() => jumpToAnchor(tab.viewport, doc, entry.page, entry.y)}
                >
                  <ScriptText value={entry.label} />
                </button>
              </div>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}
