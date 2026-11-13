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
 * - A PDF with no outline of its own, or a trivial one (`outlineIsTrivial`),
 *   gets one **generated** from its headings instead: the engine reads the
 *   pages at idle priority, a slice per call, and this store calls again
 *   until it is done. A big document's provisional outline shows as it
 *   grows. When nothing confident is found the document keeps the outline
 *   it had, which is usually none.
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
  /** The outline shown was generated from the page headings, not read from
   * the PDF. */
  outlineGenerated: boolean;
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
  outlineGenerated: false,
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

  /** Ask for the generated outline a slice at a time until it is done. Each
   * call is one idle job on the engine thread, so rendering always goes
   * first; stops as soon as another document takes this tab. */
  const generate = (docId: number, original: OutlineNode[]): void => {
    // Whether a provisional outline is showing - a plain variable, for the
    // same reason as `current`.
    let shown = false;
    const step = (): void => {
      void engine
        .getGeneratedOutline(docId)
        .then((result) => {
          if (current !== docId) return;
          if (result.outline.length > 0) {
            shown = true;
            setState({ outline: result.outline, outlineGenerated: true });
          } else if (result.done && shown) {
            // A provisional outline the full document did not bear out.
            shown = false;
            setState({ outline: original, outlineGenerated: false });
          }
          if (!result.done) setTimeout(step, 0);
        })
        .catch(() => {
          // The document closed, or the engine has nothing to offer; the
          // outline it had stands.
        });
    };
    step();
  };

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
          if (outlineIsTrivial(outline)) generate(docId, outline);
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

/** Whether a PDF's own outline is too thin to navigate by: none at all, a
 * single entry, or entries that all point at the first page (a bookmark for
 * the title and nothing else). Such a document gets a generated outline. */
export function outlineIsTrivial(nodes: readonly OutlineNode[]): boolean {
  const rows = flattenOutline(nodes);
  if (rows.length <= 1) return true;
  return rows.every(({ node }) => node.page === null || node.page === 0);
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
