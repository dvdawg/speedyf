/** One quiet line under an item's project checkboxes: the project the
 * citation graph ties it to most, if any, with Add and Not now.
 *
 * Never files anything by itself. Not now is per item and project, for the
 * session: the next suggestion for the item, if there is one, takes its
 * place. The suggestions are re-read when the item changes and on every
 * `library:changed`, sequence-guarded so a slow answer for the previous item
 * does not land on this one. */
import { createEffect, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { ProjectSuggestion } from '../../types/engine';
import { projectsStore } from './projectsStore';
import { dismissSuggestion, suggestionDismissed } from './suggestionDismissals';

export { resetDismissedSuggestions } from './suggestionDismissals';

export default function ProjectSuggestionLine(props: { paperId: string; class?: string }) {
  const [suggestions, setSuggestions] = createSignal<ProjectSuggestion[]>([]);
  /** Projects added from this line, hidden before the refresh says so. */
  const [added, setAdded] = createSignal<ReadonlySet<string>>(new Set());
  let seq = 0;
  /** The item shown, for the change event to re-read. */
  let current = '';

  const load = (paperId: string) => {
    const mine = ++seq;
    engine.librarySuggestProjects(paperId).then(
      (list) => {
        if (mine === seq) setSuggestions(list);
      },
      () => {
        // A suggestion is a nicety: no line is the quiet answer.
        if (mine === seq) setSuggestions([]);
      }
    );
  };

  createEffect(
    on(
      () => props.paperId,
      (paperId) => {
        current = paperId;
        setSuggestions([]);
        setAdded(new Set<string>());
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

  const top = () =>
    suggestions().find(
      (s) => !suggestionDismissed(props.paperId, s.projectId) && !added().has(s.projectId)
    ) ?? null;

  const add = (suggestion: ProjectSuggestion) => {
    setAdded((ids) => new Set([...ids, suggestion.projectId]));
    void projectsStore.setMembership(props.paperId, suggestion.projectId, true);
  };
  const notNow = (suggestion: ProjectSuggestion) =>
    dismissSuggestion(props.paperId, suggestion.projectId);

  return (
    <Show when={top()}>
      {(s) => (
        <div class={`project-suggestion ${props.class ?? ''}`} role="status">
          <span class="project-suggestion-text">
            Add to {s().name}? ({s().reason})
          </span>
          <button type="button" class="paper-mini-btn" onClick={() => add(s())}>
            Add
          </button>
          <button type="button" class="paper-mini-btn" onClick={() => notNow(s())}>
            Not now
          </button>
        </div>
      )}
    </Show>
  );
}
