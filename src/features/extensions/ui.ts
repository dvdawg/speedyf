/** The vocabulary extensions describe UI in.
 *
 * Extensions do not render. They send a description of what they want shown and
 * SpeedyF draws it with its own components. That is what keeps a third-party
 * panel from breaking layout, clashing with the stylesheet, or holding a
 * reference into the reactive graph — and it is why extensions inherited dark
 * mode and the inverted-page palette without their authors doing anything.
 *
 * The cost is that the vocabulary is finite, and it will feel too small the
 * first time a real extension wants something it cannot say. Widening it once
 * per request is how this ends up as DOM injection with extra steps; the bar
 * for a new node type is that several unrelated extensions need it.
 *
 * Everything here arrives from a worker, so it is untrusted input. `parseUi`
 * validates structurally and drops what it cannot read rather than trusting a
 * cast — a panel description is the one place an extension gets to put text in
 * front of the user, and the renderer must never receive a half-typed object.
 */

/** Hard caps. A panel is a browsing aid, not a data dump, and an extension
 * looping a render call must not be able to grow the DOM without bound. */
export const MAX_ROWS = 500;
export const MAX_SECTIONS = 50;
export const MAX_TEXT = 2_000;

export interface UiAction {
  /** The extension's own command id, invoked when the row is activated. */
  command: string;
  /** Passed back verbatim, so one command can serve many rows. */
  value?: string;
}

export interface UiRow {
  /** Stable across renders; used as the list key. */
  id: string;
  title: string;
  subtitle?: string;
  /** Short status word shown at the end of the row. */
  badge?: string;
  /** Whether this row belongs to the one above it — a detail of that row
   * rather than an entry in its own right. Drawn indented and hung off a rule.
   *
   * Nesting a *structure* was the alternative and was not worth it: rows would
   * have to carry children, every panel would pay for a tree it does not use,
   * and one flag says the only thing any of them needs to say. */
  nested?: boolean;
  action?: UiAction;
}

export interface UiSection {
  title?: string;
  rows: UiRow[];
}

export interface UiDescription {
  sections: UiSection[];
  /** Shown instead of the sections when there is nothing to list. */
  empty?: string;
  /** Shown above everything, in the house error style. */
  error?: string;
}

function text(value: unknown, limit = MAX_TEXT): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (trimmed.length === 0) return null;
  // Truncated rather than refused: an over-long title is a formatting mistake,
  // not a reason to throw away the whole panel.
  return trimmed.length > limit ? `${trimmed.slice(0, limit)}…` : trimmed;
}

function parseAction(value: unknown): UiAction | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const record = value as Record<string, unknown>;
  const command = text(record.command, 200);
  if (!command || command.includes('/')) return undefined;
  const arg = text(record.value, 500);
  return arg === null ? { command } : { command, value: arg };
}

function parseRow(value: unknown, index: number): UiRow | null {
  if (typeof value !== 'object' || value === null) return null;
  const record = value as Record<string, unknown>;
  const title = text(record.title);
  if (!title) return null; // a row with nothing to say is not a row
  const id = text(record.id, 200) ?? `row-${index}`;
  const subtitle = text(record.subtitle);
  const badge = text(record.badge, 40);
  const action = parseAction(record.action);
  return {
    id,
    title,
    ...(subtitle ? { subtitle } : {}),
    ...(badge ? { badge } : {}),
    ...(record.nested === true ? { nested: true } : {}),
    ...(action ? { action } : {}),
  };
}

function parseRows(value: unknown, budget: { left: number }): UiRow[] {
  if (!Array.isArray(value)) return [];
  const rows: UiRow[] = [];
  for (let i = 0; i < value.length && budget.left > 0; i++) {
    const row = parseRow(value[i], i);
    if (!row) continue;
    rows.push(row);
    budget.left--;
  }
  return rows;
}

/** Read a description an extension sent.
 *
 * Total: whatever comes back is safe to render. Unreadable pieces are dropped
 * individually so one bad row does not cost the whole panel, and the caps are
 * applied across the description rather than per section — an extension cannot
 * route around `MAX_ROWS` by splitting into more sections.
 */
export function parseUi(value: unknown): UiDescription {
  const empty: UiDescription = { sections: [] };
  if (typeof value !== 'object' || value === null) return empty;
  const record = value as Record<string, unknown>;

  const budget = { left: MAX_ROWS };
  const sections: UiSection[] = [];

  // `rows` at the top level is shorthand for one untitled section: the common
  // case is a flat list, and making every extension wrap it would be noise.
  if (Array.isArray(record.rows)) {
    const rows = parseRows(record.rows, budget);
    if (rows.length > 0) sections.push({ rows });
  }

  if (Array.isArray(record.sections)) {
    for (const raw of record.sections.slice(0, MAX_SECTIONS)) {
      if (typeof raw !== 'object' || raw === null) continue;
      const section = raw as Record<string, unknown>;
      const rows = parseRows(section.rows, budget);
      const title = text(section.title, 200);
      if (rows.length === 0 && !title) continue;
      sections.push({ ...(title ? { title } : {}), rows });
    }
  }

  const emptyText = text(record.empty);
  const errorText = text(record.error);
  return {
    sections,
    ...(emptyText ? { empty: emptyText } : {}),
    ...(errorText ? { error: errorText } : {}),
  };
}

/** Whether a description would render as nothing at all, so the panel can show
 * its own placeholder rather than an empty box. */
export function isBlank(ui: UiDescription): boolean {
  return (
    ui.sections.every((section) => section.rows.length === 0) &&
    ui.empty === undefined &&
    ui.error === undefined
  );
}
