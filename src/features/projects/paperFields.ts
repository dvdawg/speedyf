/** Pure helpers for the detail column: how an Author[] round-trips through a
 * textarea, how a typed year is read, and what a provenance source is called
 * on screen. Kept out of the component so they can be tested as functions. */
import type { Author, MetaSource, PaperKind, PaperRow } from '../../types/engine';

/** What each provenance is called on screen. A Record over the union, so a
 * source the engine grows is a type error here before it is a blank tag. */
export const SOURCE_LABEL: Record<MetaSource, string> = {
  user: 'edited',
  crossref: 'Crossref',
  inspire: 'INSPIRE',
  datacite: 'DataCite',
  arxiv: 'arXiv',
  'semantic-scholar': 'Semantic Scholar',
  dblp: 'DBLP',
  openalex: 'OpenAlex',
  'open-library': 'Open Library',
  xmp: 'XMP',
  identifier: 'identifier',
  'pdf-info': 'PDF info',
  citations: 'citing papers',
  'first-page': 'first page',
  filename: 'file name',
};

export const KINDS: PaperKind[] = ['article', 'inproceedings', 'book', 'thesis', 'misc'];

/** One author per line, "Family, Given". */
export function formatAuthors(authors: Author[]): string {
  return authors.map((a) => (a.given ? `${a.family}, ${a.given}` : a.family)).join('\n');
}

/** A line without a comma is a family name alone: better than guessing which
 * of "Jean Pierre Dupont"'s words are the given names. */
export function parseAuthors(text: string): Author[] {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const comma = line.indexOf(',');
      if (comma < 0) return { family: line, given: '' };
      return { family: line.slice(0, comma).trim(), given: line.slice(comma + 1).trim() };
    });
}

/** "Given Family, Given Family" for a candidate's one-line summary. */
export function authorsLine(authors: Author[]): string {
  return authors.map((a) => [a.given, a.family].filter(Boolean).join(' ')).join(', ');
}

/** Empty clears, digits are a year, anything else is refused rather than
 * silently dropped. */
export function parseYear(text: string): number | null | 'invalid' {
  const trimmed = text.trim();
  if (!trimmed) return null;
  return /^\d{1,4}$/.test(trimmed) ? Number(trimmed) : 'invalid';
}

/** The year as shown: the year, else "2019+" when the item's references
 * say it is at least that new, else ''. Never a year for BibTeX. */
export function yearText(item: {
  year?: number | null | undefined;
  yearFloor?: number | null | undefined;
}): string {
  if (item.year !== null && item.year !== undefined) return String(item.year);
  if (item.yearFloor !== null && item.yearFloor !== undefined) return `${item.yearFloor}+`;
  return '';
}

/** A field's provenance tag: the source's name, or for a value citing
 * papers agreed on, how many ("from 3 citing papers"). */
export function sourceText(source: MetaSource, vouched?: number): string {
  if (source === 'citations' && vouched !== undefined && vouched > 0) {
    return `from ${vouched} citing paper${vouched === 1 ? '' : 's'}`;
  }
  return SOURCE_LABEL[source];
}

/** The dimmed second line of a list row. */
export function metaLine(row: PaperRow): string {
  return [row.authors, yearText(row), row.venue]
    .filter((part) => part !== null && part !== '')
    .map(String)
    .join(', ');
}
