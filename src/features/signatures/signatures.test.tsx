// @vitest-environment happy-dom
/**
 * What a signed document shows: the status line's wording (worst signature
 * first), the panel's rows and their jump, and that an unsigned document
 * says nothing at all.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { engine } from '../../lib/transport/engine';
import { TabContext } from '../../app/TabContext';
import { createDocumentStore } from '../document/documentStore';
import { createViewportStore } from '../../stores/viewportStore';
import type { TabRecord } from '../../stores/tabsStore';
import SignaturesPanel from './SignaturesPanel';
import {
  clearSignatures,
  documentSignatures,
  knownSignatures,
  statusLine,
  worstStatus,
} from './signatureStore';
import { SIGNATURE_COMMANDS, toggleSignaturesPanel } from './signatureCommands';
import type { SignatureReport } from './types';

function report(over: Partial<SignatureReport> = {}): SignatureReport {
  return {
    fieldName: 'Signature1',
    page: 1,
    rect: { x: 72, y: 100, w: 200, h: 50 },
    kind: 'approval',
    subFilter: 'adbe.pkcs7.detached',
    status: 'valid',
    summary: 'Signed by a trusted certificate; the signed bytes are intact.',
    signer: {
      name: 'Ada Lovelace',
      email: 'ada@example.com',
      organization: null,
      issuer: 'Test CA',
    },
    time: '2026-11-17 12:00 UTC',
    timeSource: 'signer',
    timestamp: null,
    intact: true,
    problem: null,
    coversWholeFile: true,
    revision: 1,
    revisions: 1,
    trust: 'trusted',
    trustDetail: null,
    certification: null,
    locks: null,
    reason: null,
    location: null,
    digest: 'SHA-256',
    changes: [],
    ...over,
  };
}

afterEach(() => {
  cleanup();
  clearSignatures();
  vi.restoreAllMocks();
});

describe('status line', () => {
  it('says nothing for an unsigned document', () => {
    expect(statusLine([])).toBeNull();
    expect(worstStatus([])).toBeNull();
  });

  it('reports the worst signature', () => {
    expect(statusLine([report()])).toBe('Signed - valid');
    expect(statusLine([report(), report({ status: 'identityUnknown' })])).toBe(
      'Signed - valid, identity unknown'
    );
    expect(statusLine([report({ status: 'modified' }), report({ status: 'invalid' })])).toBe(
      'Signed - invalid'
    );
    expect(statusLine([report({ status: 'modified' })])).toBe('Signed - modified after signing');
  });
});

describe('store', () => {
  it('asks once per document and remembers the answer', async () => {
    const check = vi.spyOn(engine, 'checkSignatures').mockResolvedValue([report()]);
    expect(knownSignatures(4)).toEqual([]);
    await documentSignatures(4, '/tmp/a.pdf');
    await documentSignatures(4, '/tmp/a.pdf');
    expect(check).toHaveBeenCalledTimes(1);
    expect(knownSignatures(4)).toHaveLength(1);
    clearSignatures(4);
    expect(knownSignatures(4)).toEqual([]);
  });

  it('treats an unsaved document and a failed check as unsigned', async () => {
    vi.spyOn(engine, 'checkSignatures').mockRejectedValue(new Error('gone'));
    expect(await documentSignatures(5, null)).toEqual([]);
    expect(await documentSignatures(6, '/tmp/b.pdf')).toEqual([]);
  });
});

const meta = {
  docId: 7,
  path: '/tmp/signed.pdf',
  name: 'signed.pdf',
  pageCount: 2,
  sizes: [
    [612, 792, 0, 0, 0],
    [612, 792, 0, 0, 0],
  ] as [number, number, number, number, number][],
  estimatedSize: [612, 792] as [number, number],
};

function mount() {
  const documentStore = createDocumentStore();
  documentStore.initFromMeta(meta);
  const viewport = createViewportStore();
  const tab = { id: 'tab-1', documentStore, viewport } as unknown as TabRecord;
  const view = render(() => (
    <TabContext.Provider value={tab}>
      <SignaturesPanel />
    </TabContext.Provider>
  ));
  return { view, viewport, tab };
}

describe('panel', () => {
  it('lists each signature with its status, signer and later changes', async () => {
    vi.spyOn(engine, 'checkSignatures').mockResolvedValue([
      report(),
      report({
        fieldName: 'Signature2',
        status: 'modified',
        signer: { name: 'Bob', email: null, organization: null, issuer: null },
        changes: [
          { kind: 'formFill', allowed: true, page: null, detail: 'Filled in 1 form field (Name)' },
          { kind: 'pageContent', allowed: false, page: 0, detail: 'Page content was changed' },
        ],
      }),
    ]);
    const { view } = mount();
    expect(await view.findByText('Ada Lovelace')).toBeTruthy();
    expect(view.getByText('Valid')).toBeTruthy();
    expect(view.getByText('Modified after signing')).toBeTruthy();
    expect(view.getByText('Page content was changed (page 1) - not allowed')).toBeTruthy();
    expect(view.getByText('Filled in 1 form field (Name)')).toBeTruthy();
    expect(view.getAllByText('Certificate trusted by this Mac')).toHaveLength(2);
  });

  it('goes to the signature on the page', async () => {
    vi.spyOn(engine, 'checkSignatures').mockResolvedValue([report()]);
    const { view, viewport } = mount();
    const scroll = vi.spyOn(viewport, 'requestScrollToPage');
    fireEvent.click((await view.findByText('Ada Lovelace')).closest('button')!);
    expect(scroll).toHaveBeenCalledWith(1, expect.any(Number));
  });

  it('says so when there are none', async () => {
    vi.spyOn(engine, 'checkSignatures').mockResolvedValue([]);
    const { view } = mount();
    expect(await view.findByText('This document has no digital signatures.')).toBeTruthy();
  });
});

describe('command', () => {
  it('is offered only for a signed document and swaps out the form panel', async () => {
    vi.spyOn(engine, 'checkSignatures').mockResolvedValue([report()]);
    const { tab, viewport } = mount();
    const command = SIGNATURE_COMMANDS[0]!;
    expect(command.enabled({ tab })).toBe(false);
    await documentSignatures(7, '/tmp/signed.pdf');
    expect(command.enabled({ tab })).toBe(true);
    viewport.setState('formPanelOpen', true);
    toggleSignaturesPanel(tab);
    expect(viewport.state.signaturesPanelOpen).toBe(true);
    expect(viewport.state.formPanelOpen).toBe(false);
  });
});
