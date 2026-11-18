/** What a protected document allows this session, and how the reader says
 * no.
 *
 * A file with a permissions (owner) password can withhold printing, copying,
 * commenting and so on from whoever opened it without that password. The
 * reader honours that rather than quietly stripping it on save: the actions
 * it withholds are disabled, and say why. Typing the permissions password in
 * Protect... lifts them for the session. */
import type { PermissionSet, ProtectionInfo } from '../../types/engine';

export type Right = 'print' | 'copy' | 'edit' | 'annotate' | 'fillForms' | 'assemble' | 'protect';

export const ALL_PERMISSIONS: PermissionSet = {
  print: 'high',
  copy: true,
  edit: true,
  annotate: true,
  fillForms: true,
  assemble: true,
  accessibility: true,
};

export function grantsEverything(p: PermissionSet): boolean {
  return (
    p.print === 'high' &&
    p.copy &&
    p.edit &&
    p.annotate &&
    p.fillForms &&
    p.assemble &&
    p.accessibility
  );
}

/** Some right is withheld from this session. */
export function isRestricted(p: ProtectionInfo | null | undefined): boolean {
  return !!p && p.encrypted && !p.ownerUnlocked && !grantsEverything(p.permissions);
}

/** Whether this session may do `right`. Unprotected documents allow all. */
export function allows(p: ProtectionInfo | null | undefined, right: Right): boolean {
  if (!isRestricted(p)) return true;
  const perms = p!.permissions;
  switch (right) {
    case 'print':
      return perms.print !== 'none';
    case 'copy':
      return perms.copy;
    case 'edit':
      return perms.edit;
    case 'annotate':
      return perms.annotate;
    // Commenting rights include filling in fields (ISO 32000, table 22).
    case 'fillForms':
      return perms.fillForms || perms.annotate;
    case 'assemble':
      return perms.assemble || perms.edit;
    case 'protect':
      return false;
  }
}

const ACTION: Record<Right, string> = {
  print: 'Printing',
  copy: 'Copying text',
  edit: 'Changing the content',
  annotate: 'Adding or changing comments',
  fillForms: 'Filling in the form',
  assemble: 'Changing pages',
  protect: 'Changing the protection',
};

/** Why `right` is unavailable, or null when it is not. */
export function deniedReason(p: ProtectionInfo | null | undefined, right: Right): string | null {
  if (allows(p, right)) return null;
  return `${ACTION[right]} is not allowed by this document's permissions`;
}

/** Commands that need a right beyond having the document open. */
export const COMMAND_RIGHTS: Readonly<Record<string, Right>> = {
  'file.print': 'print',
  'edit.deleteSelection': 'annotate',
  'tools.highlight': 'annotate',
  'tools.ink': 'annotate',
  'tools.rect': 'annotate',
  'tools.textbox': 'annotate',
  'tools.note': 'annotate',
  'tools.bubble': 'annotate',
  'page.rotateRight': 'assemble',
  'page.rotateLeft': 'assemble',
  'page.duplicate': 'assemble',
  'page.delete': 'assemble',
  'page.addBlank': 'assemble',
  'forms.reset': 'fillForms',
  'sign.place': 'annotate',
  'sign.placeInitials': 'annotate',
  'sign.flatten': 'edit',
};

/** The right a document edit needs; null for edits anyone may make. Only
 * forward edits are checked: undoing one that was allowed is allowed. */
export function rightForOp(type: string): Right | null {
  switch (type) {
    case 'reorder':
    case 'delete':
    case 'duplicate':
    case 'rotate':
    case 'addBlank':
      return 'assemble';
    case 'addAnnot':
    case 'patchAnnot':
    case 'deleteAnnot':
      return 'annotate';
    case 'setForm':
    case 'setFormButton':
    case 'setFormChoice':
    case 'setFormValues':
      return 'fillForms';
    case 'setFlattenSignatures':
      return 'edit';
    case 'setProtection':
      return 'protect';
    default:
      return null;
  }
}
