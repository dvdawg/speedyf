/** Lines extensions add to the Projects item detail column
 * (`itemDetailLines`): one quiet line each, such as "Mentioned in notepad
 * (3)". The extensions are asked when the item or the scope changes, and
 * again shortly after any note changes; a disabled extension's line goes
 * with it. */
import { createEffect, For, on, onCleanup, Show } from 'solid-js';
import { docEventSeq, itemLines } from './editorHost';
import { extensionUsable, requestItemLines, runItemLineAction } from './extensionStore';

export default function ItemLines(props: {
  itemId: string;
  citeKey: string;
  keyHistory: string[];
  projectIds: string[];
  /** The project selected on the Projects screen, if one is. */
  projectId: string | null;
}) {
  const ask = () =>
    void requestItemLines({
      item: {
        id: props.itemId,
        citeKey: props.citeKey,
        keyHistory: [...props.keyHistory],
        projectIds: [...props.projectIds],
      },
      projectId: props.projectId,
    });
  createEffect(
    on(
      () => [props.itemId, props.citeKey, props.projectId, props.projectIds.join(',')],
      () => ask()
    )
  );
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(
    on(
      docEventSeq,
      () => {
        clearTimeout(timer);
        timer = setTimeout(ask, 1_000);
      },
      { defer: true }
    )
  );
  onCleanup(() => clearTimeout(timer));
  const lines = () => itemLines(props.itemId, extensionUsable);
  return (
    <Show when={lines().length > 0}>
      <div class="item-lines">
        <For each={lines()}>
          {(line) => (
            <Show when={line.command} fallback={<span class="item-line">{line.text}</span>}>
              <button
                type="button"
                class="item-line is-action"
                onClick={() => void runItemLineAction(line)}
              >
                {line.text}
              </button>
            </Show>
          )}
        </For>
      </div>
    </Show>
  );
}
