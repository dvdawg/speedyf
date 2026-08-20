/** Turning CUPS vocabulary into words.
 *
 * A printer reports `DuplexNoTumble` and `cupsPrintQuality`; neither belongs in
 * front of a person. Anything not named here falls through unchanged, so a
 * printer with options we have never seen still works — it just shows its own
 * spelling rather than ours. */

const OPTION_LABELS: Record<string, string> = {
  Duplex: 'Two-sided',
  PageSize: 'Paper',
  cupsPrintQuality: 'Quality',
  ColorModel: 'Color',
  MediaType: 'Paper type',
  OutputBin: 'Output tray',
  InputSlot: 'Paper source',
};

const CHOICE_LABELS: Record<string, Record<string, string>> = {
  Duplex: {
    None: 'Off',
    DuplexNoTumble: 'On, long edge',
    DuplexTumble: 'On, short edge',
  },
  ColorModel: {
    RGB: 'Color',
    DeviceRGB: 'Color',
    AdobeRGB: 'Color (Adobe RGB)',
    DeviceRGB16: 'Color (16-bit)',
    Gray: 'Black and white',
    DeviceGray: 'Black and white',
    Gray16: 'Black and white (16-bit)',
    DeviceGray16: 'Black and white (16-bit)',
    CMYK: 'Color (CMYK)',
  },
  cupsPrintQuality: { Draft: 'Draft', Normal: 'Normal', High: 'Best' },
  MediaType: { any: 'Automatic', stationery: 'Plain paper' },
};

/** The name to show for an option, preferring ours, then the printer's own
 * human label, then the raw key. */
export function optionLabel(key: string, reported: string): string {
  return OPTION_LABELS[key] ?? (reported && reported !== key ? reported : key);
}

/** The name to show for one of an option's choices. */
export function choiceLabel(key: string, choice: string): string {
  const known = CHOICE_LABELS[key]?.[choice];
  if (known) return known;
  // Paper sizes and anything else unrecognized read fine as themselves;
  // just break up the ones CUPS runs together.
  return choice.replace(/([a-z])([A-Z])/g, '$1 $2');
}

/** Options worth a control, in the order people look for them. Printers report
 * dozens, most of which are driver plumbing. */
export const SHOWN_OPTIONS = ['Duplex', 'PageSize', 'ColorModel', 'cupsPrintQuality'];
