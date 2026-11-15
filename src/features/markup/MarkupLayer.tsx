/** A page's review marks while reviewing: a soft wash under each commented
 * passage in its kind's colour, a strike through words a typo fix
 * replaces, a thin box round a region, and the comment's number just after
 * its phrase (at a region's top-right corner) - where the export draws it.
 * The wash lets text selection through; the number opens the comment in
 * the drawer.
 *
 * Lives in the page's unrotated page space, like the annotation layer, so
 * it follows zoom and rotation without arithmetic of its own. */
import { createMemo, For, Show, useContext } from 'solid-js';
import { TabContext } from '../../app/TabContext';
import { markerRects, numberComments, quadRect } from './geometry';
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

  /** The numbers by the phrases, where the export draws them. */
  const badges = createMemo(() => {
    const numbered = marks().filter((m) => m.number !== null);
    const rects = markerRects(numbered.map((m) => ({ number: m.number!, anchor: m.anchor })));
    return numbered.flatMap((mark) => {
      const rect = rects.get(mark.number!);
      return rect ? [{ mark, rect }] : [];
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
              left: `${b.rect.x * z()}px`,
              top: `${(props.heightPt - b.rect.y - b.rect.h) * z()}px`,
              width: `${b.rect.w * z()}px`,
              height: `${b.rect.h * z()}px`,
              'font-size': `${b.rect.h * 0.78 * z()}px`,
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
