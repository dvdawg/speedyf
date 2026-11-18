/** Typed transport for visual signatures (src-tauri/src/commands/signatures.rs):
 * the saved-signature store in the app data directory, and the two jobs the
 * create dialog hands to Rust. */
import { invoke } from '@tauri-apps/api/core';
import type { SignatureStyle } from '../../types/model';

export interface TypedOutline {
  /** SVG path data, y down, in a `w` x `h` box */
  d: string;
  w: number;
  h: number;
  /** characters the style has no glyph for */
  missing: string;
}

export interface PreparedImage {
  /** base64 PNG, paper made transparent, cropped to the ink */
  png: string;
  w: number;
  h: number;
}

export const signaturesApi = {
  load: (): Promise<string | null> => invoke('signatures_load'),
  save: (json: string): Promise<void> => invoke('signatures_save', { json }),
  outline: (text: string, style: SignatureStyle): Promise<TypedOutline> =>
    invoke('signature_outline', { text, style }),
  prepareImage: (path: string): Promise<PreparedImage> => invoke('signature_image', { path }),
};
