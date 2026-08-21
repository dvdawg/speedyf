/** Annotations as Markdown.
 *
 * The point of a notes panel is that the notes leave — into a paper you are
 * writing, a reading log, wherever. So the export is plain Markdown rather than
 * anything SpeedyF-shaped: a highlight becomes a blockquote, whatever you wrote
 * about it follows underneath, and the page number goes on the quote so you can
 * find it again.
 *
 * Annotations with neither text of their own nor text underneath them — an
 * empty rectangle, a stray ink stroke — carry nothing a reader could use, so
 * they are counted in the panel but left out of the export.
 */
import type { Annotation } from '../../types/model';

export interface ExportEntry {
  annot: Annotation;
  /** 1-based page number, as shown in the panel. */
  page: number;
  /** The text underneath a highlight, once recovered. */
  quoted?: string;
}

/** A readable name for each kind, for annotations that have no quote. */
const KIND_LABEL: Record<string, string> = {
  highlight: 'Highlight',
  ink: 'Drawing',
  rect: 'Box',
  note: 'Note',
  textbox: 'Text',
  image: 'Image',
};

function entryToMarkdown(entry: ExportEntry): string | null {
  const quoted = entry.quoted?.trim();
  const own = entry.annot.text?.trim();
  if (!quoted && !own) return null;

  const lines: string[] = [];
  if (quoted) {
    // Blockquote every line, so a multi-line quote stays one quote.
    for (const line of quoted.split('\n')) lines.push(`> ${line}`);
    lines.push(`>`);
    lines.push(`> — p${entry.page}`);
  } else {
    lines.push(`**${KIND_LABEL[entry.annot.kind] ?? 'Annotation'}** — p${entry.page}`);
  }
  if (own) {
    if (quoted) lines.push('');
    lines.push(own);
  }
  return lines.join('\n');
}

/** Every annotation worth exporting, as one Markdown document.
 *
 * `title` becomes the heading, so a pasted export says which paper it came
 * from — the single most useful thing to know about a page of quotes. */
export function annotationsToMarkdown(title: string, entries: readonly ExportEntry[]): string {
  const blocks = entries
    .slice()
    .sort((a, b) => a.page - b.page)
    .map(entryToMarkdown)
    .filter((block): block is string => block !== null);

  const heading = title.trim();
  if (blocks.length === 0) {
    return heading ? `# ${heading}\n\nNo notes yet.\n` : 'No notes yet.\n';
  }
  const body = blocks.join('\n\n');
  return heading ? `# ${heading}\n\n${body}\n` : `${body}\n`;
}
