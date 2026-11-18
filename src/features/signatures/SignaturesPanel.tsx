/** The Signatures panel: every digital signature in the document, checked -
 * whether its bytes are intact, who signed and when, whether the signer's
 * certificate is trusted, and what changed in the file after it was signed.
 * Clicking a signature goes to its field on the page. */
import { createResource, For, Show, useContext } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import { layoutIndexOf } from '../outline/jumpToAnchor';
import { recordJumpForViewport } from '../viewer/navHistory';
import {
  documentSignatures,
  signerLabel,
  statusLabel,
  statusTone,
  timeLabel,
} from './signatureStore';
import type { SignatureReport } from './types';

/** How far down the viewport a signature scrolled into view lands. */
const LAND_AT = 0.3;

const CERTIFICATION: Record<number, string> = {
  1: 'Certified: no changes allowed',
  2: 'Certified: form filling and signing allowed',
  3: 'Certified: form filling, signing and comments allowed',
};

export function kindLine(report: SignatureReport): string | null {
  if (report.kind === 'certification') {
    return CERTIFICATION[report.certification ?? 2] ?? 'Certified';
  }
  if (report.kind === 'timestamp') return 'Document timestamp';
  return null;
}

export function trustLine(report: SignatureReport): string | null {
  if (report.status === 'invalid' || report.status === 'unsupported') return null;
  switch (report.trust) {
    case 'trusted':
      return 'Certificate trusted by this Mac';
    case 'untrusted':
      return 'Certificate not trusted by this Mac';
    case 'notChecked':
      return 'Identity not checked on this system';
  }
}

export default function SignaturesPanel() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp } = tab;
  const doc = documentStore.state;
  const [reports] = createResource(
    () => (doc.loaded ? { docId: doc.docId, path: doc.path } : null),
    ({ docId, path }) => documentSignatures(docId, path)
  );

  const jump = (report: SignatureReport) => {
    if (report.page === null) return;
    const page = layoutIndexOf(doc, report.page);
    if (page === null) return; // the page was deleted in this session
    let offsetCss = 0;
    if (report.rect) {
      const geom = doc.pages[page]!;
      const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
      const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
      const top = report.rect.y + report.rect.h;
      offsetCss = Math.max(0, (heightPt - top) * vp.state.zoom - vp.state.containerH * LAND_AT);
    }
    recordJumpForViewport(vp, { page, offsetCss });
    vp.requestScrollToPage(page, offsetCss);
  };

  return (
    <div class="form-panel sig-panel" aria-label="Digital signatures">
      <div class="panel-head">
        <h2>Signatures</h2>
        <IconButton
          label="Close signatures panel"
          onClick={() => vp.setState('signaturesPanelOpen', false)}
        >
          <IconClose />
        </IconButton>
      </div>
      <Show when={!reports.loading} fallback={<div class="panel-note">Checking signatures...</div>}>
        <Show
          when={(reports() ?? []).length > 0}
          fallback={<div class="panel-note">This document has no digital signatures.</div>}
        >
          <div class="sig-list" role="list">
            <For each={reports()}>
              {(report) => (
                <div class="sig-item" role="listitem">
                  <button
                    type="button"
                    class="sig-row"
                    disabled={report.page === null}
                    title={report.page === null ? undefined : 'Go to the signature'}
                    onClick={() => jump(report)}
                  >
                    <span class="sig-status" data-tone={statusTone(report.status)}>
                      {statusLabel(report.status)}
                    </span>
                    <span class="sig-signer">{signerLabel(report)}</span>
                    <Show when={timeLabel(report)}>
                      {(time) => <span class="sig-meta">{time()}</span>}
                    </Show>
                    <Show when={report.fieldName || report.page !== null}>
                      <span class="sig-meta">
                        {[report.fieldName, report.page !== null ? `page ${report.page + 1}` : null]
                          .filter(Boolean)
                          .join(', ')}
                      </span>
                    </Show>
                  </button>
                  <div class="sig-detail">
                    <p>{report.summary}</p>
                    <Show when={kindLine(report)}>{(line) => <p>{line()}</p>}</Show>
                    <Show when={report.locks}>{(locks) => <p>{locks()}</p>}</Show>
                    <Show when={trustLine(report)}>
                      {(line) => <p title={report.trustDetail ?? undefined}>{line()}</p>}
                    </Show>
                    <Show when={report.signer.email || report.signer.organization}>
                      <p>
                        {[report.signer.organization, report.signer.email]
                          .filter(Boolean)
                          .join(', ')}
                      </p>
                    </Show>
                    <Show when={report.signer.issuer && report.kind !== 'timestamp'}>
                      <p>Issued by {report.signer.issuer}</p>
                    </Show>
                    <Show when={report.reason}>{(reason) => <p>Reason: {reason()}</p>}</Show>
                    <Show when={report.timestamp && !report.timestamp.valid}>
                      <p class="sig-warn">The embedded timestamp does not match this signature.</p>
                    </Show>
                    <Show
                      when={report.changes.length > 0}
                      fallback={
                        <Show when={report.intact && report.coversWholeFile}>
                          <p>No changes since it was signed.</p>
                        </Show>
                      }
                    >
                      <h3 class="sig-changes-head">Changed after signing</h3>
                      <ul class="sig-changes">
                        <For each={report.changes}>
                          {(change) => (
                            <li classList={{ 'sig-warn': !change.allowed }}>
                              {change.detail}
                              {change.page !== null ? ` (page ${change.page + 1})` : ''}
                              {change.allowed ? '' : ' - not allowed'}
                            </li>
                          )}
                        </For>
                      </ul>
                    </Show>
                  </div>
                </div>
              )}
            </For>
          </div>
          <div class="form-nav-foot">
            <span>Checked offline. Revocation is not checked.</span>
          </div>
        </Show>
      </Show>
    </div>
  );
}
