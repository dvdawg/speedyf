// The frontend half of the field-script evaluation (see
// src-tauri/src/engine/forms/behaviour_eval.rs): for each <form>.json the
// engine dumped, pick a valid value for every text field with a recognised
// keystroke or format script, run the calculations the way the app does,
// format every value the way the app saves it, and write <form>.plan.json.
//
//   node scripts/forms-behaviour-eval.mjs <out dir>
//
// Runs the app's own formScripts.ts (Node strips its types).
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  acceptKeystroke,
  appearanceEntries,
  commitValue,
  recalculate,
} from '../src/features/forms/formScripts.ts';

const out = process.argv[2];
if (!out) {
  console.error('usage: node scripts/forms-behaviour-eval.mjs <out dir>');
  process.exit(1);
}

/** Values computed by hand for the invoice fixture (behaviour_tests.rs). */
const HAND = {
  invoice: {
    subtotal: '1350.7',
    tax: '108.056',
    total: '1458.756',
    avg: '450.233333333333',
    maxv: '1000',
  },
};
const INVOICE_TYPED = {
  l1: '100',
  l2: '250.7',
  l3: '1000',
  rate: '0.08',
  net: '-42.5',
  when: '3/5/2026',
  ssn: '123456789',
  phone: '5551234567',
};

/** A value a field with this keystroke or format takes. */
function sample(spec, n) {
  switch (spec.kind) {
    case 'number':
      return n % 3 === 2 ? `-${1234.5 + n}` : `${1234.5 + n}`;
    case 'percent':
      return '0.125';
    case 'date':
      return '3/5/2026';
    case 'time':
      return '13:05';
    case 'special':
      return ['94720', '947201234', '5551234567', '123456789'][spec.style];
    case 'mask': {
      let digit = 0;
      return [...spec.mask]
        .map((c) => {
          if (c === '9') return String((digit += 1) % 10);
          if (c === 'A') return 'a';
          if (c === 'O') return '1';
          if (c === 'X') return 'x';
          return c;
        })
        .join('');
    }
    default:
      return null;
  }
}

for (const file of readdirSync(out).filter((f) => f.endsWith('.json') && !f.includes('.plan.'))) {
  const name = file.replace(/\.json$/, '');
  const form = JSON.parse(readFileSync(join(out, file), 'utf8'));
  const behaviours = form.behaviours ?? {};
  const texts = new Map();
  let n = 0;
  for (const f of [...form.fields].sort((a, b) => a.tabOrder - b.tabOrder)) {
    if (f.kind !== 'text' || f.readOnly || texts.has(f.name)) continue;
    const b = behaviours[f.name];
    const spec = b?.keystroke ?? b?.format;
    if (!spec) continue;
    let value = name === 'invoice' ? INVOICE_TYPED[f.name] : sample(spec, (n += 1));
    if (value === undefined || value === null) continue;
    if (f.maxLen > 0 && value.length > f.maxLen) continue;
    if (b.keystroke) {
      const typed = acceptKeystroke(b.keystroke, value);
      if (!typed.ok) throw new Error(`${name}/${f.name}: ${value} refused`);
      const committed = commitValue(b.keystroke, typed.value);
      if (committed.error) throw new Error(`${name}/${f.name}: ${committed.error}`);
      value = committed.value;
    }
    texts.set(f.name, value);
  }
  if (name === 'invoice') texts.set('note', 'Paid');

  const names = [...new Set(form.fields.map((f) => f.name))];
  const byName = (field) => form.fields.find((f) => f.name === field);
  const results = recalculate(
    [...texts.keys()],
    behaviours,
    form.calcOrder ?? [],
    names,
    (field) => texts.get(field) ?? byName(field)?.value ?? ''
  );
  const calcs = Object.entries(results).map(([field, value]) => [
    field,
    value,
    HAND[name]?.[field] ?? null,
  ]);
  for (const [field, value] of Object.entries(results)) texts.set(field, value);

  const entries = [...texts.entries()];
  const plan = {
    texts: entries,
    display: appearanceEntries(behaviours, form.fields, entries),
    calcs,
  };
  writeFileSync(join(out, `${name}.plan.json`), JSON.stringify(plan, null, 2));
  console.log(
    `${name}: ${entries.length} values, ${calcs.length} calculated, ${plan.display.length} formatted`
  );
}
