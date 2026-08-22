/** A document's navigable structure, held per tab rather than per panel.
 *
 * The outline and the formal-environment list used to be fetched inside their
 * own components, which meant they existed only while their sidebar mode was
 * mounted. Nothing else could see them — in particular the command palette,
 * which wants to offer sections as jump targets without opening a panel first.
 *
 * The two halves are loaded on deliberately different terms:
 *
 * - The **outline** is one cheap call, so it is fetched as soon as a document
 *   opens. That is what lets the palette always offer sections rather than
 *   offering them only when you happen to have had the panel open.
 * - **Formal environments** are a whole-document text pass. Paying that on
 *   every open would make opening a large paper slower for a feature most
 *   documents cannot even supply, so it is loaded on demand — and the palette
 *   surfaces environments only once something else has paid for them.
 */
import { createStore } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import type { FormalEntry, OutlineNode } from '../../types/engine';

export interface StructureState {
  docId: number | null;
  outline: OutlineNode[];
  outlineLoading: boolean;
  formal: FormalEntry[];
  formalLoading: boolean;
  /** True once the environment pass has actually run for this document. */
  formalLoaded: boolean;
}

export interface StructureStore {
  get state(): StructureState;
  /** Point at a new document: clears everything and fetches the outline. */
  syncDocument(docId: number): void;
  /** Run the environment pass once, if it has not run for this document. */
  ensureFormal(): void;
  reset(): void;
}

const empty = (): StructureState => ({
  docId: null,
  outline: [],
  outlineLoading: false,
  formal: [],
  formalLoading: false,
  formalLoaded: false,
});

export function createStructureStore(): StructureStore {
  const [state, setState] = createStore<StructureState>(empty());

  /** Which document these responses are still wanted for.
   *
   * A plain variable rather than a read of `state.docId`: this asks "is this
   * answer stale", which is a question about identity, not a reactive
   * dependency. Reading the store from inside an awaited callback would also
   * be exactly the pattern `solid/reactivity` warns about, and it would be
   * right to — a tracked read there subscribes to nothing useful.
   */
  let current: number | null = null;

  return {
    get state() {
      return state;
    },

    syncDocument(docId: number): void {
      current = docId;
      setState({ ...empty(), docId, outlineLoading: true });
      void engine
        .getOutline(docId)
        .then((outline) => {
          // A second document can open while this is in flight; writing its
          // predecessor's outline over it would mislabel every row.
          if (current !== docId) return;
          setState({ outline, outlineLoading: false });
        })
        .catch(() => {
          if (current === docId) setState('outlineLoading', false);
        });
    },

    ensureFormal(): void {
      const docId = current;
      if (docId === null || state.formalLoaded || state.formalLoading) return;
      setState('formalLoading', true);
      void engine
        .getFormalEnvs(docId)
        .then((formal) => {
          if (current !== docId) return;
          setState({ formal, formalLoading: false, formalLoaded: true });
        })
        .catch(() => {
          // A document with no recoverable environments is the common case,
          // not an error; record it as done so it is not retried on every
          // panel open.
          if (current !== docId) return;
          setState({ formal: [], formalLoading: false, formalLoaded: true });
        });
    },

    reset(): void {
      current = null;
      setState(empty());
    },
  };
}

/** Outline rows in document order, with their nesting depth.
 *
 * The tree is what the sidebar draws; a flat list is what anything searching
 * it wants. Kept here so both readings come from one traversal.
 */
export function flattenOutline(
  nodes: readonly OutlineNode[],
  depth = 0
): { node: OutlineNode; depth: number }[] {
  const out: { node: OutlineNode; depth: number }[] = [];
  for (const node of nodes) {
    out.push({ node, depth });
    if (node.children.length > 0) out.push(...flattenOutline(node.children, depth + 1));
  }
  return out;
}
