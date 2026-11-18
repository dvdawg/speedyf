/** A document's digital signatures, checked once per opened document and
 * shared by the status bar, the panel and the palette.
 *
 * Checking reads the file from disk on the Rust side (off the render worker)
 * and is offline. Most documents have no signatures; for those the answer is
 * an empty list and nothing anywhere says anything about signatures. */
import { createSignal } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import type { SignatureReport, SignatureStatus } from './types';

const inFlight = new Map<number, Promise<SignatureReport[]>>();
/** Settled answers, so a command's `enabled` can ask synchronously. */
const [settled, setSettled] = createSignal<ReadonlyMap<number, SignatureReport[]>>(new Map());

/** The signatures in document `docId`, read from `path` on disk. */
export function documentSignatures(
  docId: number,
  path: string | null
): Promise<SignatureReport[]> {
  if (!path) return Promise.resolve([]);
  const hit = inFlight.get(docId);
  if (hit) return hit;
  const request = engine
    .checkSignatures(path)
    .catch(() => [] as SignatureReport[])
    .then((reports) => {
      if (inFlight.get(docId) === request) {
        setSettled((prev) => new Map(prev).set(docId, reports));
      }
      return reports;
    });
  inFlight.set(docId, request);
  return request;
}

/** What is known so far, without asking. */
export function knownSignatures(docId: number): SignatureReport[] {
  return settled().get(docId) ?? [];
}

/** Forget a document's answer (closed, or reopened after a save). */
export function clearSignatures(docId?: number) {
  if (docId === undefined) {
    inFlight.clear();
    setSettled(new Map());
    return;
  }
  inFlight.delete(docId);
  if (settled().has(docId)) {
    setSettled((prev) => {
      const next = new Map(prev);
      next.delete(docId);
      return next;
    });
  }
}

/** Worst first: the one the status line reports when there are several. */
const SEVERITY: SignatureStatus[] = [
  'invalid',
  'modified',
  'unsupported',
  'identityUnknown',
  'valid',
];

export function worstStatus(reports: SignatureReport[]): SignatureStatus | null {
  let worst: SignatureStatus | null = null;
  for (const r of reports) {
    if (worst === null || SEVERITY.indexOf(r.status) < SEVERITY.indexOf(worst)) worst = r.status;
  }
  return worst;
}

export function statusLabel(status: SignatureStatus): string {
  switch (status) {
    case 'valid':
      return 'Valid';
    case 'identityUnknown':
      return 'Valid, identity unknown';
    case 'modified':
      return 'Modified after signing';
    case 'invalid':
      return 'Invalid';
    case 'unsupported':
      return 'Cannot be checked';
  }
}

/** The status bar's line, or null for an unsigned document. */
export function statusLine(reports: SignatureReport[]): string | null {
  const worst = worstStatus(reports);
  if (worst === null) return null;
  const label = statusLabel(worst);
  return `Signed - ${label.charAt(0).toLowerCase()}${label.slice(1)}`;
}

/** Tone for the status: plain, a caution, or a failure. */
export function statusTone(status: SignatureStatus): 'ok' | 'warn' | 'bad' {
  switch (status) {
    case 'valid':
      return 'ok';
    case 'invalid':
      return 'bad';
    default:
      return 'warn';
  }
}

/** Who signed, as one line: the certificate's name, else the PDF's /Name. */
export function signerLabel(report: SignatureReport): string {
  if (report.kind === 'timestamp') {
    return report.signer.name ? `Timestamp by ${report.signer.name}` : 'Document timestamp';
  }
  return report.signer.name ?? 'Unknown signer';
}

/** Where the time came from, said plainly. */
export function timeLabel(report: SignatureReport): string | null {
  if (!report.time) return null;
  switch (report.timeSource) {
    case 'timestamp':
      return `${report.time} (timestamped)`;
    case 'pdf':
      return `${report.time} (signer's clock, unsigned)`;
    default:
      return `${report.time} (signer's clock)`;
  }
}
