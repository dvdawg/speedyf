/** The hover preview of a note's links: what a citation, a statement, a
 * cross-reference or a quoted paper's reference stands for, shown the way
 * the reader previews its own links - a picture of the place in the paper
 * (the paper need not be open: `paperDocs` opens it in the engine), or the
 * item's title, or the reference entry with its DOI or arXiv link.
 *
 * One popover for the window. A token asks for it on pointer enter or
 * keyboard focus, after a short dwell, and lets it go on leave or blur; the
 * pointer may move onto the popover and stay. */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { followExternalLink } from '../../../lib/links/followExternalLink';
import { positionPreviewPopover } from '../../citations/previewLayout';
import MathText from '../../../components/MathText';
import { placePreview, type Rect } from '../paperDocs';

export type PreviewContent =
  /** A place in a paper: its picture when the item and box are known,
   * with a caption under it. */
  | {
      kind: 'place';
      itemId: string | null;
      page: number | null;
      box: Rect | null;
      title: string;
      detail: string;
      previewKind?: string;
    }
  /** An item, by its title and authors. */
  | { kind: 'item'; title: string; detail: string }
  /** A reference the quoted paper printed, no item in the library. */
  | { kind: 'reference'; label: string; text: string };

interface Shown {
  content: PreviewContent;
  anchor: DOMRect;
}

const [shown, setShown] = createSignal<Shown | null>(null);
let showTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;

const DWELL_MS = 320;
const LEAVE_MS = 180;

/** A token wants its preview: shown after the dwell (at once when one is
 * already showing, as the reader's previews do). */
export function previewSoon(element: HTMLElement, content: PreviewContent): void {
  clearTimeout(hideTimer);
  clearTimeout(showTimer);
  const show = () => setShown({ content, anchor: element.getBoundingClientRect() });
  if (shown()) show();
  else showTimer = setTimeout(show, DWELL_MS);
}

/** The pointer or focus left a token (or the popover). */
export function previewLeave(): void {
  clearTimeout(showTimer);
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => setShown(null), LEAVE_MS);
}

export function previewClose(): void {
  clearTimeout(showTimer);
  clearTimeout(hideTimer);
  setShown(null);
}

/** For tests: what is showing. */
export function previewShown(): PreviewContent | null {
  return shown()?.content ?? null;
}

/** The DOIs and arXiv ids in a reference entry, as links. */
export function referenceLinks(text: string): { label: string; url: string }[] {
  const out: { label: string; url: string }[] = [];
  const doi = /\b(10\.\d{4,9}\/[^\s"<>]+[^\s"<>.,;)])/.exec(text);
  if (doi) out.push({ label: `doi:${doi[1]}`, url: `https://doi.org/${doi[1]}` });
  const arxiv =
    /arXiv:\s*(\d{4}\.\d{4,5}(?:v\d+)?|[a-z-]+(?:\.[A-Z]{2})?\/\d{7}(?:v\d+)?)/i.exec(text) ??
    /arxiv\.org\/abs\/(\d{4}\.\d{4,5}(?:v\d+)?)/i.exec(text);
  if (arxiv) out.push({ label: `arXiv:${arxiv[1]}`, url: `https://arxiv.org/abs/${arxiv[1]}` });
  return out;
}

function PlaceCard(props: { content: Extract<PreviewContent, { kind: 'place' }> }) {
  const [picture, setPicture] = createSignal<
    { url: string; width: number; height: number } | null | 'loading'
  >('loading');
  createEffect(() => {
    const c = props.content;
    if (!c.itemId || c.page === null || !c.box) {
      setPicture(null);
      return;
    }
    let live = true;
    setPicture('loading');
    void placePreview(c.itemId, c.page, c.box, c.previewKind).then((p) => {
      if (live) setPicture(p);
    });
    onCleanup(() => (live = false));
  });
  return (
    <div class="citation-preview-action is-note-preview">
      <Show when={picture() === 'loading'}>
        <div class="citation-preview-skeleton is-popover" aria-label="Loading preview" />
      </Show>
      <Show
        when={(() => {
          const p = picture();
          return p && p !== 'loading' ? p : null;
        })()}
      >
        {(p) => (
          <img
            class="citation-preview-image is-loaded note-preview-image"
            src={p().url}
            style={{ width: `${Math.min(p().width, 480)}px`, height: 'auto' }}
            alt={`Preview of ${props.content.title}`}
            draggable={false}
          />
        )}
      </Show>
      <div class="citation-preview-caption">
        <strong>
          <MathText text={props.content.title} />
        </strong>
        <Show when={props.content.detail}>
          <span>{props.content.detail}</span>
        </Show>
      </div>
    </div>
  );
}

/** Editors with a preview mounted, oldest first: only the oldest draws it. */
const [owners, setOwners] = createSignal<number[]>([]);
let nextOwner = 1;

/** The window's note preview. Every editor body mounts one; only the
 * oldest draws, so there is only ever one popover. */
export default function NotePreview() {
  const me = nextOwner++;
  setOwners((list) => [...list, me]);
  onCleanup(() => {
    setOwners((list) => list.filter((id) => id !== me));
    if (owners().length === 0) previewClose();
  });
  return (
    <Show when={owners()[0] === me ? shown() : null}>
      {(s) => {
        const wide = () => s().content.kind === 'place';
        const style = () =>
          positionPreviewPopover(
            {
              left: s().anchor.left,
              top: s().anchor.top,
              right: s().anchor.right,
              bottom: s().anchor.bottom,
              width: s().anchor.width,
              height: s().anchor.height,
            },
            { width: window.innerWidth, height: window.innerHeight },
            wide() ? 'internal' : 'default'
          );
        return (
          <Portal>
            <section
              class="citation-popover note-preview"
              classList={{ 'is-internal': wide() }}
              style={style()}
              role="dialog"
              aria-label="Link preview"
              onPointerEnter={() => clearTimeout(hideTimer)}
              onPointerLeave={previewLeave}
              onKeyDown={(e) => {
                if (e.key === 'Escape') previewClose();
              }}
            >
              <Show when={s().content.kind === 'place' ? s().content : null}>
                {(c) => <PlaceCard content={c() as Extract<PreviewContent, { kind: 'place' }>} />}
              </Show>
              <Show when={s().content.kind === 'item' ? s().content : null}>
                {(c) => {
                  const item = c() as Extract<PreviewContent, { kind: 'item' }>;
                  return (
                    <div class="citation-metadata-card note-preview-card">
                      <strong>
                        <MathText text={item.title} />
                      </strong>
                      <Show when={item.detail}>
                        <span>{item.detail}</span>
                      </Show>
                    </div>
                  );
                }}
              </Show>
              <Show when={s().content.kind === 'reference' ? s().content : null}>
                {(c) => {
                  const ref = c() as Extract<PreviewContent, { kind: 'reference' }>;
                  return (
                    <div class="citation-metadata-card note-preview-card">
                      <span class="note-preview-label">{ref.label}</span>
                      <span>
                        <MathText text={ref.text} />
                      </span>
                      <For each={referenceLinks(ref.text)}>
                        {(link) => (
                          <a
                            class="np-link"
                            href={link.url}
                            onClick={(e) => {
                              e.preventDefault();
                              void followExternalLink(link.url);
                            }}
                          >
                            {link.label}
                          </a>
                        )}
                      </For>
                      <span class="note-preview-note">Not in your library</span>
                    </div>
                  );
                }}
              </Show>
            </section>
          </Portal>
        );
      }}
    </Show>
  );
}
