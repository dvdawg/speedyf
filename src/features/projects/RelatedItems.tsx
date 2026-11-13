/** Related: the few other items the citation graph ties an item to - works
 * both cite, items citing both - each with why in plain words. Rows look and
 * act like the Cites and Cited by rows; an empty answer shows nothing at all,
 * heading included, since "nothing related" is not worth a line.
 *
 * Computed by the engine offline, re-read when the item changes and on every
 * `library:changed`, sequence-guarded so a slow answer for the previous item
 * does not land on this one. */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { PaperRow, RelatedItem } from '../../types/engine';
import MathText from '../../components/MathText';

export default function RelatedItems(props: {
  paperId: string;
  onOpen: (row: PaperRow) => void;
  /** Wraps the rows: the panel's subhead, the detail's labelled section. */
  frame: (rows: JSX.Element) => JSX.Element;
}) {
  const [items, setItems] = createSignal<RelatedItem[]>([]);
  let seq = 0;
  let current = '';

  const load = (paperId: string) => {
    const mine = ++seq;
    engine.libraryRelatedItems(paperId).then(
      (list) => {
        if (mine === seq) setItems(list);
      },
      () => {
        // A nicety: no section is the quiet answer.
        if (mine === seq) setItems([]);
      }
    );
  };

  createEffect(
    on(
      () => props.paperId,
      (paperId) => {
        current = paperId;
        setItems([]);
        load(paperId);
      }
    )
  );

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void engine
      .onLibraryChanged(() => load(current))
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    onCleanup(() => {
      disposed = true;
      unlisten?.();
    });
  });

  return (
    <Show when={items().length > 0}>
      {props.frame(
        <For each={items()}>
          {(item) => (
            <button
              type="button"
              class="paper-panel-link-row"
              disabled={item.row.missing}
              title={item.row.missing ? 'Every file of this item is missing' : undefined}
              onClick={() => props.onOpen(item.row)}
            >
              <MathText class="paper-panel-link-title" text={item.row.title} />
              <span class="paper-panel-link-meta">{item.reason}</span>
            </button>
          )}
        </For>
      )}
    </Show>
  );
}
