/** A note's references: one section at its end, a heading named
 * "References" and the Pandoc footnote definitions under it
 * (`[^simon2015almost]: B. Simon, ...`), shared by every quote in the
 * note. Read and rewritten here without a DOM, so it is tested on its own.
 *
 * A quote's markers (`[^simon2015almost]{label=Sim15}`) name an entry by an
 * id made from the work (first author, year, first title word); two quotes
 * citing the same work - even from papers that print it differently - share
 * one entry. The same work is told by a DOI or arXiv id first, then by its
 * title, first author and year all found in the entry; anything less is
 * another work, since merging two works loses one of them. */
import { identifierKey, parseIdentifier, type Identifier } from './identifiers';
import { inlineSpans, type GroupItem, type Span } from './markdown';

/** A heading line that opens the references section. */
const HEADING = /^\s{0,3}(#{1,6})\s+References\s*#*\s*$/i;
const ANY_HEADING = /^\s{0,3}(#{1,6})\s+\S/;
const DEFINITION = /^\[\^([A-Za-z0-9_.:+-]{1,80})\]:\s*(.*)$/;

/** The heading a new section opens with. */
export const REFERENCES_HEADING = '## References';

export interface Section {
  /** Offset of the heading line. */
  from: number;
  /** End of the section: the next heading as deep or deeper, or the end. */
  to: number;
  /** Line indices: the heading, and the first line after the section. */
  headingLine: number;
  endLine: number;
}

/** The note's references section: its last heading named "References",
 * running to the next heading of its level or above. Null when it has none. */
export function referencesSection(text: string): Section | null {
  const lines = text.split('\n');
  let heading = -1;
  let level = 0;
  for (let i = lines.length - 1; i >= 0; i--) {
    const m = HEADING.exec(lines[i]!);
    if (m) {
      heading = i;
      level = m[1]!.length;
      break;
    }
  }
  if (heading < 0) return null;
  let end = heading + 1;
  while (end < lines.length) {
    const m = ANY_HEADING.exec(lines[end]!);
    if (m && m[1]!.length <= level) break;
    end++;
  }
  const offsetOf = (line: number) => lines.slice(0, line).reduce((n, l) => n + l.length + 1, 0);
  return {
    from: offsetOf(heading),
    to: Math.min(text.length, offsetOf(end) - (end < lines.length ? 1 : 0)),
    headingLine: heading,
    endLine: end,
  };
}

/** Where the note's own text ends: before a references section that closes
 * it (and the blank lines before that), else its end. What is added "at the
 * end" goes here, so the references stay last. */
export function bodyEnd(text: string): number {
  const section = referencesSection(text);
  if (!section || section.to < text.length) return text.length;
  let at = section.from;
  while (at > 0 && text[at - 1] === '\n') at--;
  return at;
}

export interface Definition {
  id: string;
  /** What follows `[^id]:`. */
  body: string;
  line: number;
}

/** Every reference definition in the note, in order. */
export function definitions(text: string): Definition[] {
  const out: Definition[] = [];
  text.split('\n').forEach((line, i) => {
    const m = DEFINITION.exec(line);
    if (m) out.push({ id: m[1]!, body: m[2]!, line: i });
  });
  return out;
}

/** The ids the note's markers name (`[^id]`, not a definition's own). */
export function markerIds(text: string): Set<string> {
  const out = new Set<string>();
  const re = /\[\^([A-Za-z0-9_.:+-]{1,80})\](?!:)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) out.add(m[1]!);
  return out;
}

/* ---- telling one work from another ---- */

/** A definition's text as written (escapes undone, a leading printed
 * label taken off). */
export function entryText(body: string): string {
  return body
    .replace(/\\([!-/:-@[-`{-~])/g, '$1')
    .trim()
    .replace(/^\[[^\]\n]{1,40}\]\s*/, '');
}

/** Letters and digits only, lower case, accents off, single spaces. */
export function normalizeWords(text: string): string {
  return text
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();
}

const LEAD = /^[(<["']+/;
const TRAIL = /[.,;:!?)\]>"']+$/;

/** The DOIs and arXiv ids an entry prints, as index keys. */
export function identifiersIn(text: string): string[] {
  const out = new Set<string>();
  for (const token of text.split(/\s+/)) {
    const core = token.replace(LEAD, '');
    for (const candidate of [core, core.replace(TRAIL, '')]) {
      const id = parseIdentifier(candidate);
      if (id) {
        out.add(identifierKey(id));
        break;
      }
    }
  }
  return [...out];
}

/** What a reference list says of a work: its entry text, and what the
 * core read from it. */
export interface Work {
  text: string;
  /** `doi:10.1/x`, `arxiv:1501.01234`. */
  ids?: string[];
  title?: string | null;
  /** The first author's family name. */
  author?: string | null;
  year?: number | null;
}

function idsOf(work: Work): string[] {
  const given = (work.ids ?? []).flatMap((raw) => {
    const id: Identifier | null = parseIdentifier(raw);
    return id ? [identifierKey(id)] : [];
  });
  return [...new Set([...given, ...identifiersIn(work.text)])];
}

/** Whether `work` is the work an entry (`body`) describes: a shared DOI or
 * arXiv id; else - when neither has an identifier of the same kind that
 * differs - its title, first author and year all in the entry. */
export function sameWork(work: Work, body: string): boolean {
  const text = entryText(body);
  const mine = idsOf(work);
  const theirs = identifiersIn(text);
  if (mine.some((id) => theirs.includes(id))) return true;
  const kind = (id: string) => id.slice(0, id.indexOf(':'));
  if (mine.some((a) => theirs.some((b) => kind(a) === kind(b)))) return false;
  const title = normalizeWords(work.title ?? '');
  const author = normalizeWords(work.author ?? '');
  if (title.split(' ').length < 3 || !author || !work.year) return false;
  const words = ` ${normalizeWords(text)} `;
  return (
    words.includes(` ${title} `) &&
    words.includes(` ${author} `) &&
    words.includes(` ${work.year} `)
  );
}

const STOP = new Set(['a', 'an', 'the', 'on', 'of', 'in', 'for', 'and', 'to', 'at', 'by', 'from']);

/** A readable, stable id for a work: first author, year, first title
 * word, lower-case ASCII (`simon2015almost`), suffixed a, b, ... when
 * `taken` has it. */
export function stableId(work: Work, taken: ReadonlySet<string>): string {
  const ascii = (s: string) => normalizeWords(s).replace(/ /g, '');
  const author = ascii(work.author ?? '').slice(0, 24);
  const word =
    normalizeWords(work.title ?? '')
      .split(' ')
      .find((w) => w.length > 1 && !STOP.has(w)) ?? '';
  const year = work.year ? String(work.year) : '';
  let base = `${author}${year}${word.slice(0, 20)}`;
  if (!author || base === author) base = `ref${base}`;
  if (!taken.has(base)) return base;
  for (let n = 0; n < 26; n++) {
    const id = `${base}${String.fromCharCode(97 + n)}`;
    if (!taken.has(id)) return id;
  }
  let n = 2;
  while (taken.has(`${base}-${n}`)) n++;
  return `${base}-${n}`;
}

/* ---- rewriting the section ---- */

/** Markdown for an entry: escaped as quoted text is, identifiers the text
 * does not print added. */
export function entryLine(id: string, work: Work): string {
  let text = work.text.replace(/\s+/g, ' ').trim();
  text = text.replace(/^\[[^\]]{1,40}\]\s*/, '');
  for (const raw of work.ids ?? []) {
    const id2 = parseIdentifier(raw);
    if (!id2 || identifiersIn(text).includes(identifierKey(id2))) continue;
    text += id2.kind === 'arxiv' ? ` arXiv:${id2.id}` : ` doi:${id2.id}`;
  }
  const escaped = text.replace(/[\\`*_[\]{}<>$@|~]/g, (c) => `\\${c}`);
  return `[^${id}]: ${escaped.replace(/^([#>+-]|\d+[.)])/, '\\$1')}`;
}

/** The note with every definition in its references section: definitions
 * elsewhere (each quote's, as an earlier Notepad wrote them) moved there in
 * order, an exact repeat dropped, the blank line each left closed up, and
 * the section made when there is none and something goes in it. Then
 * `extra` lines appended to it. A note already tidy comes back unchanged. */
export function tidyReferences(text: string, extra: string[] = []): string {
  const lines = text.split('\n');
  const section = referencesSection(text);
  const inSection = (i: number) =>
    section !== null && i > section.headingLine && i < section.endLine;
  const moved: string[] = [];
  const drop = new Set<number>();
  for (let i = 0; i < lines.length; i++) {
    if (inSection(i) || !DEFINITION.test(lines[i]!)) continue;
    let end = i;
    while (end + 1 < lines.length && !inSection(end + 1) && DEFINITION.test(lines[end + 1]!)) end++;
    for (let k = i; k <= end; k++) {
      moved.push(lines[k]!);
      drop.add(k);
    }
    // The blank line that set the run apart goes with it.
    const before = i === 0 || lines[i - 1]!.trim() === '';
    const after = end + 1 < lines.length && lines[end + 1]!.trim() === '';
    if (before && after) drop.add(end + 1);
    i = end;
  }
  if (moved.length === 0 && extra.length === 0) return text;
  const existing = new Set(
    section
      ? lines.slice(section.headingLine + 1, section.endLine).filter((l) => DEFINITION.test(l))
      : []
  );
  const adding: string[] = [];
  for (const line of [...moved, ...extra]) {
    if (existing.has(line)) continue;
    existing.add(line);
    adding.push(line);
  }
  if (!section) {
    const body = lines
      .filter((_, i) => !drop.has(i))
      .join('\n')
      .replace(/\s+$/, '');
    if (adding.length === 0) return body === text ? text : `${body}\n`;
    return `${body}${body ? '\n\n' : ''}${REFERENCES_HEADING}\n\n${adding.join('\n')}\n`;
  }
  // After the section's last definition, else after its heading and a
  // blank line.
  let last = -1;
  for (let i = section.headingLine + 1; i < section.endLine; i++) {
    if (DEFINITION.test(lines[i]!)) last = i;
  }
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (!drop.has(i)) out.push(lines[i]!);
    if (adding.length === 0) continue;
    if (last >= 0 && i === last) out.push(...adding);
    if (last < 0 && i === section.headingLine) {
      const next = lines[i + 1];
      if (next !== undefined && next.trim() === '') {
        out.push(next, ...adding);
        i++;
      } else {
        out.push('', ...adding);
      }
    }
  }
  return out.join('\n');
}

/** Add works to the note's references: each one an entry already names
 * reuses that entry's id; the rest get a new entry, under a new stable id.
 * Definitions elsewhere move into the section first. The note as it is
 * then, and the id for each work in order. */
export function addReferences(text: string, works: Work[]): { text: string; ids: string[] } {
  const tidy = tidyReferences(text);
  const defs = definitions(tidy);
  const taken = new Set<string>([...defs.map((d) => d.id), ...markerIds(tidy)]);
  const added: { id: string; work: Work }[] = [];
  const lines: string[] = [];
  const ids = works.map((work) => {
    const known = defs.find((d) => sameWork(work, d.body));
    if (known) return known.id;
    const again = added.find((a) => sameWork(work, a.work.text) || a.work.text === work.text);
    if (again) return again.id;
    const id = stableId(work, taken);
    taken.add(id);
    added.push({ id, work });
    lines.push(entryLine(id, work));
    return id;
  });
  return { text: lines.length > 0 ? tidyReferences(tidy, lines) : tidy, ids };
}

/** Definitions no marker in the note names any more. */
export function unusedReferences(text: string): string[] {
  const used = markerIds(text);
  return definitions(text)
    .filter((d) => !used.has(d.id))
    .map((d) => d.id);
}

/** The note without the definitions of `ids`. */
export function removeReferences(text: string, ids: readonly string[]): string {
  const gone = new Set(ids);
  return text
    .split('\n')
    .filter((line) => {
      const m = DEFINITION.exec(line);
      return !m || !gone.has(m[1]!);
    })
    .join('\n');
}

/** Whether definitions sit outside the references section (an earlier
 * Notepad's, under each quote). */
export function hasStrayDefinitions(text: string): boolean {
  return tidyReferences(text) !== text;
}

/** An item the editor knows: what an entry is matched against. */
export interface LibraryWork {
  key: string;
  title: string;
  /** "Attias, Smith - 2022". */
  detail: string;
  doi?: string | null;
  arxiv?: string | null;
}

/** Entries whose work is now an item: by its DOI or arXiv id, else by its
 * title (four words or more), first author and year all in the entry. The
 * key for each entry id. */
export function libraryMatches(text: string, items: readonly LibraryWork[]): Map<string, string> {
  const out = new Map<string, string>();
  for (const def of definitions(text)) {
    const body = entryText(def.body);
    const ids = identifiersIn(body);
    const found = items.find((item) => {
      const own = [
        item.doi ? `doi:${item.doi.toLowerCase()}` : null,
        item.arxiv ? `arxiv:${item.arxiv.toLowerCase()}` : null,
      ].filter((x): x is string => x !== null);
      if (own.some((id) => ids.includes(id))) return true;
      const title = normalizeWords(item.title);
      if (title.split(' ').length < 4) return false;
      const [authors, year] = item.detail.split(' - ');
      const author = normalizeWords((authors ?? '').split(/,|&| and | et al/)[0] ?? '')
        .split(' ')
        .pop();
      if (!author || !year || !/^\d{4}$/.test(year.trim())) return false;
      return sameWork({ text: '', title: item.title, author, year: Number(year) }, def.body);
    });
    if (found) out.set(def.id, found.key);
  }
  return out;
}

/** A marker as Notepad writes one: `[^id]{label=Sim15 at=1}`. */
export function markerText(id: string, label: string | null, at: number | null): string {
  const attrs = [
    label ? `label=${/[\s}"]/.test(label) ? `"${label.replace(/"/g, '')}"` : label}` : null,
    at !== null ? `at=${at}` : null,
  ].filter(Boolean);
  return `[^${id}]${attrs.length > 0 ? `{${attrs.join(' ')}}` : ''}`;
}

/** A group written back: its items' citations as one Pandoc group, then
 * its markers, each with its place among them. */
function groupText(items: GroupItem[]): string {
  const cites = items.filter((it) => it.kind === 'cite');
  const head =
    cites.length > 0
      ? `[${cites
          .map((it) => `@${it.cite.key}${it.cite.locator ? `, ${it.cite.locator}` : ''}`)
          .join('; ')}]`
      : '';
  const marks = items.flatMap((it, at) =>
    it.kind === 'noteref'
      ? [markerText(it.id, it.label, cites.length > 0 && at < items.length - 1 ? at : null)]
      : []
  );
  return head + marks.join('');
}

/** The note with the markers of `keys`' entries written as citations of
 * the items (`[^id]{...}` -> `[@key]`, taken into the group it sits in, in
 * its printed place), and those entries gone. */
export function linkToLibrary(text: string, keys: ReadonlyMap<string, string>): string {
  if (keys.size === 0) return text;
  const lines = text.split('\n').map((line) => {
    if (DEFINITION.test(line) || !line.includes('[^')) return line;
    let out = '';
    let rest = line;
    const walk = (spans: Span[]) => {
      for (const span of spans) {
        if (span.kind === 'em' || span.kind === 'strong' || span.kind === 'link') {
          walk(span.children);
          continue;
        }
        let replacement: string | null = null;
        if (span.kind === 'noteref' && keys.has(span.id)) replacement = `[@${keys.get(span.id)!}]`;
        if (
          span.kind === 'group' &&
          span.items.some((it) => it.kind === 'noteref' && keys.has(it.id)) &&
          span.items.every((it) => it.kind !== 'cite' || !it.cite.box)
        ) {
          replacement = groupText(
            span.items.map((it): GroupItem =>
              it.kind === 'noteref' && keys.has(it.id)
                ? {
                    kind: 'cite',
                    raw: '',
                    cite: { key: keys.get(it.id)!, locator: null, page: null, box: null },
                  }
                : it
            )
          );
        }
        if (replacement === null || !('raw' in span)) continue;
        const at = rest.indexOf(span.raw);
        if (at < 0) continue;
        out += rest.slice(0, at) + replacement;
        rest = rest.slice(at + span.raw.length);
      }
    };
    walk(inlineSpans(line));
    return out + rest;
  });
  return removeReferences(lines.join('\n'), [...keys.keys()]);
}
