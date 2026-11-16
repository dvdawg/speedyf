import type { AnchorRect } from './linkStore';
import type { PreviewSpec } from '../../types/engine';

export type PreviewPopoverVariant = 'default' | 'internal';

export interface PreviewSize {
  width: number;
  height: number;
}

const TARGET_SIZE: Record<PreviewPopoverVariant, PreviewSize> = {
  default: { width: 420, height: 320 },
  internal: { width: 504, height: 384 },
};

// Border + popover padding + the white paper-card margins. Keeping these in
// step with global.css lets positioning use the same content-aware footprint
// that the browser renders.
const INTERNAL_CHROME_WIDTH = 26;
const INTERNAL_CHROME_HEIGHT = 42;
const INTERNAL_MIN_WIDTH = 140;
/** A tall preview (a whole figure with its caption) may grow the card to
 * this height before it is scaled down to fit... */
const INTERNAL_TALL_HEIGHT = 560;
/** ...but never below this scale: smaller than that, a paper's 9-10pt
 * type stops being legible, and the card scrolls instead. */
export const LEGIBLE_SCALE = 0.6;
/** The room a seam takes between the pieces of a stitched preview: a
 * hairline with a little paper either side. Keep in step with global.css
 * (.citation-preview-seam). */
export const SEAM_HEIGHT = 9;

type Sized = Pick<PreviewSpec, 'tile' | 'scaleMilli'> & Partial<Pick<PreviewSpec, 'more'>>;

function deviceScale(scaleMilli: number): number {
  return Number.isFinite(scaleMilli) && scaleMilli > 0 ? scaleMilli / 1_000 : 1;
}

/** One piece's size in document-sized CSS pixels. */
export function previewPartSize(tile: PreviewSpec['tile'], scaleMilli: number): PreviewSize {
  const scale = deviceScale(scaleMilli);
  return {
    width: Math.max(1, tile.w / scale),
    height: Math.max(1, tile.h / scale),
  };
}

/** Convert the encoded crop back to document-sized CSS pixels. Preview PNGs
 * are rendered above CSS resolution for sharpness; displaying their raw pixel
 * width would magnify intrinsically small targets such as equations, labels,
 * headings, or compact table cells. A stitched preview is its pieces stacked,
 * a seam between each two. */
export function previewContentSize(preview: Sized): PreviewSize {
  const parts = [preview.tile, ...(preview.more ?? []).map((part) => part.tile)].map((tile) =>
    previewPartSize(tile, preview.scaleMilli)
  );
  return {
    width: Math.max(...parts.map((part) => part.width)),
    height: parts.reduce((sum, part) => sum + part.height, 0) + SEAM_HEIGHT * (parts.length - 1),
  };
}

/** Display the high-density raster at document scale. `height: auto` is
 * intentional: the PNG pixel height is also exposed as an HTML attribute for
 * intrinsic layout, and allowing that attribute to become the CSS height
 * would stretch every 2x preview vertically. */
export function previewRasterStyle(
  preview: Pick<PreviewSpec, 'tile' | 'scaleMilli'>,
  scale = 1
): {
  width: string;
  height: 'auto';
} {
  const size = previewPartSize(preview.tile, preview.scaleMilli);
  return { width: `${size.width * scale}px`, height: 'auto' };
}

/** How much an internal preview is scaled to fit its card: down to the
 * card's width, and down to its tall height - a figure with its caption, a
 * statement stitched across a page break, is shown whole - but never below
 * a legible scale: past that the card scrolls. */
export function previewFitScale(viewport: PreviewSize, content: PreviewSize): number {
  const maxWidth = Math.min(TARGET_SIZE.internal.width, Math.max(0, viewport.width - 16));
  const maxHeight = Math.min(INTERNAL_TALL_HEIGHT, Math.max(0, viewport.height - 16));
  const widthFit = Math.min(1, Math.max(1, maxWidth - INTERNAL_CHROME_WIDTH) / content.width);
  const heightFit = Math.max(1, maxHeight - INTERNAL_CHROME_HEIGHT) / content.height;
  // Too wide to fit legibly at all: legible, and scrolled across.
  if (widthFit < LEGIBLE_SCALE) return LEGIBLE_SCALE;
  return Math.max(Math.min(widthFit, heightFit), LEGIBLE_SCALE);
}

export function previewPopoverSize(
  viewport: PreviewSize,
  variant: PreviewPopoverVariant,
  content?: PreviewSize
): PreviewSize {
  const availableWidth = Math.max(0, viewport.width - 16);
  const availableHeight = Math.max(0, viewport.height - 16);
  if (variant === 'internal' && content) {
    const maxWidth = Math.min(TARGET_SIZE.internal.width, availableWidth);
    const minWidth = Math.min(INTERNAL_MIN_WIDTH, maxWidth);
    const scale = previewFitScale(viewport, content);
    const width = Math.min(
      maxWidth,
      Math.max(minWidth, Math.ceil(content.width * scale + INTERNAL_CHROME_WIDTH))
    );
    return {
      width,
      height: Math.min(
        INTERNAL_TALL_HEIGHT,
        availableHeight,
        Math.ceil(content.height * scale + INTERNAL_CHROME_HEIGHT)
      ),
    };
  }
  return {
    width: Math.min(TARGET_SIZE[variant].width, availableWidth),
    height: Math.min(TARGET_SIZE[variant].height, availableHeight),
  };
}

export function positionPreviewPopover(
  anchor: AnchorRect | undefined,
  viewport: PreviewSize,
  variant: PreviewPopoverVariant,
  content?: PreviewSize
): { left: string; top: string } {
  if (!anchor) return { left: '8px', top: '8px' };

  const { width, height } = previewPopoverSize(viewport, variant, content);
  let left = anchor.right + 10;
  if (left + width > viewport.width - 8) left = anchor.left - width - 10;
  left = Math.max(8, Math.min(left, viewport.width - width - 8));

  let top = anchor.top;
  if (top + height > viewport.height - 8) top = anchor.bottom - height;
  top = Math.max(8, Math.min(top, viewport.height - height - 8));

  return { left: `${left}px`, top: `${top}px` };
}
