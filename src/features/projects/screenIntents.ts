/** How commands outside the Projects screen reach state that lives inside
 * its components: the list's multi-selection, the graph's view, the new
 * project field, the detail column's lookup results.
 *
 * Two shapes. A registration is a handler a mounted component leaves here
 * and takes back on cleanup, read on demand like `citeSelection`: it exists
 * only while there is something on screen to act on. A request is a signal
 * a component consumes when it is (or next becomes) mounted, so a command
 * can switch to the screen and ask in the same breath. */
import { createSignal } from 'solid-js';

/* ---- registrations ---- */

export interface GraphControls {
  fit: () => void;
  resetLayout: () => void;
  hasMoved: () => boolean;
}

let graph: GraphControls | null = null;
let merge: (() => void) | null = null;

/** Returns the unregister. */
export function registerGraphControls(controls: GraphControls): () => void {
  graph = controls;
  return () => {
    if (graph === controls) graph = null;
  };
}

/** The graph on screen, if one is. */
export function graphControls(): GraphControls | null {
  return graph;
}

/** The list's way into its merge confirmation for the multi-selection.
 * Returns the unregister. */
export function registerMergeRequest(begin: () => void): () => void {
  merge = begin;
  return () => {
    if (merge === begin) merge = null;
  };
}

/** Shows the list's inline merge confirmation. Never merges by itself:
 * false when no list is there to ask. */
export function requestMerge(): boolean {
  if (!merge) return false;
  merge();
  return true;
}

/* ---- requests ---- */

const [newProject, setNewProject] = createSignal(false);
const [titleLookup, setTitleLookup] = createSignal<string | null>(null);

/** Asks the left column to open its new-project field. */
export function requestNewProject(): void {
  setNewProject(true);
}

/** Whether a new-project field was asked for; reading it with `take`
 * consumes the request. */
export function newProjectRequested(): boolean {
  return newProject();
}

export function takeNewProjectRequest(): void {
  setNewProject(false);
}

/** Asks the detail column to search by title for this item, once it shows
 * it, so the matches appear where they can be picked. */
export function requestTitleLookup(paperId: string): void {
  setTitleLookup(paperId);
}

export function titleLookupRequested(): string | null {
  return titleLookup();
}

export function takeTitleLookupRequest(): void {
  setTitleLookup(null);
}

const [details, setDetails] = createSignal(false);

/** Asks a narrow screen to unfold its detail column (where a project's
 * notepad shows), now or when the screen next mounts. A wide one shows it
 * anyway and just consumes the request. */
export function requestDetails(): void {
  setDetails(true);
}

export function detailsRequested(): boolean {
  return details();
}

export function takeDetailsRequest(): void {
  setDetails(false);
}
