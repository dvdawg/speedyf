/** A page's review marks while reviewing: a soft wash under each commented
 * passage in its kind's colour, a strike through words a typo fix
 * replaces, a thin box round a region, and the comment's number in the
 * page's right margin, level with it. The wash lets text selection
 * through; the number opens the comment in the drawer.
 *
 * Lives in the page's unrotated page space, like the annotation layer, so
 * it follows zoom and rotation without arithmetic of its own. */
import { createMemo, For, Show, useContext } from 'solid-js';
import { TabContext } from '../../app/TabContext';
import { anchorTopFromTop, numberComments, quadRect } from './geometry';
import { draft, editComment, flashCard, reviewFor, reviewing } from './markupStore';
import type { Anchor, CommentKind } from './types';

interface Props {
  /** The page in the file. */
  src: number;
  widthPt: number;
  heightPt: number;
  zoom: number;
}

interface Mark {
  id: string;
  number: number | null;
  kind: CommentKind;
  anchor: Anchor;
  strike: boolean;
  pending: boolean;
}

/** Badges closer than this (points) are moved apart. */
const BADGE_STEP = 11;

export default function MarkupLayer(props: Props) {
  return (
    <Show when={reviewing()}>
      <Marks {...props} />
    </Show>
  );
}

function Marks(props: Props) {
  const tab = useContext(TabContext)!;
  const path = () => tab.documentStore.state.path;

  const marks = createMemo<Mark[]>(() => {
    const review = reviewFor(path());
    const out: Mark[] = [];
    if (review) {
      for (const { comment, number } of numberComments(review.doc.comments)) {
        if (comment.anchor.page !== props.src) continue;
        out.push({
          id: comment.id,
          number,
          kind: comment.kind,
          anchor: comment.anchor,
          strike: comment.kind === 'typo' && comment.replacement !== null,
          pending: false,
        });
      }
    }
    const d = draft();
    if (d && d.path === path() && d.anchor.page === props.src) {
      out.push({
        id: 'draft',
        number: null,
        kind: d.kind,
        anchor: d.anchor,
        strike: d.mode === 'typo',
        pending: true,
      });
    }
    return out;
  });

  /** The margin numbers, nudged apart where they would touch. */
  const badges = createMemo(() => {
    let last = -Infinity;
    return marks()
      .filter((m) => m.number !== null)
      .map((m) => ({ mark: m, top: anchorTopFromTop(m.anchor, props.heightPt) }))
      .sort((a, b) => a.top - b.top)
      .map((b) => {
        const top = Math.max(b.top, last + BADGE_STEP);
        last = top;
        return { ...b, top };
      });
  });

  const z = () => props.zoom;
  const open = (id: string) => {
    editComment(null);
    flashCard(id);
  };

  return (
    <div class="mk-layer" aria-hidden="false">
      <For each={marks()}>
        {(mark) => (
          <>
            <Show when={mark.anchor.rect}>
              {(r) => (
                <div
                  class={`mk-mark mk-region is-${mark.kind}`}
                  classList={{ 'is-pending': mark.pending }}
                  style={{
                    left: `${r().x * z()}px`,
                    top: `${(props.heightPt - r().y - r().h) * z()}px`,
                    width: `${r().w * z()}px`,
                    height: `${r().h * z()}px`,
                  }}
                />
              )}
            </Show>
            <For each={mark.anchor.quads ?? []}>
              {(q) => {
                const r = quadRect(q);
                return (
                  <div
                    class={`mk-mark is-${mark.kind}`}
                    classList={{ 'is-strike': mark.strike, 'is-pending': mark.pending }}
                    style={{
                      left: `${r.x * z()}px`,
                      top: `${(props.heightPt - r.y - r.h) * z()}px`,
                      width: `${r.w * z()}px`,
                      height: `${r.h * z()}px`,
                    }}
                  />
                );
              }}
            </For>
          </>
        )}
      </For>
      <For each={badges()}>
        {(b) => (
          <button
            type="button"
            class={`mk-pin is-${b.mark.kind}`}
            style={{
              left: `${(props.widthPt - 20) * z()}px`,
              top: `${b.top * z()}px`,
              'font-size': `${Math.max(8, Math.min(11, 7.5 * z()))}px`,
            }}
            title={`Comment ${b.mark.number}`}
            aria-label={`Comment ${b.mark.number}`}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => open(b.mark.id)}
            onDblClick={() => editComment(b.mark.id)}
          >
            {b.mark.number}
          </button>
        )}
      </For>
    </div>
  );
}
