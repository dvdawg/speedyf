/** Sign with a certificate: choose who signs (or make or import an
 * identity), where the signature goes, what it says, and how the file is
 * saved. Restrained on purpose: the defaults sign a document the way most
 * readers expect, and everything else sits under Options. */
import { createResource, createSignal, For, Show } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { activeTab } from '../../../stores/tabsStore';
import { settings, updateSettings } from '../../../stores/settings';
import { defaultSignature, loadSignatures } from '../signStore';
import type { Identity, LockChoice } from './api';
import {
  certState,
  closeCertDialog,
  createIdentity,
  errorText,
  importIdentity,
  lockableFields,
  openSignatureFields,
  setCertError,
  setCertTarget,
  signDocument,
  startPlacing,
  usesKeychain,
} from './certStore';
import './certificate.css';
import Sheet from '../../../components/Sheet';

type Panel = 'none' | 'create' | 'import';

const dateOf = (secs: number) => new Date(secs * 1000).toLocaleDateString();

function IdentityRow(props: { identity: Identity; selected: boolean; onSelect: () => void }) {
  const expired = () => props.identity.notAfter * 1000 < Date.now();
  return (
    <label class="cert-identity" classList={{ 'is-selected': props.selected }}>
      <input
        type="radio"
        name="cert-identity"
        checked={props.selected}
        onChange={() => props.onSelect()}
      />
      <span class="cert-identity-text">
        <span class="cert-identity-name">{props.identity.name}</span>
        <span class="cert-identity-meta">
          {props.identity.email ? `${props.identity.email} - ` : ''}
          {props.identity.selfSigned ? 'self-signed' : `issued by ${props.identity.issuer}`}
          {' - '}
          {expired() ? 'expired ' : 'valid until '}
          {dateOf(props.identity.notAfter)}
        </span>
      </span>
    </label>
  );
}

export default function CertSignDialog() {
  const tab = () => activeTab();
  const doc = () => tab()?.documentStore.state;
  const docId = () => doc()?.docId ?? -1;

  const [identityId, setIdentityId] = createSignal<string | null>(null);
  const [passphrase, setPassphrase] = createSignal('');
  const [panel, setPanel] = createSignal<Panel>('none');
  const [busyPanel, setBusyPanel] = createSignal(false);
  // create
  const [newName, setNewName] = createSignal('');
  const [newEmail, setNewEmail] = createSignal('');
  const [newOrg, setNewOrg] = createSignal('');
  const [newPass, setNewPass] = createSignal('');
  const [newPass2, setNewPass2] = createSignal('');
  // import
  const [p12Path, setP12Path] = createSignal<string | null>(null);
  const [p12Password, setP12Password] = createSignal('');
  // details
  const [reason, setReason] = createSignal('');
  const [location, setLocation] = createSignal('');
  const [contact, setContact] = createSignal('');
  const [showArt, setShowArt] = createSignal(false);
  // options
  const [certify, setCertify] = createSignal<0 | 1 | 2 | 3>(0);
  const [lockKind, setLockKind] = createSignal<'none' | 'all' | 'include'>('none');
  const [locked, setLocked] = createSignal<string[]>([]);
  const [format, setFormat] = createSignal<'cades' | 'pkcs7'>('cades');
  const [saveAs, setSaveAs] = createSignal(false);

  const [openFields] = createResource(docId, (id) =>
    id >= 0 ? openSignatureFields(id) : Promise.resolve([])
  );
  const [lockable] = createResource(docId, (id) =>
    id >= 0 ? lockableFields(id) : Promise.resolve([])
  );
  const [art] = createResource(async () => {
    await loadSignatures();
    return defaultSignature('signature');
  });

  const identities = () => certState.identities;
  const selected = () =>
    identities().find((i) => i.id === identityId()) ??
    (identities().length === 1 ? identities()[0] : undefined);

  const pageLabel = (index: number) => `page ${index + 1}`;
  const targetLabel = () => {
    const t = certState.target;
    if (!t) return null;
    return t.kind === 'existing'
      ? `Field "${t.name}" on ${pageLabel(t.pageIndex)}`
      : `New box on ${pageLabel(t.pageIndex)}`;
  };

  const pageIndexOf = (src: number) => {
    const at = doc()?.pages.findIndex((p) => p.srcIndex === src) ?? -1;
    return at < 0 ? src : at;
  };

  const timestampReady = () =>
    !settings.timestampOnline || /^https?:\/\/\S+$/i.test(settings.timestampUrl.trim());

  const canSign = () =>
    !certState.busy &&
    !!selected() &&
    !!certState.target &&
    (!selected()!.needsPassphrase || passphrase() !== '') &&
    timestampReady() &&
    (lockKind() !== 'include' || locked().length > 0);

  // ---------- identities ----------

  const create = async () => {
    if (!newName().trim()) return;
    if (!usesKeychain() && (newPass() === '' || newPass() !== newPass2())) {
      setCertError('Enter the same passphrase twice.');
      return;
    }
    setBusyPanel(true);
    setCertError(null);
    try {
      const made = await createIdentity({
        name: newName(),
        email: newEmail(),
        organization: newOrg(),
        passphrase: usesKeychain() ? null : newPass(),
      });
      setIdentityId(made.id);
      if (!usesKeychain()) setPassphrase(newPass());
      setPanel('none');
    } catch (e) {
      setCertError(`Could not create the identity: ${errorText(e)}`);
    } finally {
      setBusyPanel(false);
    }
  };

  const pickP12 = async () => {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: 'Certificate files', extensions: ['p12', 'pfx'] }],
    });
    if (typeof picked === 'string') setP12Path(picked);
  };

  const doImport = async () => {
    const path = p12Path();
    if (!path) return;
    if (!usesKeychain() && (newPass() === '' || newPass() !== newPass2())) {
      setCertError('Enter the same passphrase twice.');
      return;
    }
    setBusyPanel(true);
    setCertError(null);
    try {
      const made = await importIdentity({
        path,
        password: p12Password(),
        passphrase: usesKeychain() ? null : newPass(),
      });
      setIdentityId(made.id);
      if (!usesKeychain()) setPassphrase(newPass());
      setPanel('none');
      setP12Path(null);
      setP12Password('');
    } catch (e) {
      setCertError(`Could not import: ${errorText(e)}`);
    } finally {
      setBusyPanel(false);
    }
  };

  // ---------- signing ----------

  const sign = async () => {
    const who = selected();
    if (!canSign() || !who) return;
    const lock: LockChoice =
      lockKind() === 'include'
        ? { kind: 'include', fields: locked() }
        : lockKind() === 'all'
          ? { kind: 'all' }
          : { kind: 'none' };
    const saved = showArt() ? art() : null;
    await signDocument(tab(), {
      identity: who.id,
      passphrase: who.needsPassphrase ? passphrase() : null,
      saveAs: saveAs(),
      request: {
        reason: reason().trim(),
        location: location().trim(),
        contact: contact().trim(),
        art: saved ? { art: saved.art, color: saved.color } : null,
        certify: certify() === 0 ? null : (certify() as 1 | 2 | 3),
        lock,
        subFilter: format(),
        tsaUrl: settings.timestampOnline ? settings.timestampUrl.trim() : null,
      },
    });
  };

  return (
    <Sheet
      label="Sign with certificate"
      class="cert-dialog"
      onCancel={() => {
        if (!certState.busy) closeCertDialog();
      }}
    >
      <h2>Sign with certificate</h2>
      <p class="cert-intro">
        A certificate signature seals the file as it is now. Readers can check who signed it and see
        any change made afterwards.
      </p>

      {/* ---------- who signs ---------- */}
      <section class="cert-section" aria-label="Sign as">
        <h3>Sign as</h3>
        <Show
          when={identities().length > 0}
          fallback={
            <p class="cert-empty">
              {certState.loaded ? 'No signing identity yet.' : 'Looking for identities...'}
            </p>
          }
        >
          <div class="cert-identities" role="radiogroup" aria-label="Identity">
            <For each={identities()}>
              {(identity) => (
                <IdentityRow
                  identity={identity}
                  selected={selected()?.id === identity.id}
                  onSelect={() => setIdentityId(identity.id)}
                />
              )}
            </For>
          </div>
        </Show>
        <Show when={selected()?.needsPassphrase}>
          <input
            class="cert-input"
            type="password"
            placeholder="Passphrase for this identity"
            aria-label="Passphrase"
            value={passphrase()}
            onInput={(e) => setPassphrase(e.currentTarget.value)}
          />
        </Show>
        <div class="cert-row">
          <button
            type="button"
            class="secondary-btn"
            aria-expanded={panel() === 'create'}
            onClick={() => setPanel(panel() === 'create' ? 'none' : 'create')}
          >
            Create self-signed...
          </button>
          <button
            type="button"
            class="secondary-btn"
            aria-expanded={panel() === 'import'}
            onClick={() => setPanel(panel() === 'import' ? 'none' : 'import')}
          >
            Import .p12...
          </button>
        </div>

        <Show when={panel() === 'create'}>
          <div class="cert-panel">
            <input
              class="cert-input"
              type="text"
              placeholder="Name"
              aria-label="Name"
              value={newName()}
              onInput={(e) => setNewName(e.currentTarget.value)}
            />
            <input
              class="cert-input"
              type="email"
              placeholder="Email (optional)"
              aria-label="Email"
              value={newEmail()}
              onInput={(e) => setNewEmail(e.currentTarget.value)}
            />
            <input
              class="cert-input"
              type="text"
              placeholder="Organization (optional)"
              aria-label="Organization"
              value={newOrg()}
              onInput={(e) => setNewOrg(e.currentTarget.value)}
            />
            <Show when={!usesKeychain()}>
              <input
                class="cert-input"
                type="password"
                placeholder="Passphrase to protect the key"
                aria-label="New passphrase"
                value={newPass()}
                onInput={(e) => setNewPass(e.currentTarget.value)}
              />
              <input
                class="cert-input"
                type="password"
                placeholder="Passphrase again"
                aria-label="Repeat passphrase"
                value={newPass2()}
                onInput={(e) => setNewPass2(e.currentTarget.value)}
              />
            </Show>
            <p class="cert-note">
              A self-signed certificate shows that the file was not changed after signing. It does
              not prove who you are to others unless they choose to trust it.{' '}
              {usesKeychain()
                ? 'It is kept in your Keychain.'
                : 'It is kept on this computer, the key encrypted with your passphrase.'}
            </p>
            <div class="cert-row">
              <button
                type="button"
                class="primary-btn"
                disabled={busyPanel() || !newName().trim()}
                onClick={() => void create()}
              >
                Create
              </button>
            </div>
          </div>
        </Show>

        <Show when={panel() === 'import'}>
          <div class="cert-panel">
            <div class="cert-row">
              <button type="button" class="secondary-btn" onClick={() => void pickP12()}>
                {p12Path() ? 'Choose another file...' : 'Choose file...'}
              </button>
              <span class="cert-file">{p12Path()?.split(/[\\/]/).pop() ?? ''}</span>
            </div>
            <input
              class="cert-input"
              type="password"
              placeholder="Password of the .p12 file"
              aria-label="File password"
              value={p12Password()}
              onInput={(e) => setP12Password(e.currentTarget.value)}
            />
            <Show when={!usesKeychain()}>
              <input
                class="cert-input"
                type="password"
                placeholder="Passphrase to protect the key here"
                aria-label="New passphrase"
                value={newPass()}
                onInput={(e) => setNewPass(e.currentTarget.value)}
              />
              <input
                class="cert-input"
                type="password"
                placeholder="Passphrase again"
                aria-label="Repeat passphrase"
                value={newPass2()}
                onInput={(e) => setNewPass2(e.currentTarget.value)}
              />
            </Show>
            <p class="cert-note">
              {usesKeychain()
                ? 'The identity is added to your Keychain.'
                : 'The key is kept on this computer, encrypted with your passphrase.'}
            </p>
            <div class="cert-row">
              <button
                type="button"
                class="primary-btn"
                disabled={busyPanel() || !p12Path()}
                onClick={() => void doImport()}
              >
                Import
              </button>
            </div>
          </div>
        </Show>
      </section>

      {/* ---------- where ---------- */}
      <section class="cert-section" aria-label="Signature field">
        <h3>Where</h3>
        <Show
          when={certState.target}
          fallback={
            <div class="cert-fields">
              <For each={openFields() ?? []}>
                {(f) => (
                  <button
                    type="button"
                    class="secondary-btn"
                    onClick={() =>
                      setCertTarget({
                        kind: 'existing',
                        name: f.name,
                        pageIndex: pageIndexOf(f.page),
                      })
                    }
                  >
                    {f.tooltip || f.name} ({pageLabel(pageIndexOf(f.page))})
                  </button>
                )}
              </For>
              <button type="button" class="secondary-btn" onClick={() => startPlacing()}>
                Draw a box on the page...
              </button>
            </div>
          }
        >
          <div class="cert-row">
            <span class="cert-target">{targetLabel()}</span>
            <button type="button" class="secondary-btn" onClick={() => setCertTarget(null)}>
              Change
            </button>
          </div>
        </Show>
      </section>

      {/* ---------- what it says ---------- */}
      <section class="cert-section" aria-label="Details">
        <h3>Details</h3>
        <input
          class="cert-input"
          type="text"
          placeholder="Reason (optional), for example: I approve this document"
          aria-label="Reason"
          value={reason()}
          onInput={(e) => setReason(e.currentTarget.value)}
        />
        <div class="cert-pair">
          <input
            class="cert-input"
            type="text"
            placeholder="Location (optional)"
            aria-label="Location"
            value={location()}
            onInput={(e) => setLocation(e.currentTarget.value)}
          />
          <input
            class="cert-input"
            type="text"
            placeholder="Contact (optional)"
            aria-label="Contact"
            value={contact()}
            onInput={(e) => setContact(e.currentTarget.value)}
          />
        </div>
        <Show when={art()}>
          <label class="cert-check">
            <input
              type="checkbox"
              checked={showArt()}
              onChange={(e) => setShowArt(e.currentTarget.checked)}
            />
            Show my visual signature beside the details
          </label>
        </Show>
      </section>

      {/* ---------- options ---------- */}
      <details class="cert-options">
        <summary>Options</summary>
        <label class="cert-field">
          <span>Certify</span>
          <select
            value={String(certify())}
            onChange={(e) => setCertify(Number(e.currentTarget.value) as 0 | 1 | 2 | 3)}
          >
            <option value="0">Do not certify</option>
            <option value="1">Certify: no changes allowed</option>
            <option value="2">Certify: allow form filling and signing</option>
            <option value="3">Certify: also allow comments</option>
          </select>
        </label>
        <label class="cert-field">
          <span>After signing</span>
          <select
            value={lockKind()}
            onChange={(e) => setLockKind(e.currentTarget.value as 'none' | 'all' | 'include')}
          >
            <option value="none">Leave fields editable</option>
            <option value="all">Lock all fields</option>
            <option value="include" disabled={(lockable() ?? []).length === 0}>
              Lock chosen fields
            </option>
          </select>
        </label>
        <Show when={lockKind() === 'include'}>
          <div class="cert-locks">
            <For each={lockable() ?? []}>
              {(name) => (
                <label class="cert-check">
                  <input
                    type="checkbox"
                    checked={locked().includes(name)}
                    onChange={(e) =>
                      setLocked(
                        e.currentTarget.checked
                          ? [...locked(), name]
                          : locked().filter((n) => n !== name)
                      )
                    }
                  />
                  {name}
                </label>
              )}
            </For>
          </div>
        </Show>
        <label class="cert-field">
          <span>Format</span>
          <select
            value={format()}
            onChange={(e) => setFormat(e.currentTarget.value as 'cades' | 'pkcs7')}
          >
            <option value="cades">PAdES (recommended)</option>
            <option value="pkcs7">PKCS #7 (older readers)</option>
          </select>
        </label>
        <label class="cert-check">
          <input
            type="checkbox"
            checked={settings.timestampOnline}
            onChange={(e) => updateSettings({ timestampOnline: e.currentTarget.checked })}
          />
          Add a timestamp from a server (connects to the internet)
        </label>
        <Show when={settings.timestampOnline}>
          <input
            class="cert-input"
            type="url"
            placeholder="Time-stamping server, for example https://tsa.example.org"
            aria-label="Time-stamping server"
            value={settings.timestampUrl}
            onInput={(e) => updateSettings({ timestampUrl: e.currentTarget.value })}
          />
          <p class="cert-note">
            Only the hash of the signature is sent, to this address and nowhere else.
          </p>
        </Show>
      </details>

      {/* ---------- saving ---------- */}
      <section class="cert-section" aria-label="Save">
        <div class="cert-row" role="radiogroup" aria-label="Where to save">
          <label class="cert-check">
            <input
              type="radio"
              name="cert-save"
              checked={!saveAs()}
              disabled={!doc()?.path}
              onChange={() => setSaveAs(false)}
            />
            Save in this file
          </label>
          <label class="cert-check">
            <input
              type="radio"
              name="cert-save"
              checked={saveAs() || !doc()?.path}
              onChange={() => setSaveAs(true)}
            />
            Save as a new file...
          </label>
        </div>
        <Show when={doc()?.dirty}>
          <p class="cert-note">Your unsaved changes are saved first, then signed.</p>
        </Show>
      </section>

      <Show when={certState.error}>
        <p class="cert-error" role="alert">
          {certState.error}
        </p>
      </Show>
      <div class="modal-actions">
        <button
          type="button"
          class="secondary-btn"
          disabled={certState.busy}
          onClick={() => closeCertDialog()}
        >
          Cancel
        </button>
        <button type="button" class="primary-btn" disabled={!canSign()} onClick={() => void sign()}>
          {certState.busy ? 'Signing...' : 'Sign'}
        </button>
      </div>
    </Sheet>
  );
}
