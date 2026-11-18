/** The toolbar's Sign button and its small menu: the saved signatures and
 * initials to place, making a new one, and whether this document's visual
 * signatures are flattened into the page when it is saved. */
import { startPreparing } from '../prepare/prepareStore';
import { For, onCleanup, Show, createEffect } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconTrash } from '../../components/icons';
import { activeTab } from '../../stores/tabsStore';
import { askConfirm } from '../../stores/modalStore';
import {
  armPlacement,
  openSignDialog,
  removeSignature,
  setSignMenuOpen,
  signaturesOf,
  signState,
  toggleFlattenSignatures,
  type SavedSignature,
} from './signStore';
import { openCertDialog } from './certificate/certStore';
import type { SignatureRole } from './art';
import SignatureArtView from './SignatureArtView';
import { fillActive, toggleFill } from './fillStore';
import './sign.css';

/** A pen nib over a signature line. */
export const IconSign = () => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d="M9.5 2.5l3 3-6 6H3.5v-3z" />
    <path d="M2 14.5h12" />
  </svg>
);

function Group(props: { role: SignatureRole }) {
  const remove = async (item: SavedSignature) => {
    const noun = props.role === 'initials' ? 'initials' : 'signature';
    if (!(await askConfirm(`Remove this saved ${noun} from this computer?`, 'Remove'))) return;
    await removeSignature(item.id).catch(() => undefined);
  };
  return (
    <div
      class="sign-menu-group"
      role="group"
      aria-label={props.role === 'initials' ? 'Initials' : 'Signatures'}
    >
      <For each={signaturesOf(props.role)}>
        {(item) => (
          <div class="sign-menu-row">
            <button
              type="button"
              role="menuitem"
              class="sign-menu-item sign-menu-saved"
              title={`Place this ${props.role === 'initials' ? 'initials' : 'signature'}, then click on the page`}
              onClick={() => armPlacement(item)}
            >
              <SignatureArtView art={item.art} color={item.color} />
            </button>
            <button
              type="button"
              class="sign-menu-remove"
              title="Remove from this computer"
              aria-label="Remove saved signature"
              onClick={() => void remove(item)}
            >
              <IconTrash />
            </button>
          </div>
        )}
      </For>
      <button
        type="button"
        role="menuitem"
        class="sign-menu-item"
        onClick={() => openSignDialog(props.role)}
      >
        {props.role === 'initials' ? 'Add initials...' : 'Add signature...'}
      </button>
    </div>
  );
}

export function SignMenu() {
  const doc = () => activeTab()?.documentStore.state;
  const loaded = () => doc()?.loaded ?? false;
  let wrap: HTMLDivElement | undefined;

  // Dismiss on a press anywhere else, or Escape.
  createEffect(() => {
    if (!signState.menuOpen) return;
    const outside = (e: PointerEvent) => {
      if (wrap && e.target instanceof Node && !wrap.contains(e.target)) setSignMenuOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSignMenuOpen(false);
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);
    onCleanup(() => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape);
    });
  });

  return (
    <div class="sign-menu-wrap" ref={wrap}>
      <IconButton
        label="Sign: signatures and filling printed forms"
        submenu
        disabled={!loaded()}
        active={signState.menuOpen || signState.pending !== null || fillActive()}
        onClick={() => setSignMenuOpen(!signState.menuOpen)}
      >
        <IconSign />
      </IconButton>
      <Show when={signState.menuOpen && loaded()}>
        <div class="sign-menu" role="menu" aria-label="Sign">
          <button
            type="button"
            role="menuitemcheckbox"
            class="sign-menu-item sign-menu-check"
            aria-checked={fillActive()}
            title="Type, tick and date a printed form that has no fields"
            onClick={() => {
              toggleFill();
              setSignMenuOpen(false);
            }}
          >
            <span class="sign-menu-box" aria-hidden="true">
              <Show when={fillActive()}>
                <svg viewBox="0 0 12 12" width="12" height="12">
                  <path
                    d="M2.5 6.2l2.3 2.3 4.7-5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
              </Show>
            </span>
            Fill printed form
          </button>
          <button
            type="button"
            role="menuitem"
            class="sign-menu-item"
            title="Add fillable fields to this document: text, checkboxes, dropdowns, signatures"
            onClick={() => {
              setSignMenuOpen(false);
              void startPreparing();
            }}
          >
            Prepare form...
          </button>
          <div class="sign-menu-separator" role="separator" />
          <Group role="signature" />
          <div class="sign-menu-separator" role="separator" />
          <Group role="initials" />
          <div class="sign-menu-separator" role="separator" />
          <button
            type="button"
            role="menuitem"
            class="sign-menu-item"
            title="Sign the file with a digital certificate, so readers can check who signed it and whether it changed"
            onClick={() => {
              setSignMenuOpen(false);
              openCertDialog();
            }}
          >
            Sign with certificate...
          </button>
          <div class="sign-menu-separator" role="separator" />
          <button
            type="button"
            role="menuitemcheckbox"
            class="sign-menu-item sign-menu-check"
            aria-checked={doc()?.flattenSignatures ?? false}
            title="Write this document's signatures into the page when it is saved, so they can no longer be moved"
            onClick={() => toggleFlattenSignatures()}
          >
            <span class="sign-menu-box" aria-hidden="true">
              <Show when={doc()?.flattenSignatures}>
                <svg viewBox="0 0 12 12" width="12" height="12">
                  <path
                    d="M2.5 6.2l2.3 2.3 4.7-5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
              </Show>
            </span>
            Flatten signatures on save
          </button>
          <p class="sign-menu-note">Visual signatures only, not certificate signatures.</p>
        </div>
      </Show>
    </div>
  );
}
