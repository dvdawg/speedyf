import { describe, expect, it } from 'vitest';
import type { AnchorRect } from './linkStore';
import {
  positionPreviewPopover,
  previewContentSize,
  previewPopoverSize,
  previewRasterStyle,
} from './previewLayout';

const viewport = { width: 1_000, height: 800 };
const lowerRightAnchor: AnchorRect = {
  left: 890,
  top: 700,
  right: 910,
  bottom: 720,
  width: 20,
  height: 20,
};

describe('preview popover geometry', () => {
  it('uses the current dimensions for default previews and 20 percent larger internal dimensions', () => {
    expect(previewPopoverSize(viewport, 'default')).toEqual({ width: 420, height: 320 });
    expect(previewPopoverSize(viewport, 'internal')).toEqual({ width: 504, height: 384 });
  });

  it('caps both variants to the available viewport', () => {
    expect(previewPopoverSize({ width: 300, height: 240 }, 'internal')).toEqual({
      width: 284,
      height: 224,
    });
  });

  it('derives display size from crop pixels instead of magnifying render resolution', () => {
    expect(previewContentSize({ tile: { x: 0, y: 0, w: 240, h: 120 }, scaleMilli: 2_000 })).toEqual(
      { width: 120, height: 60 }
    );
  });

  it('lets intrinsic aspect ratio scale the raster height instead of using device pixels', () => {
    expect(previewRasterStyle({ tile: { x: 0, y: 0, w: 802, h: 230 }, scaleMilli: 2_000 })).toEqual(
      { width: '401px', height: 'auto' }
    );
  });

  it('shrinks compact internal cards to their content footprint', () => {
    const content = { width: 100, height: 60 };
    expect(previewPopoverSize(viewport, 'internal', content)).toEqual({
      width: 140,
      height: 102,
    });
  });

  it('fits wide content before deciding whether vertical scrolling is needed', () => {
    const content = { width: 600, height: 400 };
    expect(previewPopoverSize(viewport, 'internal', content)).toEqual({
      width: 504,
      height: 361,
    });
  });

  it('caps tall content at the scrollable internal-card height', () => {
    const content = { width: 200, height: 800 };
    expect(previewPopoverSize(viewport, 'internal', content)).toEqual({
      width: 226,
      height: 384,
    });
  });

  it('positions each variant within the lower-right viewport edge', () => {
    expect(positionPreviewPopover(lowerRightAnchor, viewport, 'default')).toEqual({
      left: '460px',
      top: '400px',
    });
    expect(positionPreviewPopover(lowerRightAnchor, viewport, 'internal')).toEqual({
      left: '376px',
      top: '336px',
    });
  });

  it('uses the safe viewport origin without an anchor', () => {
    expect(positionPreviewPopover(undefined, viewport, 'internal')).toEqual({
      left: '8px',
      top: '8px',
    });
  });
});
