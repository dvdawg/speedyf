/** Typed transport for certificate signing (src-tauri/src/commands/certsign.rs). */
import { invoke } from '@tauri-apps/api/core';
import type { SignatureArt } from '../../../types/model';

/** A certificate and its private key, as the signing dialog lists it. */
export interface Identity {
  id: string;
  /** Where the private key lives. */
  store: 'keychain' | 'file';
  name: string;
  email: string;
  organization: string;
  issuer: string;
  /** Validity, seconds since the Unix epoch. */
  notBefore: number;
  notAfter: number;
  selfSigned: boolean;
  /** Signing asks for the passphrase the key was stored under. */
  needsPassphrase: boolean;
}

export type StoreKind = 'keychain' | 'file';

export type FieldChoice =
  | { kind: 'existing'; name: string }
  | { kind: 'new'; page: number; rect: { x: number; y: number; w: number; h: number } };

export type LockChoice = { kind: 'none' } | { kind: 'all' } | { kind: 'include'; fields: string[] };

export interface SignRequest {
  field: FieldChoice;
  reason: string;
  location: string;
  contact: string;
  /** The date as the appearance shows it, in local time. */
  dateText: string;
  art: { art: SignatureArt; color: string } | null;
  /** DocMDP permissions when certifying. */
  certify: 1 | 2 | 3 | null;
  lock: LockChoice;
  subFilter: 'cades' | 'pkcs7';
  /** Only sent when the user turned the online timestamp on. */
  tsaUrl: string | null;
}

export interface SignResult {
  path: string;
  bytes: number;
  timestamped: boolean;
}

export const certApi = {
  identities: (): Promise<Identity[]> => invoke('cert_identities'),
  create: (args: {
    name: string;
    email: string;
    organization: string;
    store: StoreKind;
    passphrase: string | null;
  }): Promise<Identity> => invoke('cert_create_identity', args),
  importP12: (args: {
    path: string;
    password: string;
    store: StoreKind;
    passphrase: string | null;
  }): Promise<Identity> => invoke('cert_import_identity', args),
  remove: (id: string): Promise<void> => invoke('cert_remove_identity', { id }),
  sign: (args: {
    docId: number;
    srcPath: string;
    destPath: string;
    identity: string;
    passphrase: string | null;
    request: SignRequest;
  }): Promise<SignResult> => invoke('cert_sign', args),
};
