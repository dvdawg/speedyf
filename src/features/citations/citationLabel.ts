import type { CitationId } from '../../types/engine';

export function citationLabel(id: CitationId): string {
  return id.scheme === 'doi' ? `doi:${id.value}` : `arXiv:${id.value}`;
}

export function citationKey(id: CitationId): string {
  return `${id.scheme}:${id.value}`;
}
