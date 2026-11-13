/** Authors as people (projects.md sec. 2, "Authors as people"): a name in
 * the detail column or the reader's Library panel is a quiet link to the
 * person's items, and the middle column lists them under "Items by John
 * Smith" until Escape or the close button, like the graph's focus.
 *
 * Only an author the engine knows on other items is a link (`authorPersons`
 * on the record); the rest stay plain text, so a link always leads
 * somewhere with more than the item already open. */
import { For, Show } from 'solid-js';
import { IconClose } from '../../components/icons';
import type { Author, Paper } from '../../types/engine';
import { projectsStore } from './projectsStore';

/** "Family, Given", as the Authors field shows each line. */
const fieldName = (author: Author) =>
  author.given ? `${author.family}, ${author.given}` : author.family;

/** One author: a link when they are a person with other items. */
function AuthorName(props: {
  text: string;
  personId: string | null | undefined;
  onOpen?: (() => void) | undefined;
}) {
  return (
    <Show when={props.personId} fallback={<span>{props.text}</span>}>
      {(id) => (
        <button
          type="button"
          class="author-link"
          title="Show their items"
          onClick={() => {
            void projectsStore.showPerson(id());
            props.onOpen?.();
          }}
        >
          {props.text}
        </button>
      )}
    </Show>
  );
}

/** Whether any author of the record links anywhere. */
export function hasPersonLinks(paper: Paper): boolean {
  return (paper.authorPersons ?? []).some((id) => id !== null && id !== undefined);
}

/** The Authors field's value, one author a line, each a link where it can
 * be. */
export function AuthorLines(props: { paper: Paper }) {
  const authors = () => props.paper.meta.authors?.value ?? [];
  return (
    <span class="author-lines">
      <For each={authors()}>
        {(author, at) => (
          <span class="author-line">
            <AuthorName text={fieldName(author)} personId={props.paper.authorPersons?.[at()]} />
          </span>
        )}
      </For>
    </span>
  );
}

/** The panel's one-line authors ("A, B & C", "A et al."), with the names
 * shown as links where they can be. `onOpen` takes the reader to the
 * Projects screen. */
export function AuthorsInline(props: { paper: Paper; onOpen: () => void }) {
  const named = () =>
    (props.paper.meta.authors?.value ?? [])
      .map((author, at) => ({
        text: author.family.trim() || author.given.trim(),
        personId: props.paper.authorPersons?.[at] ?? null,
      }))
      .filter((name) => name.text.length > 0);
  const shown = () => (named().length > 3 ? named().slice(0, 1) : named());
  const separator = (at: number) => {
    const count = shown().length;
    if (at === 0) return '';
    return at === count - 1 ? ' & ' : ', ';
  };
  return (
    <>
      <For each={shown()}>
        {(name, at) => (
          <>
            {separator(at())}
            <AuthorName text={name.text} personId={name.personId} onOpen={props.onOpen} />
          </>
        )}
      </For>
      {named().length > 3 ? ' et al.' : ''}
    </>
  );
}

/** "Items by John Smith" over the middle column's list, with the other
 * forms the library prints the name in, and the way back. */
export function PersonBar() {
  const store = projectsStore;
  return (
    <Show when={store.state.person}>
      {(person) => (
        <div
          class="person-bar"
          onKeyDown={(e) => {
            if (e.key !== 'Escape' || e.defaultPrevented) return;
            e.preventDefault();
            store.leavePerson();
          }}
        >
          <div class="graph-focus-bar person-bar-head">
            <span class="graph-focus-title">
              Items by <span class="person-bar-name">{person().name}</span>
            </span>
            <button
              type="button"
              class="graph-focus-leave"
              aria-label="Leave the person"
              title="Back to the list (Escape)"
              onClick={() => store.leavePerson()}
            >
              <IconClose />
            </button>
          </div>
          <Show when={person().variants.length > 0}>
            <div class="person-bar-also">also as {person().variants.join(', ')}</div>
          </Show>
        </div>
      )}
    </Show>
  );
}
