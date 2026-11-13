/** Export bundle's inline result: what the notice bar says once the zip is
 * written, and that a failed export says nothing of the kind. */
import { afterEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  exportBundle: vi.fn(),
}));

vi.mock('../../lib/transport/engine', () => ({
  engine: { libraryExportBundle: h.exportBundle },
  isEngineError: () => false,
  onLibraryChanged: vi.fn(),
}));

const { bundleSummary, projectsStore } = await import('./projectsStore');

const report = (pdfs: number, missing: string[]) => ({
  path: '/out/Thesis.zip',
  bibName: 'Thesis.bib',
  pdfs: Array.from({ length: pdfs }, (_, i) => `pdfs/key${i}.pdf`),
  missing,
});

afterEach(() => {
  projectsStore.clearNotice();
  projectsStore.clearError();
  vi.clearAllMocks();
});

describe('bundleSummary', () => {
  it('counts the PDFs and names the missing items by key', () => {
    expect(bundleSummary(report(23, ['key1', 'key2']))).toBe(
      'Exported 23 PDFs and the .bib; 2 missing: key1, key2'
    );
    expect(bundleSummary(report(1, []))).toBe('Exported 1 PDF and the .bib');
    expect(bundleSummary(report(0, []))).toBe('Exported 0 PDFs and the .bib');
  });

  it('names a long missing list only in part', () => {
    const missing = Array.from({ length: 10 }, (_, i) => `k${i}`);
    expect(bundleSummary(report(2, missing))).toBe(
      'Exported 2 PDFs and the .bib; 10 missing: k0, k1, k2, k3, k4, k5, k6, k7 and 2 more'
    );
  });
});

describe('projectsStore.exportBundle', () => {
  it('shows the result inline', async () => {
    h.exportBundle.mockResolvedValueOnce(report(3, ['gone2020']));
    await projectsStore.exportBundle('p1', '/out/Thesis.zip');
    expect(h.exportBundle).toHaveBeenCalledWith('p1', '/out/Thesis.zip');
    expect(projectsStore.state.notice).toBe('Exported 3 PDFs and the .bib; 1 missing: gone2020');
  });

  it('reports a failure as an error and no notice', async () => {
    h.exportBundle.mockRejectedValueOnce(new Error('cannot write in /out'));
    await projectsStore.exportBundle('p1', '/out/Thesis.zip');
    expect(projectsStore.state.notice).toBeNull();
    expect(projectsStore.state.error).toMatch(/cannot write/);
  });
});
