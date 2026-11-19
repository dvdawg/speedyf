// @vitest-environment happy-dom
/**
 * Every confirm and choice dialog is a sheet with the same keys: Return
 * presses the default button, Escape cancels, Tab stays inside, focus goes
 * back where it was - and an action that cannot be undone is never what
 * Return presses.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { createSignal, Show } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Sheet from './Sheet';
import Modals from './Modals';
import { askConfirm, askUnsaved, modal, resolveConfirm } from '../stores/modalStore';

afterEach(() => {
  if (modal.kind === 'confirm') resolveConfirm(false);
  cleanup();
});

const settle = () => new Promise((r) => setTimeout(r, 0));
const key = (target: Element, k: string, extra: KeyboardEventInit = {}) =>
  fireEvent.keyDown(target, { key: k, ...extra });

describe('Sheet', () => {
  const setup = () => {
    const cancel = vi.fn();
    const ok = vi.fn();
    const other = vi.fn();
    const [open, setOpen] = createSignal(false);
    render(() => (
      <>
        <button type="button">Opener</button>
        <Show when={open()}>
          <Sheet label="Test sheet" onCancel={cancel}>
            <label>
              <input type="checkbox" /> Option
            </label>
            <div class="modal-actions">
              <button type="button" class="secondary-btn" onClick={other}>
                Other
              </button>
              <button type="button" class="primary-btn" onClick={ok}>
                OK
              </button>
            </div>
          </Sheet>
        </Show>
      </>
    ));
    const opener = screen.getByRole('button', { name: 'Opener' });
    opener.focus();
    setOpen(true);
    return { cancel, ok, other, opener, close: () => setOpen(false) };
  };

  it('opens on the default button and gives focus back on close', async () => {
    const { opener, close } = setup();
    await settle();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'OK' }));
    expect(screen.getByRole('dialog', { name: 'Test sheet' }).getAttribute('aria-modal')).toBe(
      'true'
    );
    close();
    await settle();
    expect(document.activeElement).toBe(opener);
  });

  it('presses the default button on Return from anywhere but another button', async () => {
    const { ok, other } = setup();
    await settle();
    key(screen.getByRole('checkbox'), 'Enter');
    expect(ok).toHaveBeenCalledTimes(1);
    // A focused button keeps Return for itself (the browser clicks it).
    key(screen.getByRole('button', { name: 'Other' }), 'Enter');
    expect(ok).toHaveBeenCalledTimes(1);
    expect(other).not.toHaveBeenCalled();
  });

  it('cancels on Escape', async () => {
    const { cancel } = setup();
    await settle();
    key(screen.getByRole('checkbox'), 'Escape');
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it('keeps Tab inside', async () => {
    setup();
    await settle();
    const box = screen.getByRole('checkbox');
    const ok = screen.getByRole('button', { name: 'OK' });
    key(ok, 'Tab');
    expect(document.activeElement).toBe(box);
    key(box, 'Tab', { shiftKey: true });
    expect(document.activeElement).toBe(ok);
  });
});

describe('every dialog is a sheet', () => {
  it('leaves no hand-rolled modal backdrop outside Sheet (palettes aside)', () => {
    const files = (dir: string): string[] =>
      readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
        e.isDirectory()
          ? files(join(dir, e.name))
          : e.name.endsWith('.tsx') && !e.name.endsWith('.test.tsx')
            ? [join(dir, e.name)]
            : []
      );
    const offenders = files(join(process.cwd(), 'src')).filter((path) =>
      /class="modal-backdrop(?! palette-backdrop)(?! sheet-backdrop)/.test(
        readFileSync(path, 'utf8')
      )
    );
    expect(offenders).toEqual([]);
  });
});

describe('Modals as sheets', () => {
  it('makes Cancel the default of an irreversible confirm', async () => {
    render(() => <Modals />);
    const answer = askConfirm('Apply 2 redaction marks?', 'Apply redactions', {
      destructive: true,
    });
    await settle();
    const apply = screen.getByRole('button', { name: 'Apply redactions' });
    expect(apply.className).toContain('destructive-btn');
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    expect(document.activeElement).toBe(cancel);
    key(screen.getByRole('dialog'), 'Enter');
    await expect(answer).resolves.toBe(false);
  });

  it('confirms an ordinary choice on Return', async () => {
    render(() => <Modals />);
    const answer = askConfirm('Save a copy?', 'Save a Copy...');
    await settle();
    key(screen.getByRole('dialog'), 'Enter');
    await expect(answer).resolves.toBe(true);
  });

  it('saves on Return and cancels on Escape in the unsaved-changes sheet', async () => {
    render(() => <Modals />);
    const first = askUnsaved('Save changes to a.pdf?');
    await settle();
    expect(screen.getByRole('button', { name: 'Discard changes' }).className).toContain(
      'destructive-btn'
    );
    key(screen.getByRole('dialog'), 'Enter');
    await expect(first).resolves.toBe('save');
    const second = askUnsaved('Save changes to b.pdf?');
    await settle();
    key(screen.getByRole('dialog'), 'Escape');
    await expect(second).resolves.toBe('cancel');
  });
});
