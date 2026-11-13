/** The Files list of the detail column: byte-identical copies of one PDF
 * (the engine marks each with `duplicateOf`) fold into one row. */
import type { PaperFile } from '../../types/engine';

/** One row: a file, and the copies of it the engine grouped under it. */
export interface FileRow {
  file: PaperFile;
  copies: PaperFile[];
}

/** The item's files as rows, in the item's order (the main one first): a
 * copy joins the row of the file that stands for it, and one whose file is
 * not in the list keeps a row of its own. */
export function fileRows(files: PaperFile[]): FileRow[] {
  const rows: FileRow[] = files
    .filter((file) => !file.duplicateOf)
    .map((file) => ({ file, copies: [] }));
  for (const file of files) {
    if (!file.duplicateOf) continue;
    const row = rows.find((r) => r.file.path === file.duplicateOf);
    if (row) row.copies.push(file);
    else rows.push({ file, copies: [] });
  }
  return rows;
}
