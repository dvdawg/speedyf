/** While a certificate signature is being placed: every page takes a drag
 * (the box the signature is drawn in) or a click (a box of the usual size
 * there, or the empty signature field under the click). Escape goes back to
 * the dialog. Pointer events only: HTML drag and drop does not work in the
 * app's webview. */
import { createResource, createSignal, Show } from 'solid-js';
import type { PageEntry, PdfPoint, PdfRect } from '../../../types/model';
import {
  boxAt,
  boxFrom,
  cancelPlacing,
  finishPlacing,
  openSignatureFields,
  setCertError,
} from './certStore';
import './certificate.css';

interface Props {
  docId: number;
  page: PageEntry;
  /** The page's position in the document as shown. */
  pageIndex: number;
  widthPt: number;
  heightPt: number;
  zoom: number;
}

/** A drag shorter than this, in points, is a click. */
const CLICK_SLOP = 3;

export default function CertPlaceLayer(props: Props) {
  let layerEl: HTMLDivElement | undefined;
  const [start, setStart] = createSignal<PdfPoint | null>(null);
  const [draft, setDraft] = createSignal<PdfRect | null>(null);
  const [fields] = createResource(
    () => props.docId,
    (id) => openSignatureFields(id)
  );

  const toPdf = (e: PointerEvent): PdfPoint => {
    const box = layerEl!.getBoundingClientRect();
    return {
      x: ((e.clientX - box.left) / Math.max(1, box.width)) * props.widthPt,
      y: props.heightPt - ((e.clientY - box.top) / Math.max(1, box.height)) * props.heightPt,
    };
  };

  const pageBox = () => ({ w: props.widthPt, h: props.heightPt });

  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    if (props.page.userRotation !== 0) {
      cancelPlacing();
      setCertError('Save the rotated page first, then sign it.');
      return;
    }
    layerEl!.setPointerCapture?.(e.pointerId);
    const p = toPdf(e);
    setStart(p);
    setDraft(null);
  };

  const onMove = (e: PointerEvent) => {
    const a = start();
    if (!a) return;
    const b = toPdf(e);
    const x0 = Math.min(a.x, b.x);
    const y0 = Math.min(a.y, b.y);
    setDraft({ x: x0, y: y0, w: Math.abs(b.x - a.x), h: Math.abs(b.y - a.y) });
  };

  const onUp = (e: PointerEvent) => {
    const a = start();
    if (!a) return;
    setStart(null);
    setDraft(null);
    const b = toPdf(e);
    const moved = Math.hypot(b.x - a.x, b.y - a.y) > CLICK_SLOP;
    if (!moved) {
      // A click in an empty signature field picks that field.
      const field = (fields() ?? []).find(
        (f) =>
          f.page === props.page.srcIndex &&
          b.x >= f.rect.x &&
          b.x <= f.rect.x + f.rect.w &&
          b.y >= f.rect.y &&
          b.y <= f.rect.y + f.rect.h
      );
      if (field) {
        finishPlacing({ kind: 'existing', name: field.name, pageIndex: props.pageIndex });
        return;
      }
      finishPlacing({ kind: 'new', pageIndex: props.pageIndex, rect: boxAt(b, pageBox()) });
      return;
    }
    const rect = boxFrom(a, b, pageBox());
    if (rect) finishPlacing({ kind: 'new', pageIndex: props.pageIndex, rect });
  };

  return (
    <div
      ref={layerEl}
      class="cert-place-layer"
      role="presentation"
      title="Drag a box for the signature, or click an empty signature field. Esc goes back."
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={() => {
        setStart(null);
        setDraft(null);
      }}
    >
      <Show when={draft()}>
        {(r) => (
          <div
            class="cert-place-draft"
            style={{
              left: `${r().x * props.zoom}px`,
              top: `${(props.heightPt - r().y - r().h) * props.zoom}px`,
              width: `${r().w * props.zoom}px`,
              height: `${r().h * props.zoom}px`,
            }}
          />
        )}
      </Show>
    </div>
  );
}
