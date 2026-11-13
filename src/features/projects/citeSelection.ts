/** Copy \cite{} for the rows selected in the Projects list.
 *
 * The keys come from the rows already in memory, so the clipboard write
 * happens inside the click or key press that asked for it: WebKit refuses a
 * write that lands after an engine round trip. The keys are pinned after the
 * write, as copying BibTeX pins them, because a key that changes under a
 * manuscript breaks every \cite in it (projects.md §2).
 *
 * The list registers what it has selected, in the order it shows the rows,
 * while it is mounted; the Library command reads it from here, and falls
 * back to the open document when nothing is registered or selected. */
import { engine } from '../../lib/transport/engine';

export interface CiteRow {
  id: string;
  citeKey: string;
}

/** `\cite{a,b}`: keys in the given order, each once, empty keys dropped.
 * Null when nothing is left to cite. */
export function citeCommand(rows: readonly CiteRow[]): string | null {
  const keys: string[] = [];
  for (const row of rows) {
    const key = row.citeKey.trim();
    if (key && !keys.includes(key)) keys.push(key);
  }
  return keys.length > 0 ? `\\cite{${keys.join(',')}}` : null;
}

/** Write the \cite{} for these rows, then pin their keys. Resolves false
 * when there was nothing to cite or the clipboard refused the write, and
 * then pins nothing: nothing was copied. A failed pin is quiet; the next
 * copy or .bib export pins again. */
export async function copyCite(rows: readonly CiteRow[]): Promise<boolean> {
  const text = citeCommand(rows);
  if (text === null) return false;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    return false;
  }
  const ids = Array.from(new Set(rows.map((row) => row.id)));
  try {
    await engine.libraryPinKeys(ids);
  } catch {
    // Nothing to tell: the text is on the clipboard either way.
  }
  return true;
}

interface Source {
  rows: () => CiteRow[];
  /** Tells the user a write the list asked for was refused. */
  failed: () => void;
}

let source: Source | null = null;

/** The list's selection, in its visible order, and how it reports a
 * refused write. Returns the unregister. */
export function registerCiteSelection(rows: () => CiteRow[], failed: () => void): () => void {
  const mine: Source = { rows, failed };
  source = mine;
  return () => {
    if (source === mine) source = null;
  };
}

/** What the list has selected now; empty when no list is showing. */
export function citeSelection(): CiteRow[] {
  return source?.rows() ?? [];
}

/** Copy \cite{} for the list's selection, reporting a refusal the way the
 * list does. False when there is no selection to copy, so the caller can
 * fall back to something else. */
export function copyListSelection(): boolean {
  const current = source;
  const rows = current?.rows() ?? [];
  if (!current || rows.length === 0) return false;
  void copyCite(rows).then((ok) => {
    if (!ok) current.failed();
  });
  return true;
}
