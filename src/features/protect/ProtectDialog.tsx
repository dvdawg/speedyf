/** Protect...: a password to open, a permissions password and what it
 * allows, or no protection. Nothing is written until the document is saved;
 * the dialog only stages the change, as one undoable edit. */
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { PermissionSet, PrintLevel } from '../../types/engine';
import type { TabRecord } from '../../stores/tabsStore';
import { isRestricted } from './permissions';
import {
  describeProtection,
  initialForm,
  passwordStrength,
  requestFrom,
  type ProtectForm,
} from './protectForm';
import { applyProtection, closeProtect, unlockPermissions } from './protectStore';
import './protect.css';

type Flag = Exclude<keyof PermissionSet, 'print'>;

const FLAGS: { key: Flag; label: string }[] = [
  { key: 'copy', label: 'Copy text and images' },
  { key: 'edit', label: 'Change the content' },
  { key: 'annotate', label: 'Add and change comments' },
  { key: 'fillForms', label: 'Fill in form fields' },
  { key: 'assemble', label: 'Insert, delete and rotate pages' },
  { key: 'accessibility', label: 'Read text for accessibility tools' },
];

const PRINT_LEVELS: { value: PrintLevel; label: string }[] = [
  { value: 'high', label: 'High resolution' },
  { value: 'low', label: 'Low resolution' },
  { value: 'none', label: 'Not allowed' },
];

function PasswordPair(props: {
  label: string;
  value: string;
  confirm: string;
  onValue: (v: string) => void;
  onConfirm: (v: string) => void;
}) {
  const strength = () => passwordStrength(props.value);
  return (
    <div class="protect-pair">
      <input
        type="password"
        aria-label={props.label}
        placeholder={props.label}
        value={props.value}
        onInput={(e) => props.onValue(e.currentTarget.value)}
      />
      <input
        type="password"
        aria-label={`Confirm ${props.label.toLowerCase()}`}
        placeholder="Confirm"
        value={props.confirm}
        onInput={(e) => props.onConfirm(e.currentTarget.value)}
      />
      <Show when={props.value !== ''}>
        <span class="protect-strength" data-strength={strength()}>
          Strength: {strength()}
        </span>
      </Show>
    </div>
  );
}

export default function ProtectDialog(props: { tab: TabRecord }) {
  const doc = () => props.tab.documentStore.state;
  const [form, setForm] = createStore<ProtectForm>(
    initialForm(doc().protection, doc().protectChange)
  );
  const [error, setError] = createSignal<string | null>(null);
  const [unlockPassword, setUnlockPassword] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  let dialogEl: HTMLDivElement | undefined;

  const locked = () => isRestricted(doc().protection);
  const signatures = () => doc().protection?.signatures ?? 0;
  const canRemove = () =>
    (doc().protection?.encrypted ?? false) || doc().protectChange?.kind === 'set';

  const save = async () => {
    const result = requestFrom(form, doc().protection);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setBusy(true);
    try {
      if (await applyProtection(props.tab, result.request)) closeProtect();
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      const request = doc().protection?.encrypted ? ({ kind: 'remove' } as const) : null;
      if (await applyProtection(props.tab, request)) closeProtect();
    } finally {
      setBusy(false);
    }
  };

  const unlock = async () => {
    setBusy(true);
    try {
      const failed = await unlockPermissions(props.tab, unlockPassword());
      setUnlockPassword('');
      setError(failed);
      if (!failed) setForm(initialForm(doc().protection, doc().protectChange));
    } finally {
      setBusy(false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeProtect();
      return;
    }
    if (e.key !== 'Tab' || !dialogEl) return;
    const controls = [
      ...dialogEl.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled)'
      ),
    ];
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  onMount(() => {
    const restore = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    queueMicrotask(() => dialogEl?.querySelector<HTMLElement>('input, button')?.focus());
    onCleanup(() => {
      if (restore?.isConnected) restore.focus();
    });
  });

  return (
    <div class="modal-backdrop" onKeyDown={onKey}>
      <div
        ref={dialogEl}
        class="modal protect-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Protect"
      >
        <h2>Protect</h2>
        <p class="protect-status">{describeProtection(doc().protection, doc().protectChange)}</p>

        <Show
          when={!locked()}
          fallback={
            <form
              class="protect-unlock"
              onSubmit={(e) => {
                e.preventDefault();
                void unlock();
              }}
            >
              <p class="protect-note">
                Enter the permissions password to change this document's protection or use what it
                restricts.
              </p>
              <input
                type="password"
                aria-label="Permissions password"
                placeholder="Permissions password"
                value={unlockPassword()}
                onInput={(e) => setUnlockPassword(e.currentTarget.value)}
              />
              <Show when={error()}>
                <p class="protect-error" role="alert">
                  {error()}
                </p>
              </Show>
              <div class="modal-actions">
                <button type="button" class="secondary-btn" onClick={() => closeProtect()}>
                  Close
                </button>
                <button
                  type="submit"
                  class="primary-btn"
                  disabled={busy() || unlockPassword() === ''}
                >
                  Unlock
                </button>
              </div>
            </form>
          }
        >
          <label class="modal-check">
            <input
              type="checkbox"
              checked={form.requireOpen}
              onChange={(e) => setForm('requireOpen', e.currentTarget.checked)}
            />
            <span>Require a password to open</span>
          </label>
          <Show when={form.requireOpen}>
            <PasswordPair
              label="Password to open"
              value={form.openPassword}
              confirm={form.openConfirm}
              onValue={(v) => setForm('openPassword', v)}
              onConfirm={(v) => setForm('openConfirm', v)}
            />
          </Show>

          <label class="modal-check">
            <input
              type="checkbox"
              checked={form.restrict}
              onChange={(e) => setForm('restrict', e.currentTarget.checked)}
            />
            <span>Restrict what others can do</span>
          </label>
          <Show when={form.restrict}>
            <PasswordPair
              label="Permissions password"
              value={form.permissionsPassword}
              confirm={form.permissionsConfirm}
              onValue={(v) => setForm('permissionsPassword', v)}
              onConfirm={(v) => setForm('permissionsConfirm', v)}
            />
            <fieldset class="protect-perms">
              <legend>Without the permissions password, allow</legend>
              <label class="protect-print">
                <span>Printing</span>
                <select
                  aria-label="Printing"
                  value={form.permissions.print}
                  onChange={(e) =>
                    setForm('permissions', 'print', e.currentTarget.value as PrintLevel)
                  }
                >
                  <For each={PRINT_LEVELS}>
                    {(level) => <option value={level.value}>{level.label}</option>}
                  </For>
                </select>
              </label>
              <For each={FLAGS}>
                {(flag) => (
                  <label class="modal-check">
                    <input
                      type="checkbox"
                      checked={form.permissions[flag.key]}
                      onChange={(e) => setForm('permissions', flag.key, e.currentTarget.checked)}
                    />
                    <span>{flag.label}</span>
                  </label>
                )}
              </For>
            </fieldset>
          </Show>

          <p class="protect-note">
            Saved with AES-256 encryption. Enter existing passwords again to keep them. A forgotten
            password to open cannot be recovered. Permissions are honoured by PDF readers, not
            enforced by the encryption.
          </p>
          <Show when={signatures() > 0}>
            <p class="protect-note protect-warn">
              This document is digitally signed. Changing its protection rewrites the file, and the
              signatures will no longer verify.
            </p>
          </Show>
          <Show when={error()}>
            <p class="protect-error" role="alert">
              {error()}
            </p>
          </Show>
          <div class="modal-actions">
            <Show when={canRemove()}>
              <button
                type="button"
                class="secondary-btn protect-remove"
                disabled={busy()}
                onClick={() => void remove()}
              >
                Remove protection
              </button>
            </Show>
            <button type="button" class="secondary-btn" onClick={() => closeProtect()}>
              Cancel
            </button>
            <button type="button" class="primary-btn" disabled={busy()} onClick={() => void save()}>
              Apply
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
}
