/** The Protect dialog's form, as data: what it starts from and what it asks
 * the next save to do. Kept apart from the component so the rules can be
 * tested without rendering. */
import type { PermissionSet, ProtectRequest, ProtectionInfo } from '../../types/engine';
import { ALL_PERMISSIONS, grantsEverything } from './permissions';

export interface ProtectForm {
  requireOpen: boolean;
  openPassword: string;
  openConfirm: string;
  restrict: boolean;
  permissionsPassword: string;
  permissionsConfirm: string;
  permissions: PermissionSet;
}

/** Where the form starts: the change already waiting for the next save, or
 * else the file's own protection. Passwords the file already has are never
 * known here, so they start empty and have to be typed again. */
export function initialForm(
  protection: ProtectionInfo | null,
  pending: ProtectRequest | null
): ProtectForm {
  const blank: ProtectForm = {
    requireOpen: false,
    openPassword: '',
    openConfirm: '',
    restrict: false,
    permissionsPassword: '',
    permissionsConfirm: '',
    permissions: { ...ALL_PERMISSIONS },
  };
  if (pending?.kind === 'remove') return blank;
  if (pending?.kind === 'set') {
    return {
      requireOpen: pending.openPassword !== '',
      openPassword: pending.openPassword,
      openConfirm: pending.openPassword,
      restrict: pending.permissionsPassword !== '',
      permissionsPassword: pending.permissionsPassword,
      permissionsConfirm: pending.permissionsPassword,
      permissions: { ...pending.permissions },
    };
  }
  if (!protection?.encrypted) return blank;
  return {
    ...blank,
    requireOpen: protection.openPassword,
    restrict: !grantsEverything(protection.permissions),
    permissions: { ...protection.permissions },
  };
}

export type FormResult =
  { ok: true; request: ProtectRequest | null } | { ok: false; error: string };

/** What saving the form asks of the next save, or why it cannot be saved.
 *
 * `null` keeps the file as it is: asking for no protection on a file that
 * has none is no change at all. */
export function requestFrom(form: ProtectForm, protection: ProtectionInfo | null): FormResult {
  const encrypted = protection?.encrypted ?? false;
  if (!form.requireOpen && !form.restrict) {
    return { ok: true, request: encrypted ? { kind: 'remove' } : null };
  }
  if (form.requireOpen) {
    if (form.openPassword === '') return { ok: false, error: 'Enter a password to open.' };
    if (form.openPassword !== form.openConfirm) {
      return { ok: false, error: 'The passwords to open do not match.' };
    }
  }
  if (form.restrict) {
    if (form.permissionsPassword === '') {
      return { ok: false, error: 'Enter a permissions password.' };
    }
    if (form.permissionsPassword !== form.permissionsConfirm) {
      return { ok: false, error: 'The permissions passwords do not match.' };
    }
    if (form.requireOpen && form.permissionsPassword === form.openPassword) {
      return {
        ok: false,
        error: 'The permissions password must differ from the password to open.',
      };
    }
  }
  return {
    ok: true,
    request: {
      kind: 'set',
      openPassword: form.requireOpen ? form.openPassword : '',
      permissionsPassword: form.restrict ? form.permissionsPassword : '',
      permissions: form.restrict ? { ...form.permissions } : { ...ALL_PERMISSIONS },
    },
  };
}

/** A rough guide, not a promise: length and variety of characters. */
export function passwordStrength(password: string): 'weak' | 'fair' | 'strong' {
  const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((r) =>
    r.test(password)
  ).length;
  if (password.length >= 16 || (password.length >= 12 && classes >= 3)) return 'strong';
  if (password.length >= 8 && classes >= 2) return 'fair';
  return 'weak';
}

/** One line on a file's protection, for the dialog and the status bar. */
export function describeProtection(
  protection: ProtectionInfo | null,
  pending: ProtectRequest | null
): string {
  if (pending?.kind === 'remove') return 'Protection will be removed when you save.';
  if (pending?.kind === 'set') {
    const parts = [
      pending.openPassword ? 'a password to open' : null,
      pending.permissionsPassword ? 'a permissions password' : null,
    ].filter(Boolean);
    return `Will be protected with AES-256 and ${parts.join(' and ')} when you save.`;
  }
  if (!protection?.encrypted) return 'This document is not protected.';
  const opening = protection.openPassword
    ? 'Opening it asks for a password.'
    : 'It opens without a password.';
  const rights = grantsEverything(protection.permissions)
    ? ''
    : protection.ownerUnlocked
      ? ' Its permissions are lifted for this session.'
      : ' Some actions need its permissions password.';
  return `Protected with ${protection.method}. ${opening}${rights}`;
}
