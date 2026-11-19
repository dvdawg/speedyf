/** Make a visual signature or initials: type a name in one of three
 * handwriting styles, draw one, or import a picture of one. What is made is
 * saved on this machine only and handed straight on for placing. */
import { createSignal, For, Match, Show, Switch } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import type { PdfPoint, SignatureArt, SignatureStyle } from '../../types/model';
import { signaturesApi, type PreparedImage } from '../../lib/transport/signatures';
import { closeSignDialog, finishSignDialog, signState } from './signStore';
import { drawnArt, extendStroke, finishStroke, strokeD } from './art';
import SignatureArtView from './SignatureArtView';
import Sheet from '../../components/Sheet';

type Mode = 'type' | 'draw' | 'image';

export const SIGNATURE_STYLES: { id: SignatureStyle; label: string; family: string }[] = [
  { id: 'flowing', label: 'Flowing', family: 'SpeedyF Sign Flowing' },
  { id: 'script', label: 'Script', family: 'SpeedyF Sign Script' },
  { id: 'hand', label: 'Hand', family: 'SpeedyF Sign Hand' },
];

/** Ink, not interface: the colour the signature is written in. */
export const SIGNATURE_INKS = [
  { label: 'Black ink', value: '#1a1a1a' },
  { label: 'Blue ink', value: '#1d3c8f' },
];

/** The drawing pad's own units; the artwork is cropped out of it. */
const PAD_W = 480;
const PAD_H = 160;
const PEN = 2.4;

export default function SignDialog() {
  const role = () => signState.dialog?.role ?? 'signature';
  const noun = () => (role() === 'initials' ? 'initials' : 'signature');
  const [mode, setMode] = createSignal<Mode>('type');
  const [ink, setInk] = createSignal(SIGNATURE_INKS[0]!.value);
  const [text, setText] = createSignal('');
  const [style, setStyle] = createSignal<SignatureStyle>('flowing');
  const [strokes, setStrokes] = createSignal<PdfPoint[][]>([]);
  const [live, setLive] = createSignal<PdfPoint[] | null>(null);
  const [picture, setPicture] = createSignal<PreparedImage | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let padEl: SVGSVGElement | undefined;

  const canSave = () => {
    if (busy()) return false;
    if (mode() === 'type') return text().trim() !== '';
    if (mode() === 'draw') return strokes().length > 0;
    return picture() !== null;
  };

  const choose = (next: Mode) => {
    setMode(next);
    setError(null);
  };

  // ---------- drawing ----------

  const padPoint = (e: PointerEvent): PdfPoint => {
    const box = padEl!.getBoundingClientRect();
    return {
      x: ((e.clientX - box.left) / Math.max(1, box.width)) * PAD_W,
      y: ((e.clientY - box.top) / Math.max(1, box.height)) * PAD_H,
    };
  };

  const onPadDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    padEl!.setPointerCapture?.(e.pointerId);
    setLive([padPoint(e)]);
  };

  const onPadMove = (e: PointerEvent) => {
    const stroke = live();
    if (!stroke) return;
    // A trackpad or pen reports more positions than frames; take them all.
    const events = e.getCoalescedEvents?.() ?? [];
    let changed = false;
    for (const each of events.length > 0 ? events : [e]) {
      changed = extendStroke(stroke, padPoint(each)) || changed;
    }
    if (changed) setLive([...stroke]);
  };

  const onPadUp = (e: PointerEvent) => {
    const stroke = live();
    if (!stroke) return;
    finishStroke(stroke, padPoint(e));
    setLive(null);
    setStrokes([...strokes(), stroke]);
  };

  // ---------- image ----------

  const importImage = async () => {
    setError(null);
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg'] }],
    });
    if (typeof picked !== 'string') return;
    setBusy(true);
    try {
      setPicture(await signaturesApi.prepareImage(picked));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ---------- saving ----------

  const artwork = async (): Promise<SignatureArt | null> => {
    if (mode() === 'draw') return drawnArt(strokes(), PEN);
    if (mode() === 'image') {
      const p = picture();
      return p ? { type: 'image', w: p.w, h: p.h, png: p.png } : null;
    }
    const name = text().trim();
    const outline = await signaturesApi.outline(name, style());
    if (outline.missing) {
      setError(`This style cannot write ${outline.missing}. Try another style, or draw it.`);
      return null;
    }
    if (!outline.d) return null;
    return { type: 'typed', w: outline.w, h: outline.h, text: name, style: style(), d: outline.d };
  };

  const save = async () => {
    if (!canSave()) return;
    setBusy(true);
    setError(null);
    try {
      const art = await artwork();
      if (!art) return;
      await finishSignDialog({ role: role(), art, color: ink() });
    } catch (e) {
      setError(`Could not save the ${noun()}: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      label={role() === 'initials' ? 'Add initials' : 'Add signature'}
      class="sign-dialog"
      onCancel={() => closeSignDialog()}
    >
      <h2>{role() === 'initials' ? 'Add initials' : 'Add signature'}</h2>
      <div class="sign-modes" role="tablist" aria-label="How to make it">
        <For each={['type', 'draw', 'image'] as const}>
          {(m) => (
            <button
              type="button"
              role="tab"
              aria-selected={mode() === m}
              classList={{ 'is-active': mode() === m }}
              onClick={() => choose(m)}
            >
              {m === 'type' ? 'Type' : m === 'draw' ? 'Draw' : 'Image'}
            </button>
          )}
        </For>
      </div>

      <Switch>
        <Match when={mode() === 'type'}>
          <input
            class="sign-name"
            type="text"
            placeholder={role() === 'initials' ? 'Your initials' : 'Your name'}
            aria-label={role() === 'initials' ? 'Initials' : 'Name'}
            value={text()}
            maxLength={80}
            onInput={(e) => setText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void save();
            }}
          />
          <div class="sign-styles" role="radiogroup" aria-label="Style">
            <For each={SIGNATURE_STYLES}>
              {(s) => (
                <button
                  type="button"
                  role="radio"
                  class="sign-style"
                  aria-checked={style() === s.id}
                  aria-label={`${s.label} style`}
                  classList={{ 'is-active': style() === s.id }}
                  onClick={() => setStyle(s.id)}
                >
                  <span
                    class="sign-style-sample"
                    style={{ 'font-family': `'${s.family}', cursive`, color: ink() }}
                  >
                    {text().trim() || (role() === 'initials' ? 'A.L.' : 'Your name')}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Match>

        <Match when={mode() === 'draw'}>
          <svg
            ref={padEl}
            class="sign-pad"
            viewBox={`0 0 ${PAD_W} ${PAD_H}`}
            role="img"
            aria-label={`Drawing area: draw your ${noun()} with the mouse or trackpad`}
            onPointerDown={onPadDown}
            onPointerMove={onPadMove}
            onPointerUp={onPadUp}
            onPointerCancel={onPadUp}
          >
            <line
              class="sign-pad-line"
              x1="24"
              x2={PAD_W - 24}
              y1={PAD_H * 0.75}
              y2={PAD_H * 0.75}
            />
            <For each={[...strokes(), ...(live() ? [live()!] : [])]}>
              {(stroke) => (
                <path
                  d={strokeD(stroke)}
                  fill="none"
                  stroke={ink()}
                  stroke-width={PEN}
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              )}
            </For>
          </svg>
          <div class="sign-pad-actions">
            <button
              type="button"
              class="secondary-btn"
              disabled={strokes().length === 0}
              onClick={() => setStrokes(strokes().slice(0, -1))}
            >
              Undo stroke
            </button>
            <button
              type="button"
              class="secondary-btn"
              disabled={strokes().length === 0}
              onClick={() => setStrokes([])}
            >
              Clear
            </button>
          </div>
        </Match>

        <Match when={mode() === 'image'}>
          <div class="sign-image">
            <Show
              when={picture()}
              fallback={
                <p class="sign-hint">
                  A photo or scan of your {noun()} on white paper. The paper is made transparent and
                  the picture is cropped to the ink.
                </p>
              }
            >
              {(p) => (
                <div class="sign-image-preview">
                  <SignatureArtView
                    art={{ type: 'image', w: p().w, h: p().h, png: p().png }}
                    color={ink()}
                  />
                </div>
              )}
            </Show>
            <button
              type="button"
              class="secondary-btn"
              disabled={busy()}
              onClick={() => void importImage()}
            >
              {picture() ? 'Choose another image...' : 'Choose image...'}
            </button>
          </div>
        </Match>
      </Switch>

      <Show when={mode() !== 'image'}>
        <div class="sign-inks" role="radiogroup" aria-label="Ink colour">
          <For each={SIGNATURE_INKS}>
            {(i) => (
              <button
                type="button"
                role="radio"
                class="sign-ink"
                aria-checked={ink() === i.value}
                aria-label={i.label}
                title={i.label}
                classList={{ 'is-active': ink() === i.value }}
                style={{ '--ink': i.value }}
                onClick={() => setInk(i.value)}
              />
            )}
          </For>
        </div>
      </Show>

      <Show when={error()}>
        <p class="sign-error" role="alert">
          {error()}
        </p>
      </Show>
      <p class="sign-hint">
        Saved on this computer only. A visual signature is a picture of your {noun()}, not a
        certificate signature.
      </p>
      <div class="modal-actions">
        <button type="button" class="secondary-btn" onClick={() => closeSignDialog()}>
          Cancel
        </button>
        <button type="button" class="primary-btn" disabled={!canSave()} onClick={() => void save()}>
          Save and place
        </button>
      </div>
    </Sheet>
  );
}
