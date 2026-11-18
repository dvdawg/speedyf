/** A checked digital signature, as `engine::signing::SignatureReport`
 * serializes it. */
import type { PdfRect } from '../../types/model';

export type SignatureStatus = 'valid' | 'identityUnknown' | 'modified' | 'invalid' | 'unsupported';

export interface SignatureChange {
  /** formFill, comment, signature, metadata, pageContent, pages, fieldAdded, other */
  kind: string;
  allowed: boolean;
  /** zero-based source page, when the change is about one page */
  page: number | null;
  detail: string;
}

export interface SignatureReport {
  fieldName: string | null;
  /** zero-based source page of the signature's widget */
  page: number | null;
  /** the widget in display-normalized page space, y up */
  rect: PdfRect | null;
  kind: 'approval' | 'certification' | 'timestamp';
  subFilter: string | null;
  status: SignatureStatus;
  summary: string;
  signer: {
    name: string | null;
    email: string | null;
    organization: string | null;
    issuer: string | null;
  };
  /** `YYYY-MM-DD HH:MM UTC` */
  time: string | null;
  timeSource: 'timestamp' | 'signer' | 'pdf' | null;
  timestamp: { time: string | null; valid: boolean; authority: string | null } | null;
  intact: boolean;
  problem: string | null;
  coversWholeFile: boolean;
  revision: number;
  revisions: number;
  trust: 'trusted' | 'untrusted' | 'notChecked';
  trustDetail: string | null;
  certification: number | null;
  locks: string | null;
  reason: string | null;
  location: string | null;
  digest: string | null;
  changes: SignatureChange[];
}
