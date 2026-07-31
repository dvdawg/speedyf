import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { clearImagePreviews, imagePreviewUrl } from './editorActions';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('image preview URL lifecycle', () => {
  const createObjectURL = vi.fn(() => 'blob:speedyf-preview');
  const revokeObjectURL = vi.fn();

  beforeEach(() => {
    clearImagePreviews();
    vi.mocked(invoke).mockReset();
    createObjectURL.mockClear();
    revokeObjectURL.mockClear();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
  });

  it('coalesces concurrent reads and revokes the one cached URL', async () => {
    const bytes = deferred<ArrayBuffer>();
    vi.mocked(invoke).mockReturnValue(bytes.promise);

    const first = imagePreviewUrl('/tmp/image.png');
    const second = imagePreviewUrl('/tmp/image.png');
    expect(invoke).toHaveBeenCalledTimes(1);

    bytes.resolve(new Uint8Array([1, 2, 3]).buffer);
    await expect(first).resolves.toBe('blob:speedyf-preview');
    await expect(second).resolves.toBe('blob:speedyf-preview');
    expect(createObjectURL).toHaveBeenCalledTimes(1);

    clearImagePreviews();
    expect(revokeObjectURL).toHaveBeenCalledOnce();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:speedyf-preview');
  });

  it('does not create a URL after document invalidation', async () => {
    const bytes = deferred<ArrayBuffer>();
    vi.mocked(invoke).mockReturnValue(bytes.promise);

    const pending = imagePreviewUrl('/tmp/slow.png');
    clearImagePreviews();
    bytes.resolve(new Uint8Array([4, 5, 6]).buffer);

    await expect(pending).rejects.toThrow('invalidated');
    expect(createObjectURL).not.toHaveBeenCalled();
  });
});
