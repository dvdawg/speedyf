// Math to LaTeX — select math in a PDF, or click "Copy LaTeX" on a display
// equation, and get LaTeX for it.
//
// There is no single good way to read math back out of a PDF, so this tries
// the exact ways first and falls back, each one used only when it applies:
//
//   1. Tagged PDF. A tagged document can carry its formulas as Formula
//      elements with the LaTeX in the alt text, or with MathML. That is the
//      author's own source, so it wins whenever it covers the selection.
//   2. arXiv source. A paper with an arXiv id has its TeX on arxiv.org. The
//      e-print is fetched once, unpacked here (gzip + tar), and its math
//      environments matched against what was selected.
//   3. Open-access journal version. For a DOI, Europe PMC may hold the JATS
//      full text, whose formulas are MathML (and sometimes TeX).
//   4. The glyphs themselves. TeX fonts say exactly which symbol each glyph
//      is (the raw code in CMMI/CMSY/CMEX/MSAM/MSBM is a table lookup), and the
//      positions say how they are arranged: raised and smaller is a
//      superscript, a rule with things above and below is a fraction, and so
//      on. This is a reconstruction, not a recovery, and its confidence says
//      so.
//
// A scanned page has no fonts to decode and its OCR'd letters are guesses;
// reconstructing math from them would be inventing it, so it is refused.
//
// Nothing here writes the clipboard: `ui.showMath` shows the result with copy
// buttons, and copying happens on the user's click.

/** Storage values are capped at 64k characters; a parsed source above this is
 * kept in memory for the session only. */
const STORAGE_LIMIT = 60_000;
const CACHE_PREFIX = 'src:v1:';
/** Everything network-bound gives up after this long, from the moment the
 * command was invoked, so the command still answers well inside the host's
 * five-second invoke timeout. A fetch that is cut off keeps running and fills
 * the in-memory cache, so asking again a moment later finds it. */
const NETWORK_BUDGET_MS = 3_800;
const ARXIV_MAX_BYTES = 20_000_000;
const SCAN_MESSAGE = "This looks like a scan; math can't be read from it.";

let nextRequestId = 1;
const pending = new Map();
/** arXiv id+version, or DOI → Promise of parsed snippets (or null). */
const sources = new Map();

self.onmessage = (event) => {
  const message = event.data;

  if (message.type === 'invoke') {
    handle(message.command, message.value)
      .then(() => done(message.requestId, true))
      .catch((err) => done(message.requestId, false, String(err?.message ?? err)));
    return;
  }

  if (message.type === 'reply') {
    const waiting = pending.get(message.requestId);
    if (!waiting) return;
    pending.delete(message.requestId);
    if (message.ok) waiting.resolve(message.value);
    else waiting.reject(new Error(message.error));
  }
};

function call(api, args) {
  const requestId = nextRequestId++;
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    self.postMessage({ type: 'call', requestId, api, args });
  });
}

function done(requestId, ok, error) {
  self.postMessage({ type: 'done', requestId, ok, ...(error ? { error } : {}) });
}

function log(level, message) {
  self.postMessage({ type: 'log', level, message });
}

// ---------- font encodings ----------
//
// The raw code of a glyph in a TeX font is its slot in the font's encoding,
// and the encodings of the Computer Modern / AMS math fonts are fixed: OML
// (math italic, cmmi), OMS (math symbols, cmsy), OMX (math extension, cmex),
// OT1 (text, cmr and friends), and the AMS msam/msbm. A slot that is a piece
// of something larger (a hook of \hookrightarrow, the top of a tall brace) is
// described as such rather than as a symbol of its own.

const UPPER = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('');
const LOWER = 'abcdefghijklmnopqrstuvwxyz'.split('');
const DIGITS = '0123456789'.split('');

/** OML: cmmi, cmmib, LM math italic, txmi/pxmi. Capital Greek here is the
 * italic one; upright capital Greek comes from the text font. */
const OML = [
  '\\varGamma',
  '\\varDelta',
  '\\varTheta',
  '\\varLambda',
  '\\varXi',
  '\\varPi',
  '\\varSigma',
  '\\varUpsilon',
  '\\varPhi',
  '\\varPsi',
  '\\varOmega',
  '\\alpha',
  '\\beta',
  '\\gamma',
  '\\delta',
  '\\epsilon',
  '\\zeta',
  '\\eta',
  '\\theta',
  '\\iota',
  '\\kappa',
  '\\lambda',
  '\\mu',
  '\\nu',
  '\\xi',
  '\\pi',
  '\\rho',
  '\\sigma',
  '\\tau',
  '\\upsilon',
  '\\phi',
  '\\chi',
  '\\psi',
  '\\omega',
  '\\varepsilon',
  '\\vartheta',
  '\\varpi',
  '\\varrho',
  '\\varsigma',
  '\\varphi',
  '\\leftharpoonup',
  '\\leftharpoondown',
  '\\rightharpoonup',
  '\\rightharpoondown',
  'piece:lhook',
  'piece:rhook',
  '\\triangleright',
  '\\triangleleft',
  ...DIGITS,
  '.',
  ',',
  '<',
  '/',
  '>',
  '\\star',
  '\\partial',
  ...UPPER,
  '\\flat',
  '\\natural',
  '\\sharp',
  '\\smile',
  '\\frown',
  '\\ell',
  ...LOWER,
  '\\imath',
  '\\jmath',
  '\\wp',
  'accent:\\vec',
  null,
];

/** OMS: cmsy, cmbsy, LM math symbols, txsy/pxsy. */
const OMS = [
  '-',
  '\\cdot',
  '\\times',
  '\\ast',
  '\\div',
  '\\diamond',
  '\\pm',
  '\\mp',
  '\\oplus',
  '\\ominus',
  '\\otimes',
  '\\oslash',
  '\\odot',
  '\\bigcirc',
  '\\circ',
  '\\bullet',
  '\\asymp',
  '\\equiv',
  '\\subseteq',
  '\\supseteq',
  '\\leq',
  '\\geq',
  '\\preceq',
  '\\succeq',
  '\\sim',
  '\\approx',
  '\\subset',
  '\\supset',
  '\\ll',
  '\\gg',
  '\\prec',
  '\\succ',
  '\\leftarrow',
  '\\to',
  '\\uparrow',
  '\\downarrow',
  '\\leftrightarrow',
  '\\nearrow',
  '\\searrow',
  '\\simeq',
  '\\Leftarrow',
  '\\Rightarrow',
  '\\Uparrow',
  '\\Downarrow',
  '\\Leftrightarrow',
  '\\nwarrow',
  '\\swarrow',
  '\\propto',
  '\\prime',
  '\\infty',
  '\\in',
  '\\ni',
  '\\bigtriangleup',
  '\\bigtriangledown',
  'piece:not',
  'piece:mapsto',
  '\\forall',
  '\\exists',
  '\\neg',
  '\\emptyset',
  '\\Re',
  '\\Im',
  '\\top',
  '\\bot',
  '\\aleph',
  ...UPPER.map((c) => `\\mathcal{${c}}`),
  '\\cup',
  '\\cap',
  '\\uplus',
  '\\wedge',
  '\\vee',
  '\\vdash',
  '\\dashv',
  '\\lfloor',
  '\\rfloor',
  '\\lceil',
  '\\rceil',
  '\\{',
  '\\}',
  '\\langle',
  '\\rangle',
  '|',
  '\\|',
  '\\updownarrow',
  '\\Updownarrow',
  '\\backslash',
  '\\wr',
  'radical',
  '\\amalg',
  '\\nabla',
  '\\int',
  '\\sqcup',
  '\\sqcap',
  '\\sqsubseteq',
  '\\sqsupseteq',
  '\\S',
  '\\dagger',
  '\\ddagger',
  '\\P',
  '\\clubsuit',
  '\\diamondsuit',
  '\\heartsuit',
  '\\spadesuit',
];

/** OMX: cmex, LM math extension, txex/pxex. Each slot is `[tex, kind, size]`:
 * size is the TeX size step of a delimiter (1 \big … 4 \Bigg), and pieces of
 * extensible delimiters carry which side they belong to. */
const OMX = (() => {
  const t = [];
  const delims = (list, size) => list.forEach((d) => t.push([d, 'delim', size]));
  delims(['(', ')', '[', ']', '\\lfloor', '\\rfloor', '\\lceil', '\\rceil', '\\{', '\\}'], 1);
  delims(['\\langle', '\\rangle', '|', '\\|', '/', '\\backslash'], 1);
  delims(['(', ')'], 2);
  delims(['(', ')', '[', ']', '\\lfloor', '\\rfloor', '\\lceil', '\\rceil', '\\{', '\\}'], 3);
  delims(['\\langle', '\\rangle', '/', '\\backslash'], 3);
  delims(['(', ')', '[', ']', '\\lfloor', '\\rfloor', '\\lceil', '\\rceil', '\\{', '\\}'], 4);
  delims(['\\langle', '\\rangle', '/', '\\backslash', '/', '\\backslash'], 4);
  // 0x30–0x3F: tops, bottoms, middles and extenders of tall delimiters.
  t.push(['(', 'piece', 'L'], [')', 'piece', 'R'], ['[', 'piece', 'L'], [']', 'piece', 'R']);
  t.push(['[', 'piece', 'L'], [']', 'piece', 'R'], ['[', 'piece', 'L'], [']', 'piece', 'R']);
  t.push(
    ['\\{', 'piece', 'L'],
    ['\\}', 'piece', 'R'],
    ['\\{', 'piece', 'L'],
    ['\\}', 'piece', 'R']
  );
  t.push(['\\{', 'piece', 'L'], ['\\}', 'piece', 'R'], ['brace', 'piece', '?']);
  t.push(['\\updownarrow', 'piece', '?']);
  // 0x40–0x45
  t.push(['(', 'piece', 'L'], [')', 'piece', 'R'], ['(', 'piece', 'L'], [')', 'piece', 'R']);
  delims(['\\langle', '\\rangle'], 4);
  // 0x46–0x61: big operators, text size then display size.
  const ops = (list) => list.forEach((o) => t.push([o, 'bigop', 0]));
  ops(['\\bigsqcup', '\\bigsqcup', '\\oint', '\\oint', '\\bigodot', '\\bigodot']);
  ops(['\\bigoplus', '\\bigoplus', '\\bigotimes', '\\bigotimes']);
  ops(['\\sum', '\\prod', '\\int', '\\bigcup', '\\bigcap', '\\biguplus', '\\bigwedge', '\\bigvee']);
  ops(['\\sum', '\\prod', '\\int', '\\bigcup', '\\bigcap', '\\biguplus', '\\bigwedge', '\\bigvee']);
  ops(['\\coprod', '\\coprod']);
  // 0x62–0x67: wide accents.
  for (let i = 0; i < 3; i++) t.push(['\\widehat', 'accent', 0]);
  for (let i = 0; i < 3; i++) t.push(['\\widetilde', 'accent', 0]);
  // 0x68–0x6F
  delims(['[', ']', '\\lfloor', '\\rfloor', '\\lceil', '\\rceil', '\\{', '\\}'], 2);
  // 0x70–0x7F: radicals, radical pieces, arrow pieces, brace tips.
  for (let i = 0; i < 4; i++) t.push(['\\sqrt', 'radical', i + 1]);
  t.push(['\\sqrt', 'radical', 5], ['\\sqrt', 'piece', 'V'], ['\\sqrt', 'piece', 'V']);
  t.push(['\\|', 'piece', '?'], ['\\uparrow', 'piece', '?'], ['\\downarrow', 'piece', '?']);
  for (let i = 0; i < 4; i++) t.push(['brace-tip', 'piece', 'H']);
  t.push(['\\Uparrow', 'piece', '?'], ['\\Downarrow', 'piece', '?']);
  return t;
})();

/** OT1: cmr, cmbx, cmti, cmsl, cmss, cmtt. In math this is where upright
 * capital Greek, digits, upright punctuation, function names and the
 * ordinary accents (\hat, \tilde, \bar, …) come from. */
const OT1 = [
  '\\Gamma',
  '\\Delta',
  '\\Theta',
  '\\Lambda',
  '\\Xi',
  '\\Pi',
  '\\Sigma',
  '\\Upsilon',
  '\\Phi',
  '\\Psi',
  '\\Omega',
  'ff',
  'fi',
  'fl',
  'ffi',
  'ffl',
  '\\imath',
  '\\jmath',
  'accent:\\grave',
  'accent:\\acute',
  'accent:\\check',
  'accent:\\breve',
  'accent:\\bar',
  'accent:\\mathring',
  null,
  'ß',
  'æ',
  'œ',
  'ø',
  'Æ',
  'Œ',
  'Ø',
  null,
  '!',
  '"',
  '\\#',
  '\\$',
  '\\%',
  '\\&',
  "'",
  '(',
  ')',
  '*',
  '+',
  ',',
  '-',
  '.',
  '/',
  ...DIGITS,
  ':',
  ';',
  '¡',
  '=',
  '¿',
  '?',
  '@',
  ...UPPER,
  '[',
  '“',
  ']',
  'accent:\\hat',
  'accent:\\dot',
  '‘',
  ...LOWER,
  '–',
  '—',
  null,
  'accent:\\tilde',
  'accent:\\ddot',
];

/** AMS symbols A (msam). */
const MSAM = [
  '\\boxdot',
  '\\boxplus',
  '\\boxtimes',
  '\\square',
  '\\blacksquare',
  '\\centerdot',
  '\\lozenge',
  '\\blacklozenge',
  '\\circlearrowright',
  '\\circlearrowleft',
  '\\rightleftharpoons',
  '\\leftrightharpoons',
  '\\boxminus',
  '\\Vdash',
  '\\Vvdash',
  '\\vDash',
  '\\twoheadrightarrow',
  '\\twoheadleftarrow',
  '\\leftleftarrows',
  '\\rightrightarrows',
  '\\upuparrows',
  '\\downdownarrows',
  '\\upharpoonright',
  '\\downharpoonright',
  '\\upharpoonleft',
  '\\downharpoonleft',
  '\\rightarrowtail',
  '\\leftarrowtail',
  '\\leftrightarrows',
  '\\rightleftarrows',
  '\\Lsh',
  '\\Rsh',
  '\\rightsquigarrow',
  '\\leftrightsquigarrow',
  '\\looparrowleft',
  '\\looparrowright',
  '\\circeq',
  '\\succsim',
  '\\gtrsim',
  '\\gtrapprox',
  '\\multimap',
  '\\therefore',
  '\\because',
  '\\doteqdot',
  '\\triangleq',
  '\\precsim',
  '\\lesssim',
  '\\lessapprox',
  '\\eqslantless',
  '\\eqslantgtr',
  '\\curlyeqprec',
  '\\curlyeqsucc',
  '\\preccurlyeq',
  '\\leqq',
  '\\leqslant',
  '\\lessgtr',
  '\\backprime',
  null,
  '\\risingdotseq',
  '\\fallingdotseq',
  '\\succcurlyeq',
  '\\geqq',
  '\\geqslant',
  '\\gtrless',
  '\\sqsubset',
  '\\sqsupset',
  '\\vartriangleright',
  '\\vartriangleleft',
  '\\trianglerighteq',
  '\\trianglelefteq',
  '\\bigstar',
  '\\between',
  '\\blacktriangledown',
  '\\blacktriangleright',
  '\\blacktriangleleft',
  '\\dashrightarrow',
  '\\dashleftarrow',
  '\\vartriangle',
  '\\blacktriangle',
  '\\triangledown',
  '\\eqcirc',
  '\\lesseqgtr',
  '\\gtreqless',
  '\\lesseqqgtr',
  '\\gtreqqless',
  '\\yen',
  '\\Rrightarrow',
  '\\Lleftarrow',
  '\\checkmark',
  '\\veebar',
  '\\barwedge',
  '\\doublebarwedge',
  '\\angle',
  '\\measuredangle',
  '\\sphericalangle',
  '\\varpropto',
  '\\smallsmile',
  '\\smallfrown',
  '\\Subset',
  '\\Supset',
  '\\Cup',
  '\\Cap',
  '\\curlywedge',
  '\\curlyvee',
  '\\leftthreetimes',
  '\\rightthreetimes',
  '\\subseteqq',
  '\\supseteqq',
  '\\bumpeq',
  '\\Bumpeq',
  '\\lll',
  '\\ggg',
  '\\ulcorner',
  '\\urcorner',
  '\\circledR',
  '\\circledS',
  '\\pitchfork',
  '\\dotplus',
  '\\backsim',
  '\\backsimeq',
  '\\llcorner',
  '\\lrcorner',
  '\\maltese',
  '\\complement',
  '\\intercal',
  '\\circledcirc',
  '\\circledast',
  '\\circleddash',
];

/** AMS symbols B (msbm): negated relations and the blackboard capitals. */
const MSBM = [
  '\\lvertneqq',
  '\\gvertneqq',
  '\\nleq',
  '\\ngeq',
  '\\nless',
  '\\ngtr',
  '\\nprec',
  '\\nsucc',
  '\\lneqq',
  '\\gneqq',
  '\\nleqslant',
  '\\ngeqslant',
  '\\lneq',
  '\\gneq',
  '\\npreceq',
  '\\nsucceq',
  '\\precnsim',
  '\\succnsim',
  '\\lnsim',
  '\\gnsim',
  '\\nleqq',
  '\\ngeqq',
  '\\precneqq',
  '\\succneqq',
  '\\precnapprox',
  '\\succnapprox',
  '\\lnapprox',
  '\\gnapprox',
  '\\nsim',
  '\\ncong',
  '\\diagup',
  '\\diagdown',
  '\\varsubsetneq',
  '\\varsupsetneq',
  '\\nsubseteqq',
  '\\nsupseteqq',
  '\\subsetneqq',
  '\\supsetneqq',
  '\\varsubsetneqq',
  '\\varsupsetneqq',
  '\\subsetneq',
  '\\supsetneq',
  '\\nsubseteq',
  '\\nsupseteq',
  '\\nparallel',
  '\\nmid',
  '\\nshortmid',
  '\\nshortparallel',
  '\\nvdash',
  '\\nVdash',
  '\\nvDash',
  '\\nVDash',
  '\\ntrianglerighteq',
  '\\ntrianglelefteq',
  '\\ntriangleleft',
  '\\ntriangleright',
  '\\nleftarrow',
  '\\nrightarrow',
  '\\nLeftarrow',
  '\\nRightarrow',
  '\\nLeftrightarrow',
  '\\nleftrightarrow',
  '\\divideontimes',
  '\\varnothing',
  '\\nexists',
  ...UPPER.map((c) => `\\mathbb{${c}}`),
  'accent:\\widehat',
  'accent:\\widehat',
  'accent:\\widetilde',
  'accent:\\widetilde',
  null,
  '\\Finv',
  '\\Game',
  null,
  null,
  null,
  null,
  null,
  '\\eth',
  '\\eqsim',
  '\\beth',
  '\\gimel',
  '\\daleth',
  '\\lessdot',
  '\\gtrdot',
  '\\ltimes',
  '\\rtimes',
  '\\shortmid',
  '\\shortparallel',
  '\\smallsetminus',
  '\\thicksim',
  '\\thickapprox',
  '\\approxeq',
  '\\succapprox',
  '\\precapprox',
  '\\curvearrowleft',
  '\\curvearrowright',
  '\\digamma',
  '\\varkappa',
  '\\Bbbk',
  '\\hslash',
  '\\hbar',
  '\\backepsilon',
];

// ---------- Unicode ----------

/** Unicode → LaTeX for everything that is not a plain letter or digit. One
 * pair per line-ish: the character, a space, the command. */
const UNICODE_PAIRS = `
α \\alpha|β \\beta|γ \\gamma|δ \\delta|ε \\varepsilon|ϵ \\epsilon|ζ \\zeta|η \\eta|θ \\theta
ϑ \\vartheta|ι \\iota|κ \\kappa|ϰ \\varkappa|λ \\lambda|μ \\mu|ν \\nu|ξ \\xi|ο o|π \\pi
ϖ \\varpi|ρ \\rho|ϱ \\varrho|σ \\sigma|ς \\varsigma|τ \\tau|υ \\upsilon|φ \\varphi|ϕ \\phi
χ \\chi|ψ \\psi|ω \\omega|Γ \\Gamma|Δ \\Delta|Θ \\Theta|Λ \\Lambda|Ξ \\Xi|Π \\Pi|Σ \\Sigma
Υ \\Upsilon|Φ \\Phi|Ψ \\Psi|Ω \\Omega|Α A|Β B|Ε E|Ζ Z|Η H|Ι I|Κ K|Μ M|Ν N|Ο O|Ρ P|Τ T|Χ X
ϝ \\digamma|µ \\mu
− -|∓ \\mp|± \\pm|× \\times|÷ \\div|· \\cdot|⋅ \\cdot|∙ \\cdot|∗ \\ast|∘ \\circ|• \\bullet
⊕ \\oplus|⊖ \\ominus|⊗ \\otimes|⊘ \\oslash|⊙ \\odot|∪ \\cup|∩ \\cap|⊎ \\uplus|⊔ \\sqcup
⊓ \\sqcap|∧ \\wedge|∨ \\vee|∖ \\setminus|≀ \\wr|⋄ \\diamond|△ \\bigtriangleup
▽ \\bigtriangledown|◁ \\triangleleft|▷ \\triangleright|⋆ \\star|† \\dagger|‡ \\ddagger
⨿ \\amalg|⊞ \\boxplus|⊟ \\boxminus|⊠ \\boxtimes|⊡ \\boxdot|⋉ \\ltimes|⋊ \\rtimes|∔ \\dotplus
⊺ \\intercal|⋋ \\leftthreetimes|⋌ \\rightthreetimes|⋒ \\Cap|⋓ \\Cup|⋏ \\curlywedge|⋎ \\curlyvee
≤ \\leq|≥ \\geq|≠ \\neq|≡ \\equiv|≈ \\approx|≅ \\cong|∼ \\sim|≃ \\simeq|≍ \\asymp|∝ \\propto
≺ \\prec|≻ \\succ|⪯ \\preceq|⪰ \\succeq|≼ \\preccurlyeq|≽ \\succcurlyeq|≪ \\ll|≫ \\gg
⊂ \\subset|⊃ \\supset|⊆ \\subseteq|⊇ \\supseteq|⊊ \\subsetneq|⊋ \\supsetneq|⊈ \\nsubseteq
⊉ \\nsupseteq|∈ \\in|∉ \\notin|∋ \\ni|∌ \\not\\ni|⊢ \\vdash|⊣ \\dashv|⊨ \\models|⊩ \\Vdash
∣ \\mid|∥ \\parallel|⊥ \\perp|⌣ \\smile|⌢ \\frown|≐ \\doteq|≜ \\triangleq|≔ \\coloneqq
≦ \\leqq|≧ \\geqq|⩽ \\leqslant|⩾ \\geqslant|≲ \\lesssim|≳ \\gtrsim|≮ \\nless|≯ \\ngtr
≰ \\nleq|≱ \\ngeq|≁ \\nsim|≇ \\ncong|∤ \\nmid|∦ \\nparallel|⊏ \\sqsubset|⊐ \\sqsupset
⊑ \\sqsubseteq|⊒ \\sqsupseteq|≬ \\between|⋘ \\lll|⋙ \\ggg|∵ \\because|∴ \\therefore
≗ \\circeq|≖ \\eqcirc|≏ \\bumpeq|≎ \\Bumpeq|∽ \\backsim|⋖ \\lessdot|⋗ \\gtrdot|≶ \\lessgtr
≷ \\gtrless|⋚ \\lesseqgtr|⋛ \\gtreqless|⊲ \\lhd|⊳ \\rhd|⊴ \\unlhd|⊵ \\unrhd|≑ \\doteqdot
← \\leftarrow|→ \\to|↑ \\uparrow|↓ \\downarrow|↔ \\leftrightarrow|↕ \\updownarrow
⇐ \\Leftarrow|⇒ \\Rightarrow|⇑ \\Uparrow|⇓ \\Downarrow|⇔ \\Leftrightarrow|⇕ \\Updownarrow
↦ \\mapsto|↗ \\nearrow|↘ \\searrow|↖ \\nwarrow|↙ \\swarrow|⟵ \\longleftarrow
⟶ \\longrightarrow|⟷ \\longleftrightarrow|⟸ \\Longleftarrow|⟹ \\Longrightarrow
⟺ \\Longleftrightarrow|⟼ \\longmapsto|↪ \\hookrightarrow|↩ \\hookleftarrow
⇀ \\rightharpoonup|⇁ \\rightharpoondown|↼ \\leftharpoonup|↽ \\leftharpoondown
⇌ \\rightleftharpoons|⇋ \\leftrightharpoons|↠ \\twoheadrightarrow|↞ \\twoheadleftarrow
↣ \\rightarrowtail|↢ \\leftarrowtail|⇝ \\rightsquigarrow|↺ \\circlearrowleft
↻ \\circlearrowright|⇉ \\rightrightarrows|⇇ \\leftleftarrows|↶ \\curvearrowleft
↷ \\curvearrowright|⊸ \\multimap|⇄ \\rightleftarrows|⇆ \\leftrightarrows|↾ \\upharpoonright
∞ \\infty|∂ \\partial|∇ \\nabla|∀ \\forall|∃ \\exists|∄ \\nexists|¬ \\neg|∅ \\emptyset
ℵ \\aleph|ℶ \\beth|ℷ \\gimel|ℸ \\daleth|ℏ \\hbar|ℓ \\ell|℘ \\wp|ℜ \\Re|ℑ \\Im|′ '|″ ''|‴ '''
° ^\\circ|∠ \\angle|∡ \\measuredangle|∢ \\sphericalangle|⊤ \\top|♭ \\flat|♮ \\natural
♯ \\sharp|♣ \\clubsuit|♢ \\diamondsuit|♡ \\heartsuit|♠ \\spadesuit|∎ \\blacksquare
■ \\blacksquare|□ \\square|◇ \\lozenge|◊ \\lozenge|★ \\bigstar|✓ \\checkmark|ı \\imath
ȷ \\jmath|ð \\eth|Ⅎ \\Finv|⅁ \\Game|℧ \\mho|∁ \\complement|⋯ \\cdots|… \\ldots|⋮ \\vdots
⋱ \\ddots|§ \\S|¶ \\P|© \\copyright|£ \\pounds|‰ \\permil|ℎ h|ℯ e|ℊ g|ℴ o|ϒ \\Upsilon
⟨ \\langle|⟩ \\rangle|〈 \\langle|〉 \\rangle|⌊ \\lfloor|⌋ \\rfloor|⌈ \\lceil|⌉ \\rceil
{ \\{|} \\}|‖ \\||\u2016 \\||⟦ [\\![|⟧ ]\\!]
∑ \\sum|∏ \\prod|∐ \\coprod|∫ \\int|∬ \\iint|∭ \\iiint|∮ \\oint|⋃ \\bigcup|⋂ \\bigcap
⨁ \\bigoplus|⨂ \\bigotimes|⨀ \\bigodot|⋁ \\bigvee|⋀ \\bigwedge|⨄ \\biguplus|⨆ \\bigsqcup
√ radical|∛ radical3|∜ radical4
ℂ \\mathbb{C}|ℍ \\mathbb{H}|ℕ \\mathbb{N}|ℙ \\mathbb{P}|ℚ \\mathbb{Q}|ℝ \\mathbb{R}|ℤ \\mathbb{Z}
ℬ \\mathcal{B}|ℰ \\mathcal{E}|ℱ \\mathcal{F}|ℋ \\mathcal{H}|ℐ \\mathcal{I}|ℒ \\mathcal{L}
ℳ \\mathcal{M}|ℛ \\mathcal{R}|ℭ \\mathfrak{C}|ℌ \\mathfrak{H}|ℨ \\mathfrak{Z}
ˆ accent:\\hat|˜ accent:\\tilde|¯ accent:\\bar|‾ accent:\\bar|˙ accent:\\dot|¨ accent:\\ddot
ˇ accent:\\check|˘ accent:\\breve|´ accent:\\acute|˚ accent:\\mathring|⃗ accent:\\vec
̂ accent:\\hat|̃ accent:\\tilde|̄ accent:\\bar|̅ accent:\\bar|̇ accent:\\dot|̈ accent:\\ddot
̌ accent:\\check|̆ accent:\\breve
# \\#|$ \\$|% \\%|& \\&|_ \\_|~ \\sim|∶ :|∕ /|⁄ /|＋ +|＝ =
\u2061 |\u2062 |\u2063 |\u2064 |\u200b |\u00a0
`;

/** The table, parsed. A value of '' means "nothing to write" (invisible
 * operators, zero-width spaces). */
const UNICODE = (() => {
  const map = new Map();
  for (const line of UNICODE_PAIRS.split('\n')) {
    for (const pair of line.split('|')) {
      if (pair.length === 0) continue;
      // A pair is one character (possibly astral), one space, the rest.
      const ch = String.fromCodePoint(pair.codePointAt(0));
      const rest = pair.slice(ch.length + 1);
      if (!map.has(ch)) map.set(ch, rest);
    }
  }
  // `‖ \|` reads as "‖", "\", "" across the separator; fix it by hand.
  map.set('‖', '\\|');
  map.set('\u2016', '\\|');
  map.set('|', '|');
  return map;
})();

/** Mathematical Alphanumeric Symbols, U+1D400–1D7FF: style → [start of A,
 * start of a] (the lowercase block follows the uppercase one). Italic is the
 * default for a math letter and so writes as the plain letter. */
const ALPHA_STYLES = [
  ['\\mathbf', 0x1d400],
  ['', 0x1d434],
  ['\\boldsymbol', 0x1d468],
  ['\\mathcal', 0x1d49c],
  ['\\mathcal', 0x1d4d0],
  ['\\mathfrak', 0x1d504],
  ['\\mathbb', 0x1d538],
  ['\\mathfrak', 0x1d56c],
  ['\\mathsf', 0x1d5a0],
  ['\\mathsf', 0x1d5d4],
  ['\\mathsf', 0x1d608],
  ['\\mathsf', 0x1d63c],
  ['\\mathtt', 0x1d670],
];
/** Greek in the same block: bold, italic, bold italic, sans bold, sans bold
 * italic. 25 capitals (with ϴ) then ∇, then 25 lowercase, then ∂ and the
 * variant forms ϵ ϑ ϰ ϕ ϱ ϖ. */
const GREEK_STYLES = [
  ['\\boldsymbol', 0x1d6a8],
  ['', 0x1d6e2],
  ['\\boldsymbol', 0x1d71c],
  ['\\boldsymbol', 0x1d756],
  ['\\boldsymbol', 0x1d790],
];
const GREEK_CAPS = 'ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΘΣΤΥΦΧΨΩ∇'.split('');
const GREEK_SMALL = 'αβγδεζηθικλμνξοπρςστυφχψω∂ϵϑϰϕϱϖ'.split('');
const DIGIT_STYLES = [
  ['\\mathbf', 0x1d7ce],
  ['\\mathbb', 0x1d7d8],
  ['\\mathsf', 0x1d7e2],
  ['\\mathsf', 0x1d7ec],
  ['\\mathtt', 0x1d7f6],
];

/** A code point in the Mathematical Alphanumeric block → { style, base }, or
 * null. `base` is the plain letter, digit or Greek character. */
function mathAlpha(cp) {
  if (cp < 0x1d400 || cp > 0x1d7ff) return null;
  for (const [style, start] of ALPHA_STYLES) {
    if (cp >= start && cp < start + 52) {
      const i = cp - start;
      return { style, base: i < 26 ? UPPER[i] : LOWER[i - 26] };
    }
  }
  if (cp === 0x1d6a4) return { style: '', base: 'ı' };
  if (cp === 0x1d6a5) return { style: '', base: 'ȷ' };
  for (const [style, start] of GREEK_STYLES) {
    if (cp >= start && cp < start + 58) {
      const i = cp - start;
      const base = i < 26 ? GREEK_CAPS[i] : GREEK_SMALL[i - 26];
      // Slot 17 of the capitals is ϴ (capital theta symbol); it reads as Θ.
      return base ? { style, base } : null;
    }
  }
  if (cp === 0x1d7ca || cp === 0x1d7cb) return { style: '\\boldsymbol', base: 'ϝ' };
  for (const [style, start] of DIGIT_STYLES) {
    if (cp >= start && cp < start + 10) return { style, base: DIGITS[cp - start] };
  }
  return null;
}

/** The reverse direction, for the signature of TeX source: a style command
 * and a base letter → the styled code point (or the letter itself). */
function styledChar(style, base) {
  const letterlike = {
    '\\mathbb': { C: 'ℂ', H: 'ℍ', N: 'ℕ', P: 'ℙ', Q: 'ℚ', R: 'ℝ', Z: 'ℤ' },
    '\\mathcal': { B: 'ℬ', E: 'ℰ', F: 'ℱ', H: 'ℋ', I: 'ℐ', L: 'ℒ', M: 'ℳ', R: 'ℛ' },
    '\\mathfrak': { C: 'ℭ', H: 'ℌ', I: 'ℑ', R: 'ℜ', Z: 'ℨ' },
  };
  const hole = letterlike[style]?.[base];
  if (hole) return hole;
  const start = { '\\mathbf': 0x1d400, '\\mathcal': 0x1d49c, '\\mathfrak': 0x1d504 }[style];
  const bb = style === '\\mathbb' ? 0x1d538 : undefined;
  const at = start ?? bb;
  if (at === undefined) return base;
  const u = UPPER.indexOf(base);
  if (u >= 0) return String.fromCodePoint(at + u);
  const l = LOWER.indexOf(base);
  if (l >= 0) return String.fromCodePoint(at + 26 + l);
  const d = DIGITS.indexOf(base);
  if (d >= 0 && style === '\\mathbf') return String.fromCodePoint(0x1d7ce + d);
  if (d >= 0 && style === '\\mathbb') return String.fromCodePoint(0x1d7d8 + d);
  return base;
}

// ---------- symbol classes ----------

const REL = new Set(
  (
    '= < > : \\leq \\geq \\le \\ge \\neq \\ne \\equiv \\approx \\cong \\sim \\simeq \\asymp ' +
    '\\propto \\prec \\succ \\preceq \\succeq \\preccurlyeq \\succcurlyeq \\ll \\gg \\subset ' +
    '\\supset \\subseteq \\supseteq \\subsetneq \\supsetneq \\nsubseteq \\nsupseteq \\in \\notin ' +
    '\\ni \\vdash \\dashv \\models \\mid \\parallel \\perp \\doteq \\triangleq \\coloneqq \\leqq ' +
    '\\geqq \\leqslant \\geqslant \\lesssim \\gtrsim \\nless \\ngtr \\nleq \\ngeq \\nsim \\ncong ' +
    '\\nmid \\nparallel \\sqsubset \\sqsupset \\sqsubseteq \\sqsupseteq \\leftarrow \\to ' +
    '\\rightarrow \\gets \\uparrow \\downarrow \\leftrightarrow \\Leftarrow \\Rightarrow ' +
    '\\Leftrightarrow \\mapsto \\longrightarrow \\longleftarrow \\Longrightarrow \\Longleftarrow ' +
    '\\Longleftrightarrow \\longmapsto \\hookrightarrow \\hookleftarrow \\nearrow \\searrow ' +
    '\\nwarrow \\swarrow \\rightleftharpoons \\iff \\implies \\impliedby \\approxeq \\eqsim ' +
    '\\twoheadrightarrow \\rightsquigarrow \\Vdash \\vDash \\lneq \\gneq \\lneqq \\gneqq ' +
    '\\varpropto \\backsim \\thicksim \\thickapprox \\circeq \\bumpeq \\Bumpeq \\smile \\frown'
  ).split(' ')
);
const BIN = new Set(
  (
    '+ - \\pm \\mp \\times \\div \\cdot \\ast \\star \\circ \\bullet \\oplus \\ominus \\otimes ' +
    '\\oslash \\odot \\cup \\cap \\uplus \\sqcup \\sqcap \\wedge \\vee \\land \\lor \\setminus ' +
    '\\smallsetminus \\wr \\diamond \\amalg \\dagger \\ddagger \\boxplus \\boxminus \\boxtimes ' +
    '\\boxdot \\ltimes \\rtimes \\dotplus \\bigtriangleup \\bigtriangledown \\triangleleft ' +
    '\\triangleright \\centerdot \\intercal \\barwedge \\veebar \\curlywedge \\curlyvee ' +
    '\\leftthreetimes \\rightthreetimes \\divideontimes'
  ).split(' ')
);
const OPEN = new Set(['(', '[', '\\{', '\\langle', '\\lfloor', '\\lceil', '\\lbrace', '[\\![']);
const CLOSE = new Set([')', ']', '\\}', '\\rangle', '\\rfloor', '\\rceil', '\\rbrace', ']\\!]']);
const FENCE = new Set(['|', '\\|']);
const BIGOPS = new Set(
  (
    '\\sum \\prod \\coprod \\int \\iint \\iiint \\oint \\bigcup \\bigcap \\bigoplus \\bigotimes ' +
    '\\bigodot \\bigvee \\bigwedge \\biguplus \\bigsqcup'
  ).split(' ')
);
/** Function names TeX sets upright, and the ones that take limits in a
 * display (`\lim_{x \to 0}` puts the subscript underneath). */
const FUNCTIONS = new Set(
  (
    'sin cos tan cot sec csc sinh cosh tanh coth arcsin arccos arctan log ln lg exp lim ' +
    'liminf limsup max min sup inf det tr Pr arg deg dim gcd hom ker mod'
  ).split(' ')
);
const LIMIT_FUNCTIONS = new Set([
  'lim',
  'liminf',
  'limsup',
  'max',
  'min',
  'sup',
  'inf',
  'det',
  'Pr',
  'gcd',
]);
/** Functions KaTeX knows as commands; anything else upright and wordy is
 * \operatorname. `tr` is not one, and is common enough to special-case. */
const KNOWN_FUNCTION_COMMANDS = new Set([...FUNCTIONS].filter((f) => f !== 'tr'));

function classOf(tex) {
  if (REL.has(tex)) return 'rel';
  if (BIN.has(tex)) return 'bin';
  if (OPEN.has(tex)) return 'open';
  if (CLOSE.has(tex)) return 'close';
  if (FENCE.has(tex)) return 'fence';
  if (BIGOPS.has(tex)) return 'bigop';
  if (tex === ',' || tex === ';') return 'punct';
  return 'ord';
}

// ---------- decoding one glyph ----------

/** What a font name says about how to read its codes. */
function fontKind(name) {
  const raw = String(name ?? '').replace(/^[A-Z]{6}\+/, '');
  if (raw === '__rule') return { enc: 'rule', style: '' };
  const n = raw.toLowerCase().replace(/[-_ ,]/g, '');
  if (n.length === 0) return { enc: 'none', style: '' };
  if (/^(cmmib|lmmathitalic\d*bold|eurb)/.test(n)) return { enc: 'OML', style: '\\boldsymbol' };
  if (/^(cmmi|lmmathitalic|lmmi\d|n?txmi|n?pxmi|eurm)/.test(n)) return { enc: 'OML', style: '' };
  if (/^(cmbsy|lmmathsymbols\d*bold)/.test(n)) return { enc: 'OMS', style: '\\boldsymbol' };
  if (/^(cmsy|lmmathsymbols|lmsy\d|n?txsy(?![ab])|n?pxsy(?![ab]))/.test(n)) {
    return { enc: 'OMS', style: '' };
  }
  if (/^(cmex|lmmathextension|lmex\d|n?txex|n?pxex)/.test(n)) return { enc: 'OMX', style: '' };
  if (/^(msam|n?txsya|n?pxsya)/.test(n)) return { enc: 'MSAM', style: '' };
  if (/^(msbm|n?txsyb|n?pxsyb)/.test(n)) return { enc: 'MSBM', style: '' };
  if (/^(eufm|eufb)/.test(n)) return { enc: 'ALPHA', style: '\\mathfrak' };
  if (/^rsfs/.test(n)) return { enc: 'ALPHA', style: '\\mathscr' };
  if (/^(eusm|eusb)/.test(n)) return { enc: 'ALPHA', style: '\\mathcal' };
  if (/^(cmbx|cmb\d|cmssbx)/.test(n)) return { enc: 'OT1', style: '\\mathbf', text: true };
  if (/^(cmti|cmbxti|cmmib)/.test(n)) return { enc: 'OT1', style: '\\mathit', text: true };
  if (/^(cmss)/.test(n)) return { enc: 'OT1', style: '\\mathsf', text: true };
  if (/^(cmtt|cmsltt)/.test(n)) return { enc: 'OT1', style: '\\mathtt', text: true };
  if (/^(cmr\d|cmsl|cmdunh|cmfib)/.test(n)) return { enc: 'OT1', style: '', text: true };
  // cm-super (SF*), EC fonts and Latin Modern text: T1, which agrees with
  // ASCII where math uses it; their ToUnicode is reliable.
  if (/^(sf|ec)(bx|bi)/.test(n) || /^lm(roman|sans|mono).*bold/.test(n)) {
    return { enc: 'T1', style: '\\mathbf', text: true };
  }
  if (/^(sf|ec)(ti|it|sl|si)/.test(n) || /^lm(roman|sans|mono).*(italic|oblique)/.test(n)) {
    return { enc: 'T1', style: '\\mathit', text: true };
  }
  if (/^(sfss|ecss|lmsans)/.test(n)) return { enc: 'T1', style: '\\mathsf', text: true };
  if (/^(sftt|ectt|lmmono)/.test(n)) return { enc: 'T1', style: '\\mathtt', text: true };
  if (/^(sfrm|ecrm|lmroman)/.test(n)) return { enc: 'T1', style: '', text: true };
  return { enc: 'UNI', style: '', math: /math|stix|cambria|asana|xits|tex.?gyre.*math/.test(n) };
}

/** A decoded entry (table string, `[tex, kind, size]`, or Unicode result)
 * as an atom's identity: `{ tex, kind, ... }`. */
function fromEntry(entry) {
  if (entry == null) return null;
  if (Array.isArray(entry)) {
    const [tex, kind, extra] = entry;
    if (kind === 'delim') return { tex, kind: classOf(tex), delimSize: extra, tall: true };
    if (kind === 'bigop') return { tex, kind: 'bigop', tall: true };
    if (kind === 'accent') return { tex, kind: 'accent', wide: true };
    if (kind === 'radical') return { tex, kind: 'radical', tall: true };
    return { tex, kind: 'piece', side: extra };
  }
  if (entry.startsWith('accent:')) {
    const tex = entry.slice(7);
    return { tex, kind: 'accent', wide: tex.startsWith('\\wide') };
  }
  if (entry.startsWith('piece:')) return { tex: entry.slice(6), kind: 'piece', side: '?' };
  if (entry.startsWith('radical')) return { tex: '\\sqrt', kind: 'radical', index: entry.slice(7) };
  if (/^[A-Za-z]$/.test(entry)) return { tex: entry, kind: 'letter' };
  if (/^[0-9]$/.test(entry)) return { tex: entry, kind: 'ord' };
  if (entry === "'" || entry === "''" || entry === "'''") return { tex: entry, kind: 'prime' };
  if (entry === '\\prime') return { tex: entry, kind: 'prime' };
  if (entry === '\\surd') return { tex: '\\sqrt', kind: 'radical' };
  return { tex: entry, kind: classOf(entry) };
}

/** Unicode text of a glyph → atom identity, or `{ tex, kind, ok: false }`
 * when the character is not one this knows. */
function fromUnicode(text) {
  const chars = Array.from(text);
  if (chars.length === 0) return null;
  if (chars.length > 1) {
    // A ligature or a multi-character mapping: letters stay letters.
    if (/^[A-Za-z]+$/.test(text)) return { tex: text, kind: 'letter' };
    const parts = chars.map(fromUnicode).filter(Boolean);
    const tex = parts.map((p) => p.tex).join('');
    if (tex.length === 0) return { tex: '', kind: 'space' };
    return { tex, kind: parts[0].kind, ok: parts.every((p) => p.ok !== false) };
  }
  const ch = chars[0];
  const cp = ch.codePointAt(0);
  const alpha = mathAlpha(cp);
  if (alpha) {
    const inner = fromUnicode(alpha.base) ?? { tex: alpha.base, kind: 'ord' };
    if (!alpha.style) return { ...inner, italic: true };
    return { tex: `${alpha.style}{${inner.tex}}`, kind: 'ord' };
  }
  if (UNICODE.has(ch)) {
    const mapped = UNICODE.get(ch);
    if (mapped === '') return { tex: '', kind: 'space' };
    return fromEntry(mapped);
  }
  if (/^[A-Za-z]$/.test(ch)) return { tex: ch, kind: 'letter' };
  if (/^[0-9]$/.test(ch)) return { tex: ch, kind: 'ord' };
  if (/^\s$/.test(ch)) return { tex: '', kind: 'space' };
  if ('+-=<>()[]/,;:.!?*|@'.includes(ch)) return fromEntry(ch);
  if (ch === '^') return { tex: '\\hat', kind: 'accent' };
  if (ch === '\\') return { tex: '\\backslash', kind: 'ord' };
  if (cp >= 0x300 && cp < 0x370) return { tex: '', kind: 'space' };
  // Accented Latin letters and the like: KaTeX renders them as text.
  if (/\p{L}/u.test(ch)) return { tex: ch, kind: 'letter', ok: true };
  return { tex: ch, kind: 'ord', ok: false };
}

/** One glyph → an atom: its identity plus its geometry. `ok` is false when
 * nothing could say what the glyph is. */
function decodeGlyph(g) {
  const font = fontKind(g.font);
  const geometry = {
    x: num(g.x),
    y: num(g.y),
    w: num(g.w),
    h: num(g.h),
    baseline: Number.isFinite(g.baseline) ? g.baseline : num(g.y) + num(g.h) * 0.8,
    size: num(g.size) || num(g.h) || 10,
  };
  if (font.enc === 'rule') return { tex: '', kind: 'rule', ok: true, ...geometry };
  const code = Number.isInteger(g.code) ? g.code : -1;
  const text = typeof g.text === 'string' ? g.text : '';
  let id = null;
  let style = font.style;
  switch (font.enc) {
    case 'OML':
      id = fromEntry(OML[code]);
      break;
    case 'OMS':
      id = fromEntry(OMS[code]);
      break;
    case 'OMX':
      id = fromEntry(OMX[code]);
      break;
    case 'MSAM':
      id = fromEntry(MSAM[code]);
      break;
    case 'MSBM':
      id = fromEntry(MSBM[code]);
      break;
    case 'OT1':
      id = fromEntry(OT1[code]);
      break;
    case 'ALPHA': {
      const ch = code >= 33 && code < 127 ? String.fromCharCode(code) : text;
      if (/^[A-Za-z]$/.test(ch)) {
        id = { tex: `${style}{${ch}}`, kind: 'ord' };
        style = '';
      }
      break;
    }
    default:
      break;
  }
  if (!id && text) id = fromUnicode(text);
  if (!id && (font.enc === 'T1' || font.enc === 'OT1') && code > 32 && code < 127) {
    id = fromUnicode(String.fromCharCode(code));
  }
  if (!id) return { tex: '', kind: 'unknown', ok: false, ...geometry };
  const upright =
    id.kind === 'letter' &&
    !id.italic &&
    (font.text ? style !== '\\mathit' : font.enc === 'UNI' && !g.italic && !font.math);
  // A text font's own style (bold, sans, typewriter) wraps what it sets; in
  // a word it wraps the word instead.
  return {
    ok: id.ok !== false,
    ...id,
    style: id.kind === 'letter' || id.kind === 'ord' ? style : '',
    upright,
    textFont: Boolean(font.text),
    enc: font.enc,
    ...geometry,
  };
}

function num(v) {
  return Number.isFinite(v) ? v : 0;
}

/** Whether these glyphs are an OCR layer or an image with no fonts: nothing
 * in them says what the symbols are, so nothing should be made up. */
function looksLikeScan(glyphs) {
  const real = glyphs.filter((g) => g.font !== '__rule');
  if (real.length === 0) return false;
  const fontless = real.filter(
    (g) => !g.font || /glyphless|^ocr|tesseract|abbyy|invisible/i.test(String(g.font))
  ).length;
  return fontless / real.length > 0.5;
}

// ---------- structure from geometry ----------
//
// The glyphs of a formula are a flat list of positioned symbols; TeX's
// structure survives only as geometry. It is read back in a fixed order,
// innermost first, each step replacing what it recognised with one composite
// atom so the next step sees a simpler picture:
//
//   1. pieces of tall delimiters (top, extender, bottom) → one delimiter;
//   2. accents → the accented symbol;
//   3. rules, narrowest first → fractions, radicals, over/underlines;
//   4. rows: one baseline is a line of math; several are a matrix (when a
//      tall delimiter spans them), cases (a lone tall brace) or aligned rows;
//   5. within a row: words of upright letters, big operators and their
//      limits, then sub- and superscripts by size and baseline, recursively.

const right = (a) => a.x + a.w;
const bottom = (a) => a.y + a.h;
const cx = (a) => a.x + a.w / 2;
const cy = (a) => a.y + a.h / 2;

function union(items) {
  const x = Math.min(...items.map((a) => a.x));
  const y = Math.min(...items.map((a) => a.y));
  return {
    x,
    y,
    w: Math.max(...items.map(right)) - x,
    h: Math.max(...items.map(bottom)) - y,
  };
}

function maxSize(items) {
  let s = 0;
  for (const a of items) if (a.kind !== 'rule' && a.size > s) s = a.size;
  return s || 10;
}

/** Reconstruct LaTeX from glyphs. Returns `{ latex, confidence, rows,
 * decoded, undecoded }`. */
function reconstruct(glyphs) {
  const flags = { undecoded: 0, ambiguous: 0 };
  let atoms = glyphs.map(decodeGlyph);
  const real = atoms.filter((a) => a.kind !== 'rule');
  flags.undecoded = real.filter((a) => !a.ok).length;
  atoms = atoms.filter((a) => a.kind !== 'space' && (a.ok || a.kind !== 'unknown'));
  atoms = atoms.filter((a) => !(a.kind !== 'rule' && a.tex === ''));
  if (atoms.length === 0) {
    return { latex: '', confidence: 'low', rows: 0, decoded: 0, undecoded: flags.undecoded };
  }
  atoms = mergePieces(atoms, flags);
  atoms = attachAccents(atoms, flags);
  atoms = applyRules(atoms, flags);
  const ctx = { flags, rows: 1 };
  const latex = tidy(layoutBlock(atoms, 0, ctx, true));
  const decoded = real.length - flags.undecoded;
  let confidence = 'high';
  if (flags.undecoded > 0 || flags.ambiguous > 0) confidence = 'medium';
  if (decoded === 0 || flags.undecoded > real.length / 4 || flags.ambiguous > 2) confidence = 'low';
  return { latex, confidence, rows: ctx.rows, decoded, undecoded: flags.undecoded };
}

/** Pieces of an extensible delimiter, stacked in one column, are one
 * delimiter; pieces of a radical are one radical. */
function mergePieces(atoms, flags) {
  const pieces = atoms.filter((a) => a.kind === 'piece' && a.enc === 'OMX');
  if (pieces.length === 0) return atoms;
  const rest = atoms.filter((a) => !(a.kind === 'piece' && a.enc === 'OMX'));
  const groups = [];
  for (const p of [...pieces].sort((a, b) => a.y - b.y)) {
    const group = groups.find((g) =>
      g.some(
        (q) =>
          Math.min(right(p), right(q)) - Math.max(p.x, q.x) > 0.4 * Math.min(p.w, q.w) &&
          p.y - bottom(q) < 0.6 * p.size
      )
    );
    if (group) group.push(p);
    else groups.push([p]);
  }
  for (const group of groups) {
    const named = group.find((p) => p.tex !== 'brace' && p.side !== '?') ?? group[0];
    let tex = named.tex;
    if (tex === 'brace') tex = '\\{';
    if (tex === 'brace-tip') {
      flags.ambiguous++;
      continue;
    }
    const box = union(group);
    const base = { ...named, ...box, baseline: bottom(box), ok: true, tall: true, delimSize: 5 };
    if (tex === '\\sqrt') rest.push({ ...base, tex, kind: 'radical' });
    else rest.push({ ...base, tex, kind: classOf(tex) });
  }
  return rest;
}

/** An accent sits over what it accents: the symbol it overlaps most,
 * horizontally, nearby. A wide accent covers everything under it. */
function attachAccents(atoms, flags) {
  const accents = atoms.filter((a) => a.kind === 'accent');
  if (accents.length === 0) return atoms;
  let rest = atoms.filter((a) => a.kind !== 'accent');
  for (const acc of accents.sort((a, b) => a.w - b.w)) {
    const near = rest.filter(
      (b) =>
        b.kind !== 'rule' &&
        Math.abs(cy(b) - cy(acc)) < 1.2 * Math.max(b.size, acc.size) &&
        cy(b) >= cy(acc) - 0.1 * acc.size
    );
    let under;
    if (acc.wide) {
      under = near.filter(
        (b) => cx(b) >= acc.x - 0.1 * acc.size && cx(b) <= right(acc) + 0.1 * acc.size
      );
    } else {
      const overlap = (b) => Math.min(right(b), right(acc)) - Math.max(b.x, acc.x);
      const best = near
        .filter((b) => overlap(b) > 0.3 * Math.min(b.w, acc.w))
        .sort((a, b) => overlap(b) - overlap(a))[0];
      under = best ? [best] : [];
    }
    if (under.length === 0) {
      flags.ambiguous++;
      continue;
    }
    under.sort((a, b) => a.x - b.x);
    const inner = under.length === 1 ? under[0].tex : layoutBlock(under, 1, { flags, rows: 1 });
    const anchor = under.reduce((a, b) => (b.size > a.size ? b : a));
    rest = rest.filter((b) => !under.includes(b));
    rest.push({
      ...anchor,
      tex: `${acc.tex}{${inner}}`,
      kind: 'ord',
      style: '',
      upright: false,
      composite: 'accent',
      ...union([acc, ...under]),
      baseline: anchor.baseline,
      size: anchor.size,
    });
  }
  return rest;
}

/** Rules, narrowest first, so a fraction inside a fraction is read before
 * the one that contains it. */
function applyRules(atoms, flags) {
  const rules = atoms.filter((a) => a.kind === 'rule' && a.w > a.h * 2);
  let rest = atoms.filter((a) => a.kind !== 'rule');
  const pending = [...rules].sort((a, b) => a.w - b.w);
  while (pending.length > 0) {
    const rule = pending.shift();
    const blockers = pending;
    const S = localSize(rest, rule);
    const tol = 0.15 * S;
    const inX = (b) => cx(b) >= rule.x - tol && cx(b) <= right(rule) + tol;

    // A radical: its sign ends where the rule begins, and the rule is its top.
    const radical = rest.find(
      (r) =>
        r.kind === 'radical' &&
        Math.abs(right(r) - rule.x) < 0.5 * S &&
        r.y <= rule.y + 0.5 * S &&
        bottom(r) > rule.y
    );
    if (radical) {
      const content = rest.filter(
        (b) =>
          b !== radical &&
          inX(b) &&
          cy(b) > rule.y &&
          b.y >= rule.y - tol &&
          cy(b) < bottom(radical) + tol
      );
      const index = rest.filter(
        (b) =>
          b !== radical &&
          !content.includes(b) &&
          right(b) <= radical.x + radical.w * 0.75 &&
          b.x >= radical.x - 1.5 * S &&
          b.baseline <= cy(radical) + tol &&
          b.size < 0.85 * S &&
          bottom(b) > radical.y - 0.5 * S
      );
      const body = content.length > 0 ? layoutBlock(content, 0, { flags, rows: 1 }) : '';
      const idx = index.length > 0 ? `[${layoutBlock(index, 1, { flags, rows: 1 })}]` : '';
      const used = new Set([radical, ...content, ...index]);
      const anchor =
        content.length > 0 ? content.reduce((a, b) => (b.size > a.size ? b : a)) : radical;
      rest = rest.filter((b) => !used.has(b));
      rest.push({
        tex: `\\sqrt${idx}{${body}}`,
        kind: 'ord',
        ok: true,
        composite: 'radical',
        ...union([radical, rule, ...content, ...index]),
        baseline: content.length > 0 ? anchor.baseline : radical.baseline,
        size: content.length > 0 ? maxSize(content) : radical.size,
      });
      continue;
    }

    const side = (above) => gatherSide(rest, rule, above, S, blockers, inX);
    const num = side(true);
    const den = side(false);
    if (num.length > 0 && den.length > 0) {
      const size = maxSize([...num, ...den]);
      const used = new Set([...num, ...den]);
      rest = rest.filter((b) => !used.has(b));
      rest.push({
        tex: `\\frac{${layoutBlock(num, 0, { flags, rows: 1 })}}{${layoutBlock(den, 0, { flags, rows: 1 })}}`,
        kind: 'ord',
        ok: true,
        composite: 'frac',
        ...union([rule, ...num, ...den]),
        axis: rule.y + rule.h / 2,
        baseline: rule.y + rule.h / 2 + 0.25 * size,
        size,
      });
    } else if (den.length > 0 && den.every((b) => b.y >= rule.y - tol)) {
      const used = new Set(den);
      rest = rest.filter((b) => !used.has(b));
      const anchor = den.reduce((a, b) => (b.size > a.size ? b : a));
      rest.push({
        tex: `\\overline{${layoutBlock(den, 0, { flags, rows: 1 })}}`,
        kind: 'ord',
        ok: true,
        composite: 'overline',
        ...union([rule, ...den]),
        baseline: anchor.baseline,
        size: maxSize(den),
      });
    } else if (num.length > 0) {
      const used = new Set(num);
      rest = rest.filter((b) => !used.has(b));
      const anchor = num.reduce((a, b) => (b.size > a.size ? b : a));
      rest.push({
        tex: `\\underline{${layoutBlock(num, 0, { flags, rows: 1 })}}`,
        kind: 'ord',
        ok: true,
        composite: 'underline',
        ...union([rule, ...num]),
        baseline: anchor.baseline,
        size: maxSize(num),
      });
    } else {
      flags.ambiguous++;
    }
  }
  return rest;
}

/** The size of the symbols around a rule, which sets every tolerance near it. */
function localSize(atoms, rule) {
  const near = atoms.filter(
    (b) => cx(b) >= rule.x - 2 && cx(b) <= right(rule) + 2 && Math.abs(cy(b) - rule.y) < 30
  );
  return near.length > 0 ? maxSize(near) : maxSize(atoms);
}

/** What sits on one side of a rule: the symbols right against it, and
 * whatever is stacked onto those (their scripts, a fraction above them),
 * never crossing another rule. */
function gatherSide(atoms, rule, above, S, blockers, inX) {
  const ruleMid = rule.y + rule.h / 2;
  const onSide = (b) => (above ? cy(b) < ruleMid : cy(b) > ruleMid);
  const gap = (b) => (above ? ruleMid - bottom(b) : b.y - ruleMid);
  const candidates = atoms.filter((b) => inX(b) && onSide(b));
  const blocked = (b) =>
    blockers.some(
      (r) =>
        Math.min(right(r), right(b)) - Math.max(r.x, b.x) > 0 &&
        (above
          ? r.y + r.h / 2 < ruleMid && r.y + r.h / 2 > cy(b)
          : r.y + r.h / 2 > ruleMid && r.y + r.h / 2 < cy(b))
    );
  const members = candidates.filter((b) => gap(b) < 0.9 * S && gap(b) > -0.6 * S && !blocked(b));
  let grew = true;
  while (grew) {
    grew = false;
    for (const b of candidates) {
      if (members.includes(b) || blocked(b)) continue;
      const touches = members.some((m) => {
        const vgap = Math.max(m.y - bottom(b), b.y - bottom(m));
        return vgap < 0.25 * S;
      });
      if (touches) {
        members.push(b);
        grew = true;
      }
    }
  }
  return members;
}

// ---------- blocks and rows ----------

const isTall = (a) => Boolean(a.tall);

/** A block of atoms → LaTeX. `depth` is 0 at the top and inside fractions,
 * more inside scripts (where spacing is dropped). */
function layoutBlock(items, depth, ctx, top = false) {
  if (items.length === 0) return '';
  if (items.length === 1 && !items[0].tall && !(items[0].kind === 'letter' && items[0].textFont)) {
    return wrapStyle(items[0]);
  }
  const B = maxSize(items);
  const anchors = anchorsOf(items, B);
  const clusters = clusterBaselines(anchors, B);
  if (clusters.length <= 1) {
    const baseline =
      clusters.length === 1 ? clusters[0].baseline : median(items.map((a) => a.baseline));
    return segmentsToTex(rowSegments(items, baseline, B, depth, ctx), depth);
  }

  // Several rows. A tall delimiter spanning them makes them a matrix (or
  // cases); otherwise they are lines of one display.
  const grid = findGrid(items, clusters, B, depth, ctx);
  if (grid) {
    if (top) ctx.rows = Math.max(ctx.rows, clusters.length);
    return layoutBlock(grid, depth, ctx, top);
  }

  const rows = assignRows(items, clusters, B);
  if (top) ctx.rows = Math.max(ctx.rows, rows.length);
  const segs = rows.map((row) => rowSegments(row.items, row.baseline, B, depth, ctx));
  return alignedRows(segs, B, depth);
}

function anchorsOf(items, B) {
  const anchors = items.filter((a) => !isTall(a) && a.size >= 0.85 * B);
  return anchors.length > 0 ? anchors : items.filter((a) => !isTall(a));
}

function median(values) {
  if (values.length === 0) return 0;
  const s = [...values].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}

/** Baselines of the full-size symbols, grouped: a new group starts more
 * than 0.6 of the size below the last. */
function clusterBaselines(anchors, B) {
  const sorted = [...anchors].sort((a, b) => a.baseline - b.baseline);
  const clusters = [];
  for (const a of sorted) {
    const last = clusters[clusters.length - 1];
    if (last && a.baseline - last.max <= 0.6 * B) {
      last.items.push(a);
      last.max = a.baseline;
    } else {
      clusters.push({ items: [a], max: a.baseline });
    }
  }
  for (const c of clusters) c.baseline = median(c.items.map((a) => a.baseline));
  return clusters;
}

/** Every atom to the row whose baseline it is nearest; a tall atom by its
 * middle. */
function assignRows(items, clusters, B) {
  const rows = clusters.map((c) => ({ baseline: c.baseline, items: [] }));
  for (const a of items) {
    const y = isTall(a) ? cy(a) + 0.25 * B : a.baseline;
    let best = rows[0];
    for (const r of rows) if (Math.abs(r.baseline - y) < Math.abs(best.baseline - y)) best = r;
    best.items.push(a);
  }
  return rows.filter((r) => r.items.length > 0);
}

/** A tall delimiter pair around two or more rows is a matrix; a tall left
 * brace alone is `cases`. Returns the items with the grid replaced by one
 * composite, or null. */
function findGrid(items, clusters, B, depth, ctx) {
  const spans = (d) =>
    clusters.filter((c) => c.baseline > d.y - 0.1 * B && c.baseline - 0.5 * B < bottom(d)).length >=
    2;
  const tall = items
    .filter((a) => isTall(a) && ['open', 'close', 'fence'].includes(a.kind) && spans(a))
    .sort((a, b) => a.x - b.x);
  for (let i = 0; i < tall.length; i++) {
    const left = tall[i];
    if (left.kind === 'close') continue;
    const partner = tall
      .slice(i + 1)
      .find(
        (d) =>
          (d.kind === 'close' || d.kind === 'fence') &&
          Math.abs(cy(d) - cy(left)) < 0.5 * B &&
          (left.kind !== 'fence' || d.tex === left.tex)
      );
    const lo = left.y - 0.2 * B;
    const hi = bottom(left) + 0.2 * B;
    const inside = items.filter(
      (a) =>
        a !== left &&
        a !== partner &&
        cx(a) > right(left) &&
        (!partner || cx(a) < partner.x) &&
        cy(a) > lo &&
        cy(a) < hi
    );
    if (inside.length === 0) continue;
    const innerClusters = clusterBaselines(anchorsOf(inside, maxSize(inside)), maxSize(inside));
    if (innerClusters.length < 2) continue;
    let env = null;
    if (partner) {
      env =
        {
          '(': 'pmatrix',
          '[': 'bmatrix',
          '\\{': 'Bmatrix',
          '|': 'vmatrix',
          '\\|': 'Vmatrix',
        }[left.tex] ?? null;
    } else if (left.tex === '\\{') {
      env = 'cases';
    }
    if (!env && !partner) continue;
    const rows = assignRows(inside, innerClusters, maxSize(inside));
    const cells = gridCells(rows, maxSize(inside), depth, ctx, env === 'cases' ? 2 : 0);
    const body = cells.map((r) => r.join(' & ')).join(' \\\\ ');
    const tex = env
      ? `\\begin{${env}} ${body} \\end{${env}}`
      : `\\left${left.tex} \\begin{matrix} ${body} \\end{matrix} \\right${partner.tex}`;
    const used = new Set([left, partner, ...inside]);
    const box = union([left, ...(partner ? [partner] : []), ...inside]);
    const mid = box.y + box.h / 2;
    return [
      ...items.filter((a) => !used.has(a)),
      {
        tex,
        kind: 'ord',
        ok: true,
        composite: 'grid',
        ...box,
        axis: mid,
        baseline: mid + 0.25 * B,
        size: B,
      },
    ];
  }
  return null;
}

/** Rows of a grid → cells: each row is cut where the gap is wide (a column
 * separator, not a space), and cells are put in the column whose position
 * they share. For cases, at most two columns: value and condition. */
function gridCells(rows, B, depth, ctx, maxColumns) {
  const split = rows.map((row) => {
    const segs = rowSegments(row.items, row.baseline, B, depth, ctx);
    const cells = [];
    let cur = [];
    for (const s of segs) {
      const prev = cur[cur.length - 1];
      if (prev && s.x - prev.right > 0.75 * B) {
        cells.push(cur);
        cur = [];
      }
      cur.push(s);
    }
    if (cur.length) cells.push(cur);
    if (maxColumns && cells.length > maxColumns) {
      const head = cells.slice(0, maxColumns - 1);
      head.push(cells.slice(maxColumns - 1).flat());
      return head;
    }
    return cells;
  });
  const reference = split.reduce((a, b) => (b.length > a.length ? b : a));
  const columns = reference.map((c) => ({ x: c[0].x, right: c[c.length - 1].right }));
  return split.map((cells) => {
    const out = columns.map(() => '');
    for (const cell of cells) {
      const x0 = cell[0].x;
      const x1 = cell[cell.length - 1].right;
      let best = 0;
      let bestScore = -Infinity;
      columns.forEach((col, i) => {
        const score =
          Math.min(x1, col.right) -
          Math.max(x0, col.x) -
          Math.abs((x0 + x1) / 2 - (col.x + col.right) / 2) * 0.01;
        if (score > bestScore) {
          bestScore = score;
          best = i;
        }
      });
      out[best] = out[best]
        ? `${out[best]} ${segmentsToTex(cell, depth)}`
        : segmentsToTex(cell, depth);
    }
    return out;
  });
}

/** Lines of a display. Relations that line up across the lines are the
 * alignment points of an `aligned`; lines with nothing in common are
 * `gathered`. */
function alignedRows(segRows, B, depth) {
  const relX = segRows.map((segs) => {
    const rel =
      segs.find((s, i) => s.kind === 'rel' && i > 0) ?? segs.find((s) => s.kind === 'rel');
    return rel ? rel.x : null;
  });
  const xs = relX.filter((x) => x !== null);
  let alignX = null;
  for (const x of xs) {
    if (xs.filter((y) => Math.abs(y - x) < 0.4 * B).length >= 2) {
      alignX = x;
      break;
    }
  }
  if (alignX === null) {
    return `\\begin{gathered} ${segRows.map((s) => segmentsToTex(s, depth)).join(' \\\\ ')} \\end{gathered}`;
  }
  const lines = segRows.map((segs) => {
    const at = segs.findIndex((s) => s.kind === 'rel' && Math.abs(s.x - alignX) < 0.4 * B);
    if (at >= 0) {
      const head = segmentsToTex(segs.slice(0, at), depth);
      const tail = segmentsToTex(segs.slice(at), depth);
      return head ? `${head} &${tail}` : `&${tail}`;
    }
    if (segs.length > 0 && segs[0].x > alignX) return `&\\quad ${segmentsToTex(segs, depth)}`;
    return segmentsToTex(segs, depth);
  });
  return `\\begin{aligned} ${lines.join(' \\\\ ')} \\end{aligned}`;
}

// ---------- one row ----------

/** A row → segments `{ tex, kind, x, right }` in reading order: each base
 * symbol with its limits and scripts attached. */
function rowSegments(items, baseline, B, depth, ctx) {
  const flags = ctx.flags;
  const axis = baseline - 0.25 * B;
  const isBase = (a) => {
    if (isTall(a)) return true;
    if (a.axis !== undefined) return Math.abs(a.axis - axis) < 0.35 * B;
    if (a.composite) return a.size >= 0.85 * B && Math.abs(a.baseline - baseline) < 0.35 * B;
    if (a.size >= 0.85 * B) return Math.abs(a.baseline - baseline) < 0.35 * B;
    return Math.abs(a.baseline - baseline) < 0.06 * B;
  };
  let bases = items.filter(isBase).sort((a, b) => a.x - b.x);
  let small = items.filter((a) => !bases.includes(a));

  bases = groupWords(bases, B);

  // Limits: small things centred over or under a big operator (or a
  // function like lim that takes limits).
  const limits = new Map();
  for (const op of bases) {
    const takes = op.kind === 'bigop' || op.limits;
    if (!takes || small.length === 0) continue;
    const overlapsOp = (a) => Math.min(right(a), right(op)) - Math.max(a.x, op.x) > 0;
    const upperSeed = small.filter((a) => overlapsOp(a) && bottom(a) <= op.y + 0.35 * op.h);
    const lowerSeed = small.filter((a) => overlapsOp(a) && a.y >= op.y + 0.6 * op.h);
    const upper = chainRow(upperSeed, small, B);
    const lower = chainRow(lowerSeed, small, B).filter((a) => !upper.includes(a));
    if (upper.length || lower.length) {
      limits.set(op, { upper, lower });
      small = small.filter((a) => !upper.includes(a) && !lower.includes(a));
    }
  }

  // Scripts: everything else small belongs to the base on its left.
  const scripts = new Map(bases.map((b) => [b, { sup: [], sub: [] }]));
  const pre = { sup: [], sub: [] };
  for (const a of small.sort((p, q) => p.x - q.x)) {
    let owner = null;
    for (const b of bases) if (cx(b) <= a.x + 0.05 * B) owner = b;
    const slot = owner ? scripts.get(owner) : pre;
    if (a.baseline < baseline - 0.06 * B) slot.sup.push(a);
    else slot.sub.push(a);
  }

  const segments = [];
  if (pre.sup.length || pre.sub.length) {
    flags.ambiguous++;
    segments.push({
      tex: `{}${scriptTex(pre.sub, pre.sup, depth, ctx)}`,
      kind: 'ord',
      x: Math.min(...[...pre.sup, ...pre.sub].map((a) => a.x)),
      right: Math.max(...[...pre.sup, ...pre.sub].map(right)),
    });
  }
  for (const b of bases) {
    let tex = baseTex(b);
    const lim = limits.get(b);
    if (lim) {
      if (lim.lower.length) tex += `_${braced(layoutBlock(lim.lower, depth + 1, ctx))}`;
      if (lim.upper.length) tex += `^${braced(layoutBlock(lim.upper, depth + 1, ctx))}`;
    }
    const s = scripts.get(b);
    tex += scriptTex(s.sub, s.sup, depth, ctx);
    const all = [b, ...s.sub, ...s.sup];
    segments.push({
      tex,
      kind: b.kind,
      x: b.x,
      right: Math.max(...all.map(right)),
      base: b,
    });
  }
  return pairDelimiters(fixSequences(segments, baseline, B), flags);
}

/** A seed of limit symbols plus the small symbols beside them on the same
 * level ("i" over the sum, then "=1" running past its edge). */
function chainRow(seed, pool, B) {
  const out = [...seed];
  let grew = seed.length > 0;
  while (grew) {
    grew = false;
    for (const a of pool) {
      if (out.includes(a)) continue;
      const touch = out.some(
        (m) =>
          Math.abs(cy(a) - cy(m)) < 0.5 * m.size &&
          Math.max(a.x - right(m), m.x - right(a)) < 0.35 * B
      );
      if (touch) {
        out.push(a);
        grew = true;
      }
    }
  }
  return out;
}

function scriptTex(sub, sup, depth, ctx) {
  let tex = '';
  if (sub.length) tex += `_${braced(layoutBlock(sub, depth + 1, ctx))}`;
  if (sup.length) {
    const primes = sup.every((a) => a.kind === 'prime');
    if (primes) tex += sup.map(() => "'").join('');
    else tex += `^${braced(layoutBlock(sup, depth + 1, ctx))}`;
  }
  // x_i' is written x'_i.
  const m = /^(_(?:\{.*\}|\\[A-Za-z]+|.))('+)$/.exec(tex);
  return m ? m[2] + m[1] : tex;
}

/** Runs of upright letters set side by side are words: a function name
 * (\sin), or text in math (\text{for all}). A single upright letter from a
 * TeX text font is \mathrm. */
function groupWords(bases, B) {
  const out = [];
  let run = [];
  const flush = () => {
    if (run.length === 0) return;
    const word = run;
    run = [];
    const letters = word.map((a) => a.tex).join('');
    const textFont = word.some((a) => a.textFont);
    const style = word[0].style;
    if (word.length === 1 && !textFont && word[0].enc === 'UNI') {
      out.push({ ...word[0], upright: false });
      return;
    }
    const box = union(word);
    const common = {
      ...word[0],
      ...box,
      kind: 'ord',
      composite: 'word',
      baseline: word[0].baseline,
    };
    const fn = functionWords(letters);
    if (fn && style !== '\\mathit') {
      const limitsFn = fn.length === 1 && LIMIT_FUNCTIONS.has(fn[0]);
      out.push({
        ...common,
        tex: fn
          .map((f) => (KNOWN_FUNCTION_COMMANDS.has(f) ? `\\${f}` : `\\operatorname{${f}}`))
          .join(''),
        kind: 'op',
        limits: limitsFn,
      });
      return;
    }
    if (word.length === 1) {
      out.push({
        ...common,
        tex: style === '\\mathit' ? letters : `${style || '\\mathrm'}{${letters}}`,
      });
      return;
    }
    // Spaces where the gap between letters is a word space.
    let text = '';
    word.forEach((a, i) => {
      if (i > 0 && a.x - right(word[i - 1]) > 0.15 * B) text += ' ';
      text += a.tex;
    });
    const wrap =
      {
        '\\mathbf': '\\textbf',
        '\\mathit': '\\textit',
        '\\mathsf': '\\textsf',
        '\\mathtt': '\\texttt',
      }[style] ?? '\\text';
    out.push({ ...common, tex: `${wrap}{${text}}` });
  };
  for (const a of bases) {
    const prev = run[run.length - 1];
    const fits =
      a.kind === 'letter' &&
      (a.upright || (a.textFont && a.style === '\\mathit')) &&
      (!prev ||
        (a.style === prev.style &&
          a.x - right(prev) < 0.45 * B &&
          a.x - right(prev) > -0.3 * B &&
          Math.abs(a.baseline - prev.baseline) < 0.1 * B));
    if (fits) {
      run.push(a);
      continue;
    }
    flush();
    if (a.kind === 'letter' && (a.upright || (a.textFont && a.style === '\\mathit'))) run.push(a);
    else out.push(a);
  }
  flush();
  return out;
}

/** "sin" → ['sin'], "sincos" → ['sin', 'cos'], "sinx" → null. */
function functionWords(letters) {
  if (FUNCTIONS.has(letters)) return [letters];
  const best = new Array(letters.length + 1).fill(null);
  best[0] = [];
  for (let i = 0; i < letters.length; i++) {
    if (!best[i]) continue;
    for (const f of FUNCTIONS) {
      if (letters.startsWith(f, i) && !best[i + f.length]) best[i + f.length] = [...best[i], f];
    }
  }
  return best[letters.length];
}

function baseTex(b) {
  return wrapStyle(b);
}

function wrapStyle(a) {
  if (a.kind === 'prime') return "'";
  if (a.style && (a.kind === 'letter' || (a.kind === 'ord' && !a.composite))) {
    if (a.style === '\\mathit') return a.tex;
    return `${a.style}{${a.tex}}`;
  }
  return a.tex;
}

/** Sequences that are one symbol: three dots, \not before a relation,
 * \mapstochar before an arrow, a hook beside an arrow. */
function fixSequences(segments, baseline, B) {
  const out = [];
  for (let i = 0; i < segments.length; i++) {
    const s = segments[i];
    const next = segments[i + 1];
    const t = s.base?.tex;
    const isDot = (seg) =>
      seg &&
      seg.base &&
      seg.tex === seg.base.tex &&
      ['.', '\\cdot', '\\ldots', '\\cdots'].includes(seg.tex);
    if (isDot(s) && (s.tex === '.' || s.tex === '\\cdot')) {
      const run = [s];
      while (
        run.length < 3 &&
        isDot(segments[i + run.length]) &&
        segments[i + run.length].tex === s.tex &&
        segments[i + run.length].x - run[run.length - 1].right < 0.5 * B
      ) {
        run.push(segments[i + run.length]);
      }
      if (run.length === 3) {
        const raised = s.tex === '\\cdot' || s.base.baseline < baseline - 0.15 * B;
        out.push({ ...s, tex: raised ? '\\cdots' : '\\ldots', kind: 'ord', right: run[2].right });
        i += 2;
        continue;
      }
    }
    if (s.tex === '\\ldots' && s.base && s.base.baseline < baseline - 0.15 * B) {
      out.push({ ...s, tex: '\\cdots' });
      continue;
    }
    if (t === 'not' && next) {
      const negated = { '=': '\\neq', '\\in': '\\notin', '\\equiv': '\\not\\equiv' }[next.tex];
      out.push({ ...next, tex: negated ?? `\\not${next.tex}`, x: s.x, kind: 'rel' });
      i++;
      continue;
    }
    if (t === 'mapsto' && next && next.tex === '\\to') {
      out.push({ ...next, tex: '\\mapsto', x: s.x, kind: 'rel' });
      i++;
      continue;
    }
    if (t === 'lhook' && next && next.tex === '\\to') {
      out.push({ ...next, tex: '\\hookrightarrow', x: s.x, kind: 'rel' });
      i++;
      continue;
    }
    if (s.tex === '\\leftarrow' && next?.base?.tex === 'rhook') {
      out.push({ ...s, tex: '\\hookleftarrow', right: next.right, kind: 'rel' });
      i++;
      continue;
    }
    if (s.kind === 'piece') continue;
    out.push(s);
  }
  return out;
}

/** Delimiters from the extension font are sized: a pair becomes \left …
 * \right, a lone one \big, \Big, \bigg or \Bigg. */
function pairDelimiters(segments, flags) {
  const stack = [];
  const sized = (s) => s.base && s.base.tall && ['open', 'close', 'fence'].includes(s.base.kind);
  const pairs = new Map();
  segments.forEach((s, i) => {
    if (!sized(s)) return;
    const k = s.base.kind;
    if (
      k === 'open' ||
      (k === 'fence' && !stack.some((j) => segments[j].base.tex === s.base.tex))
    ) {
      stack.push(i);
      return;
    }
    const j = stack.pop();
    if (j === undefined) return;
    pairs.set(j, i);
    pairs.set(i, j);
  });
  return segments.map((s, i) => {
    if (!sized(s)) return s;
    const scripts = s.tex.slice(s.base.tex.length);
    if (pairs.has(i)) {
      const open = pairs.get(i) > i;
      return {
        ...s,
        tex: `${open ? '\\left' : '\\right'}${s.base.tex}${scripts}`,
        kind: open ? 'open' : 'close',
      };
    }
    const size = Math.min(4, s.base.delimSize || 1);
    const word = ['\\big', '\\Big', '\\bigg', '\\Bigg'][size - 1];
    const lr = s.base.kind === 'open' ? 'l' : s.base.kind === 'close' ? 'r' : '';
    if (s.base.delimSize >= 5) flags.ambiguous++;
    return { ...s, tex: `${word}${lr}${s.base.tex}${scripts}` };
  });
}

/** Segments → LaTeX. Relations and binary operators get a space either
 * side at the top level; inside scripts everything is packed. */
function segmentsToTex(segments, depth) {
  let out = '';
  let prev = null;
  for (const s of segments) {
    let piece = s.tex;
    if (!piece) continue;
    const spaced = depth === 0 && (s.kind === 'rel' || (s.kind === 'bin' && isBinary(prev)));
    if (spaced) piece = out ? ` ${piece} ` : `${piece} `;
    else if (depth === 0 && prev && prev.kind === 'punct') piece = ` ${piece}`;
    out = joinTex(out, piece);
    prev = s;
  }
  return out.replace(/\s+/g, ' ').trim();
}

function isBinary(prev) {
  return (
    Boolean(prev) &&
    !['bin', 'rel', 'open', 'punct'].includes(prev.kind) &&
    !/\\left\S*$/.test(prev.tex)
  );
}

/** Concatenate, with a space only where a command would otherwise run into
 * a following letter. */
function joinTex(a, b) {
  if (!a) return b;
  if (/\\[A-Za-z]+$/.test(a) && /^[A-Za-z0-9]/.test(b)) return `${a} ${b}`;
  // x^2 3 is not x^23, and \sum_i^n a is clearer than \sum_i^na.
  if (/[_^][A-Za-z0-9]$/.test(a) && /^[A-Za-z0-9]/.test(b)) return `${a} ${b}`;
  return a + b;
}

/** Braces for a script or limit, only where they are needed. */
function braced(tex) {
  if (/^([A-Za-z0-9]|\\[A-Za-z]+)$/.test(tex)) return tex;
  return `{${tex}}`;
}

function tidy(tex) {
  return tex.replace(/\s+/g, ' ').replace(/\{ /g, '{').replace(/ \}/g, '}').trim();
}

// ---------- archives ----------

/** gzip → bytes, with the platform's own inflater (workers have it). */
async function gunzip(bytes) {
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

const isGzip = (b) => b.length > 2 && b[0] === 0x1f && b[1] === 0x8b;
const isPdf = (b) =>
  b.length > 4 && b[0] === 0x25 && b[1] === 0x50 && b[2] === 0x44 && b[3] === 0x46;

function ascii(bytes, start, length) {
  let s = '';
  for (let i = start; i < start + length && i < bytes.length; i++) {
    if (bytes[i] === 0) break;
    s += String.fromCharCode(bytes[i]);
  }
  return s;
}

function tarNumber(bytes, start, length) {
  if (bytes[start] & 0x80) {
    // base-256, for sizes past 8 GB; nothing here is that big.
    let n = 0;
    for (let i = start + 1; i < start + length; i++) n = n * 256 + bytes[i];
    return n;
  }
  const s = ascii(bytes, start, length).trim();
  return s ? parseInt(s, 8) : 0;
}

function tarHeaderValid(h) {
  const stored = tarNumber(h, 148, 8);
  let sum = 0;
  for (let i = 0; i < 512; i++) sum += i >= 148 && i < 156 ? 32 : h[i];
  return sum === stored;
}

function isTar(bytes) {
  return bytes.length >= 512 && tarHeaderValid(bytes.subarray(0, 512));
}

/** A tar archive → its regular files, `{ name, data }`. Understands ustar
 * prefixes, GNU long names and pax path records. */
function untar(bytes) {
  const files = [];
  let off = 0;
  let longName = null;
  let paxPath = null;
  while (off + 512 <= bytes.length) {
    const h = bytes.subarray(off, off + 512);
    if (h.every((b) => b === 0)) break;
    if (!tarHeaderValid(h)) break;
    const size = tarNumber(h, 124, 12);
    const type = h[156] === 0 ? '0' : String.fromCharCode(h[156]);
    const ustar = ascii(h, 257, 5) === 'ustar';
    const prefix = ustar ? ascii(h, 345, 155) : '';
    const name = ascii(h, 0, 100);
    const data = bytes.subarray(off + 512, off + 512 + size);
    off += 512 + Math.ceil(size / 512) * 512;
    if (type === 'L') {
      longName = ascii(data, 0, data.length);
      continue;
    }
    if (type === 'x') {
      const m = /\d+ path=([^\n]*)\n/.exec(new TextDecoder().decode(data));
      paxPath = m ? m[1] : null;
      continue;
    }
    if (type === 'g') continue;
    const full = paxPath ?? longName ?? (prefix ? `${prefix}/${name}` : name);
    paxPath = null;
    longName = null;
    if (type === '0' || type === '7') files.push({ name: full.replace(/^\.\//, ''), data });
  }
  return files;
}

/** An arXiv e-print → its TeX files `{ name, text }`, or null when it is not
 * TeX (a PDF-only submission, or something unrecognisable). */
async function readEprint(bytes) {
  let data = bytes;
  if (isPdf(data)) return null;
  if (isGzip(data)) data = await gunzip(data);
  if (isPdf(data)) return null;
  const decode = (b) => new TextDecoder('utf-8').decode(b);
  if (isTar(data)) {
    const files = untar(data)
      .filter((f) => /\.(tex|ltx|sty)$/i.test(f.name))
      .map((f) => ({ name: f.name, text: decode(f.data) }));
    return files.length > 0 ? files : null;
  }
  // A single-file submission is just the gzipped .tex.
  const text = decode(data);
  return /\\(documentclass|begin\{document\}|documentstyle)/.test(text)
    ? [{ name: 'main.tex', text }]
    : null;
}

// ---------- TeX source ----------

function stripComments(text) {
  return text.replace(/(^|[^\\])((?:\\\\)*)%[^\n]*/g, '$1$2');
}

/** The document as TeX reads it: the main file with every \input and
 * \include replaced by the file it names. */
function assemble(files) {
  const byName = new Map(files.map((f) => [f.name.replace(/\\/g, '/'), stripComments(f.text)]));
  const texFiles = files.filter((f) => !/\.sty$/i.test(f.name));
  const mains = texFiles.filter(
    (f) => /\\documentclass|\\documentstyle/.test(f.text) && /\\begin\s*\{document\}/.test(f.text)
  );
  const main =
    mains.find((f) => /(^|\/)(main|ms|paper|article)\.tex$/i.test(f.name)) ??
    mains.sort((a, b) => b.text.length - a.text.length)[0];
  const find = (ref, from) => {
    const dir = from.includes('/') ? from.slice(0, from.lastIndexOf('/') + 1) : '';
    const clean = ref.trim().replace(/^\.\//, '');
    for (const candidate of [clean, `${clean}.tex`, dir + clean, `${dir}${clean}.tex`]) {
      if (byName.has(candidate)) return candidate;
    }
    return null;
  };
  const seen = new Set();
  const expand = (name, depth) => {
    if (depth > 12 || seen.has(name)) return '';
    seen.add(name);
    return byName
      .get(name)
      .replace(
        /\\(?:input|include|subfile|import\s*\{[^}]*\})\s*(?:\{([^}]+)\}|([^\s{}\\]+))/g,
        (whole, a, b) => {
          const found = find(a ?? b, name);
          return found ? `\n${expand(found, depth + 1)}\n` : whole;
        }
      );
  };
  const body = main
    ? expand(main.name, 0)
    : texFiles.map((f) => byName.get(f.name.replace(/\\/g, '/'))).join('\n');
  const preambles = files
    .filter((f) => /\.sty$/i.test(f.name))
    .map((f) => byName.get(f.name.replace(/\\/g, '/')));
  return { body, macroText: `${preambles.join('\n')}\n${body}` };
}

/** Argument-free macros: \newcommand{\R}{\mathbb{R}}, \def\eps{\varepsilon},
 * \DeclareMathOperator{\Tr}{Tr}. A macro with arguments is left alone. */
function collectMacros(text) {
  const macros = new Map();
  const re =
    /\\(newcommand|renewcommand|providecommand|DeclareRobustCommand|def|DeclareMathOperator)(\*?)\s*(?:\{\s*\\([A-Za-z]+)\s*\}|\\([A-Za-z]+))/g;
  let m;
  while ((m = re.exec(text))) {
    const name = m[3] ?? m[4];
    let i = re.lastIndex;
    while (/\s/.test(text[i] ?? '')) i++;
    if (text[i] === '[' || (m[1] === 'def' && text[i] !== '{')) continue;
    const arg = bracedAt(text, i);
    if (!arg) continue;
    if (arg.inner.includes('#')) continue;
    const body =
      m[1] === 'DeclareMathOperator' ? `\\operatorname${m[2]}{${arg.inner}}` : arg.inner.trim();
    if (m[1] === 'providecommand' && macros.has(name)) continue;
    if (!macros.has(name) || m[1] !== 'newcommand') macros.set(name, body);
  }
  return macros;
}

/** A `{…}` group starting at `i`: its contents and where it ends. */
function bracedAt(text, i) {
  if (text[i] !== '{') return null;
  let depth = 0;
  for (let j = i; j < text.length; j++) {
    const c = text[j];
    if (c === '\\') {
      j++;
      continue;
    }
    if (c === '{') depth++;
    else if (c === '}' && --depth === 0) return { inner: text.slice(i + 1, j), end: j + 1 };
  }
  return null;
}

function expandMacros(tex, macros) {
  if (!macros || macros.size === 0) return tex;
  let out = tex;
  for (let pass = 0; pass < 6; pass++) {
    let changed = false;
    out = out.replace(/\\([A-Za-z]+)(?![A-Za-z])/g, (whole, name) => {
      if (!macros.has(name)) return whole;
      changed = true;
      return macros.get(name);
    });
    if (!changed) break;
  }
  return out;
}

const NUMBERED = new Set([
  'equation',
  'align',
  'gather',
  'multline',
  'eqnarray',
  'flalign',
  'alignat',
]);
const MATH_ENVS =
  'equation|align|gather|multline|eqnarray|flalign|alignat|displaymath|math|dmath|subequations';

/** Every piece of math in a document body, in order, with the numbers the
 * numbered ones get. */
function extractMath(source, macros) {
  const begin = source.search(/\\begin\s*\{document\}/);
  const end = source.search(/\\end\s*\{document\}/);
  const text = source.slice(begin >= 0 ? begin : 0, end > begin ? end : source.length);
  const perSection = /\\numberwithin\s*\{equation\}\s*\{section\}/.test(source);
  let section = 0;
  let appendix = false;
  let counter = 0;
  const sectionLabel = () => (appendix ? String.fromCharCode(64 + section) : String(section));
  const next = () => {
    counter++;
    return perSection ? `${sectionLabel()}.${counter}` : String(counter);
  };
  const snippets = [];
  const add = (raw, env, display, star) => {
    let inner = raw;
    const numbered = NUMBERED.has(env) && !star;
    const numbers = [];
    const rows = /^(align|eqnarray|flalign|alignat|gather|multline)$/.test(env)
      ? splitRows(inner)
      : [inner];
    if (numbered) {
      const rowsToNumber = env === 'multline' ? [inner] : rows;
      for (const row of rowsToNumber) {
        const tag = /\\tag\*?\s*\{([^}]*)\}/.exec(row);
        if (tag) numbers.push(tag[1].trim());
        else if (!/\\(nonumber|notag)\b/.test(row)) numbers.push(next());
      }
    } else {
      const tag = /\\tag\*?\s*\{([^}]*)\}/.exec(inner);
      if (tag) numbers.push(tag[1].trim());
    }
    inner = inner
      .replace(/\\label\s*\{[^}]*\}/g, '')
      .replace(/\\(nonumber|notag)\b/g, '')
      .replace(/\\tag\*?\s*\{[^}]*\}/g, '');
    inner = expandMacros(inner, macros);
    let tex;
    if (env === 'eqnarray') {
      tex = `\\begin{aligned} ${splitRows(inner)
        .map((r) => r.replace(/&([^&]*)&/, '&$1'))
        .join(' \\\\ ')} \\end{aligned}`;
    } else if (env === 'align' || env === 'flalign') {
      tex =
        rows.length > 1 || /&/.test(inner)
          ? `\\begin{aligned} ${inner.trim()} \\end{aligned}`
          : inner;
    } else if (env === 'alignat') {
      const n = /^\s*\{(\d+)\}/.exec(inner);
      const body = n ? inner.slice(n[0].length) : inner;
      tex = `\\begin{alignedat}{${n ? n[1] : 1}} ${body.trim()} \\end{alignedat}`;
    } else if (env === 'gather' || env === 'multline') {
      tex = rows.length > 1 ? `\\begin{gathered} ${inner.trim()} \\end{gathered}` : inner;
    } else {
      tex = inner
        .replace(/\\begin\{split\}/g, '\\begin{aligned}')
        .replace(/\\end\{split\}/g, '\\end{aligned}');
    }
    tex = tex.replace(/\s+/g, ' ').trim();
    if (tex.length === 0 || tex.length > 6000) return;
    snippets.push({ tex, env, display, numbers });
  };

  const envStart = new RegExp(`^\\\\begin\\s*\\{(${MATH_ENVS})(\\*?)\\}`);
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '\\') {
      const rest = text.slice(i, i + 40);
      if (/^\\section\s*[[{]/.test(rest)) {
        section++;
        if (perSection) counter = 0;
      } else if (/^\\appendix\b/.test(rest)) {
        appendix = true;
        section = 0;
      } else if (rest.startsWith('\\(') || rest.startsWith('\\[')) {
        const close = rest.startsWith('\\(') ? '\\)' : '\\]';
        const j = text.indexOf(close, i + 2);
        if (j < 0) break;
        add(
          text.slice(i + 2, j),
          rest.startsWith('\\(') ? 'inline' : 'display',
          rest.startsWith('\\['),
          false
        );
        i = j + 1;
        continue;
      } else {
        const m = envStart.exec(rest);
        if (m) {
          const env = m[1];
          const star = m[2] === '*';
          const closeRe = new RegExp(`\\\\end\\s*\\{${env}${star ? '\\*' : ''}\\}`, 'g');
          closeRe.lastIndex = i + m[0].length;
          const close = closeRe.exec(text);
          if (!close) break;
          const inner = text.slice(i + m[0].length, close.index);
          if (env === 'subequations') {
            // Its contents are read as they come; only the wrapper is skipped.
            i += m[0].length - 1;
            continue;
          }
          add(
            inner,
            env === 'displaymath' ? 'display' : env === 'math' ? 'inline' : env,
            env !== 'math',
            star
          );
          i = closeRe.lastIndex - 1;
          continue;
        }
      }
      i++;
      continue;
    }
    if (c === '$') {
      const display = text[i + 1] === '$';
      const start = i + (display ? 2 : 1);
      let j = start;
      while (j < text.length) {
        if (text[j] === '\\') {
          j += 2;
          continue;
        }
        if (text[j] === '$') break;
        if (!display && text[j] === '\n' && text[j + 1] === '\n') break;
        j++;
      }
      if (j >= text.length || text[j] !== '$') {
        i = j;
        continue;
      }
      add(text.slice(start, j), display ? 'display' : 'inline', display, false);
      i = j + (display ? 1 : 0);
    }
  }
  return snippets;
}

/** Rows of an align-like body, split on top-level `\\`. */
function splitRows(body) {
  const rows = [];
  let depth = 0;
  let last = 0;
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c === '\\' && body[i + 1] === '\\' && depth === 0) {
      rows.push(body.slice(last, i));
      i++;
      while (body[i + 1] === '*') i++;
      if (body[i + 1] === '[') {
        const close = body.indexOf(']', i);
        if (close > 0) i = close;
      }
      last = i + 1;
      continue;
    }
    if (c === '\\') {
      i++;
      continue;
    }
    if (c === '{') depth++;
    else if (c === '}') depth--;
  }
  rows.push(body.slice(last));
  return rows.filter((r) => r.trim().length > 0);
}

/** TeX files → the document's math, ready to match. */
function parseSource(files) {
  const { body, macroText } = assemble(files);
  const macros = collectMacros(macroText);
  return extractMath(body, macros);
}

// ---------- matching a selection against source ----------
//
// The selection and every candidate are reduced to the same thing: the
// sequence of symbols they contain, in reading order, without structure.
// `\frac{a+b}{c}`, the glyphs a + b — c, and the text "a+b c" all become
// a + b c. Greek and macros map to their Unicode characters, styles to the
// styled letter (ℝ), and function names to their letters.

const LATEX_TO_CHAR = (() => {
  const map = new Map();
  for (const [ch, tex] of UNICODE) {
    if (/^\\[A-Za-z]+$/.test(tex) && !map.has(tex)) map.set(tex, ch);
  }
  const aliases = {
    '\\le': '≤',
    '\\ge': '≥',
    '\\ne': '≠',
    '\\to': '→',
    '\\rightarrow': '→',
    '\\gets': '←',
    '\\lbrace': '{',
    '\\rbrace': '}',
    '\\lvert': '|',
    '\\rvert': '|',
    '\\vert': '|',
    '\\mid': '|',
    '\\lVert': '‖',
    '\\rVert': '‖',
    '\\Vert': '‖',
    '\\land': '∧',
    '\\lor': '∨',
    '\\lnot': '¬',
    '\\iff': '⟺',
    '\\implies': '⟹',
    '\\impliedby': '⟸',
    '\\dots': '…',
    '\\dotsc': '…',
    '\\dotsb': '⋯',
    '\\dotsm': '⋯',
    '\\cdotp': '·',
    '\\colon': ':',
    '\\centerdot': '·',
    '\\varnothing': '∅',
    '\\smallsetminus': '∖',
    '\\hslash': 'ℏ',
    '\\bot': '⊥',
    '\\surd': '√',
    '\\prime': '′',
    '\\lbrack': '[',
    '\\rbrack': ']',
    '\\epsilon': 'ϵ',
    '\\varepsilon': 'ε',
    '\\phi': 'ϕ',
    '\\varphi': 'φ',
    '\\lhd': '⊲',
    '\\rhd': '⊳',
    '\\neg': '¬',
    '\\owns': '∋',
    '\\leqslant': '⩽',
    '\\geqslant': '⩾',
    '\\bigstar': '★',
    '\\backslash': '\\',
  };
  for (const [k, v] of Object.entries(aliases)) map.set(k, v);
  for (const g of 'Gamma Delta Theta Lambda Xi Pi Sigma Upsilon Phi Psi Omega'.split(' ')) {
    map.set(`\\var${g}`, map.get(`\\${g}`));
  }
  return map;
})();

const NORMAL = {
  '−': '-',
  '∗': '*',
  '⋅': '·',
  '∙': '·',
  '∣': '|',
  '〈': '⟨',
  '〉': '⟩',
  ℎ: 'h',
  '∶': ':',
  "'": '′',
  '’': '′',
  '–': '-',
  '‖': '‖',
  '∕': '/',
  '…': '…',
};
const SKIP_COMMANDS = new Set(
  (
    'frac dfrac tfrac cfrac sqrt left right middle big Big bigg Bigg bigl bigr Bigl Bigr biggl ' +
    'biggr Biggl Biggr limits nolimits displaystyle textstyle scriptstyle scriptscriptstyle ' +
    'quad qquad hat bar tilde vec dot ddot widehat widetilde overline underline check breve ' +
    'acute grave mathring overbrace underbrace overset underset stackrel nonumber notag ' +
    'notag nobreak allowbreak hfill hspace vspace phantom hphantom vphantom kern mkern mskip ' +
    'not binom tbinom dbinom genfrac boxed displaylines cr substack mathstrut strut relax ' +
    'lefteqn intertext'
  ).split(' ')
);
const ARG_SKIP = new Set([
  'label',
  'tag',
  'begin',
  'end',
  'hspace',
  'vspace',
  'kern',
  'eqref',
  'ref',
  'cite',
]);
const STYLE_COMMANDS = {
  mathbb: '\\mathbb',
  mathbf: '\\mathbf',
  boldsymbol: '\\mathbf',
  bm: '\\mathbf',
  mathcal: '\\mathcal',
  mathscr: '\\mathcal',
  mathfrak: '\\mathfrak',
  mathsf: '',
  mathtt: '',
  mathrm: '',
  mathit: '',
  text: '',
  textrm: '',
  textbf: '',
  textit: '',
  textsf: '',
  texttt: '',
  operatorname: '',
  mbox: '',
  hbox: '',
  mathnormal: '',
  textnormal: '',
  pmb: '\\mathbf',
  Bbb: '\\mathbb',
  bold: '\\mathbf',
};

function normChar(ch) {
  const n = NORMAL[ch];
  if (n) return n;
  const alpha = mathAlpha(ch.codePointAt(0));
  if (alpha) {
    if (!alpha.style || alpha.style === '\\mathsf' || alpha.style === '\\mathtt') return alpha.base;
    if (alpha.style === '\\boldsymbol')
      return /^[A-Za-z0-9]$/.test(alpha.base) ? styledChar('\\mathbf', alpha.base) : alpha.base;
    return styledChar(alpha.style, alpha.base);
  }
  return ch;
}

/** LaTeX → `[{ t, start, end }]`: its symbols, with where each came from. */
function texTokens(tex) {
  const out = [];
  const walk = (s, offset, style, span) => {
    for (let i = 0; i < s.length;) {
      const c = s[i];
      const at = span ?? [offset + i, offset + i + 1];
      if (/\s/.test(c) || '{}^_&~'.includes(c)) {
        i++;
        continue;
      }
      if (c === '\\') {
        const m = /^\\([A-Za-z]+)\*?/.exec(s.slice(i));
        if (!m) {
          const nx = s[i + 1];
          if (nx === '{' || nx === '}' || nx === '|')
            out.push({
              t: nx === '|' ? '‖' : nx,
              start: at[0],
              end: span ? at[1] : offset + i + 2,
            });
          i += 2;
          continue;
        }
        const name = m[1];
        let j = i + m[0].length;
        if (ARG_SKIP.has(name)) {
          while (s[j] === ' ') j++;
          const arg = bracedAt(s, j);
          i = arg ? arg.end : j;
          if (name === 'begin' && /^\{(alignedat|alignat|array)/.test(s.slice(j))) {
            const n = bracedAt(s, i);
            if (n) i = n.end;
          }
          continue;
        }
        if (name in STYLE_COMMANDS) {
          while (s[j] === ' ') j++;
          const arg = bracedAt(s, j);
          const st = STYLE_COMMANDS[name];
          if (arg) {
            walk(arg.inner, offset + j + 1, st || style, span ?? [offset + i, offset + arg.end]);
            i = arg.end;
          } else {
            i = j;
          }
          continue;
        }
        if (SKIP_COMMANDS.has(name)) {
          i = j;
          continue;
        }
        const end = span ? at[1] : offset + j;
        if (FUNCTIONS.has(name)) {
          for (const ch of name) out.push({ t: ch, start: at[0], end });
        } else {
          const ch = LATEX_TO_CHAR.get(`\\${name}`);
          out.push({ t: ch ? normChar(ch) : `\\${name}`, start: at[0], end });
        }
        i = j;
        continue;
      }
      const cp = s.codePointAt(i);
      const ch = String.fromCodePoint(cp);
      const t = style && /^[A-Za-z0-9]$/.test(ch) ? styledChar(style, ch) : normChar(ch);
      out.push({ t, start: at[0], end: span ? at[1] : offset + i + ch.length });
      i += ch.length;
    }
  };
  walk(tex, 0, '', null);
  return out;
}

/** Plain text (a selection's own text) → symbols. */
function textTokens(text) {
  const out = [];
  for (const ch of Array.from(text ?? '')) {
    if (/\s/.test(ch)) continue;
    const mapped = UNICODE.get(ch);
    if (mapped === '' || (mapped && mapped.startsWith('accent:'))) continue;
    const cp = ch.codePointAt(0);
    if (cp >= 0x300 && cp < 0x370) continue;
    out.push(normChar(ch));
  }
  return out;
}

/** Longest common subsequence length, two rows of memory. */
function lcs(a, b) {
  let prev = new Array(b.length + 1).fill(0);
  for (let i = 1; i <= a.length; i++) {
    const cur = new Array(b.length + 1).fill(0);
    for (let j = 1; j <= b.length; j++) {
      cur[j] = a[i - 1] === b[j - 1] ? prev[j - 1] + 1 : Math.max(prev[j], cur[j - 1]);
    }
    prev = cur;
  }
  return prev[b.length];
}

function normalizeNumber(n) {
  return String(n ?? '')
    .replace(/[()[\]\s]/g, '')
    .replace(/^eq\.?/i, '');
}

/** The snippet that best matches the selection. `queries` are symbol
 * sequences (from the reconstructed LaTeX, and from the selected text);
 * `number` is the printed equation number of an equation action. Returns
 * `{ snippet, tex, exact }` or null. */
function matchSnippets(snippets, queries, { number = null, display = false } = {}) {
  const usable = queries.filter((q) => q.length >= 3);
  const wanted = number ? normalizeNumber(number) : '';
  let best = null;
  for (const snippet of snippets) {
    snippet.tokens ??= texTokens(snippet.tex);
    const S = snippet.tokens.map((t) => t.t);
    if (S.length === 0) continue;
    let score = 0;
    let exact = false;
    let span = null;
    for (const Q of usable) {
      const at = indexOfSeq(S, Q);
      if (at >= 0) {
        const s = 2 + Q.length / S.length;
        if (s > score) {
          score = s;
          exact = true;
          span = [at, at + Q.length];
        }
        continue;
      }
      const bag = new Map();
      for (const t of S) bag.set(t, (bag.get(t) ?? 0) + 1);
      let present = 0;
      for (const t of Q) {
        const n = bag.get(t) ?? 0;
        if (n > 0) {
          present++;
          bag.set(t, n - 1);
        }
      }
      if (present < 0.6 * Q.length) continue;
      const L = lcs(Q, S);
      const coverage = L / Q.length;
      const precision = L / S.length;
      if (coverage < 0.75 || precision < 0.25) continue;
      const s = coverage * 0.85 + precision * 0.15;
      if (s > score) {
        score = s;
        exact = false;
        span = null;
      }
    }
    const numbered = wanted && snippet.numbers.some((n) => normalizeNumber(n) === wanted);
    if (score === 0 && !(numbered && usable.length === 0)) continue;
    if (numbered) score += 1.5;
    else if (wanted && snippet.numbers.length > 0) score -= 0.3;
    if (display && snippet.display) score += 0.1;
    if (
      !best ||
      score > best.score + 1e-9 ||
      (Math.abs(score - best.score) < 1e-9 && snippet.tex.length < best.snippet.tex.length)
    ) {
      best = { snippet, score, exact, span, numbered };
    }
  }
  if (!best) return null;
  let tex = best.snippet.tex;
  // A selection of part of an equation gets that part, when it can be cut
  // out cleanly.
  if (
    best.exact &&
    best.span &&
    !display &&
    best.span[1] - best.span[0] < best.snippet.tokens.length
  ) {
    const part = cutSpan(best.snippet, best.span);
    if (part) tex = part;
  }
  return {
    snippet: best.snippet,
    tex,
    exact: best.exact || Boolean(best.numbered && usable.length === 0),
  };
}

function indexOfSeq(S, Q) {
  outer: for (let i = 0; i + Q.length <= S.length; i++) {
    for (let j = 0; j < Q.length; j++) if (S[i + j] !== Q[j]) continue outer;
    return i;
  }
  return -1;
}

/** The source text of tokens `[from, to)` of a snippet, when that is a
 * balanced piece of LaTeX; otherwise null. */
function cutSpan(snippet, [from, to]) {
  const tokens = snippet.tokens;
  const start = tokens[from].start;
  let end = tokens[to - 1].end;
  const tex = snippet.tex;
  // Close any group the last symbol is inside (x_{ij → x_{ij}).
  let depth = 0;
  for (let i = start; i < end; i++) {
    if (tex[i] === '\\') {
      i++;
      continue;
    }
    if (tex[i] === '{') depth++;
    else if (tex[i] === '}' && --depth < 0) return null;
  }
  while (depth > 0 && end < tex.length) {
    if (tex[end] === '}') depth--;
    else if (tex[end] === '{') depth++;
    end++;
  }
  const part = tex.slice(start, end).trim();
  if (depth !== 0 || /\\(begin|end)\b|&|\\\\/.test(part)) return null;
  return part;
}

// ---------- XML ----------
//
// Workers have no DOMParser, and JATS formulas are small, so a small reader:
// elements, attributes, text, CDATA and entities. Namespace prefixes are
// dropped (mml:mi is mi).

const ENTITIES = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
  InvisibleTimes: '⁢',
  ApplyFunction: '⁡',
  af: '⁡',
  it: '⁢',
  InvisibleComma: '⁣',
  ic: '⁣',
  PlusMinus: '±',
  pm: '±',
  times: '×',
  minus: '−',
  le: '≤',
  leq: '≤',
  ge: '≥',
  geq: '≥',
  ne: '≠',
  infin: '∞',
  sum: '∑',
  prod: '∏',
  int: '∫',
  part: '∂',
  nabla: '∇',
  isin: '∈',
  notin: '∉',
  sub: '⊂',
  sube: '⊆',
  sup: '⊃',
  supe: '⊇',
  cup: '∪',
  cap: '∩',
  rarr: '→',
  larr: '←',
  rArr: '⇒',
  hArr: '⇔',
  harr: '↔',
  forall: '∀',
  exist: '∃',
  empty: '∅',
  prime: '′',
  Prime: '″',
  middot: '·',
  sdot: '⋅',
  sim: '∼',
  asymp: '≈',
  approx: '≈',
  equiv: '≡',
  prop: '∝',
  lang: '⟨',
  rang: '⟩',
  langle: '⟨',
  rangle: '⟩',
  lceil: '⌈',
  rceil: '⌉',
  lfloor: '⌊',
  rfloor: '⌋',
  hellip: '…',
  ctdot: '⋯',
  deg: '°',
  micro: 'µ',
  thinsp: ' ',
  ThinSpace: ' ',
  DoubleStruckCapitalR: 'ℝ',
  Ropf: 'ℝ',
  Nopf: 'ℕ',
  Zopf: 'ℤ',
  Qopf: 'ℚ',
  Copf: 'ℂ',
  RightArrow: '→',
  LeftArrow: '←',
  OverBar: '¯',
  UnderBar: '_',
  Hat: '^',
  tilde: '˜',
  dot: '˙',
  die: '¨',
  DoubleDot: '¨',
  vec: '⃗',
  VerticalBar: '|',
  verbar: '|',
  Vert: '‖',
  lbrace: '{',
  rbrace: '}',
  lpar: '(',
  rpar: ')',
  lsqb: '[',
  rsqb: ']',
  ell: 'ℓ',
  hbar: 'ℏ',
  planckh: 'ℎ',
  compfn: '∘',
  sime: '≃',
  cong: '≅',
  ll: '≪',
  gg: '≫',
  mid: '∣',
  parallel: '∥',
  perp: '⊥',
  not: '¬',
  and: '∧',
  or: '∨',
  oplus: '⊕',
  otimes: '⊗',
  setminus: '∖',
  ssetmn: '∖',
  mapsto: '↦',
  to: '→',
};
for (const [
  i,
  g,
] of 'alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigmaf sigma tau upsilon phi chi psi omega'
  .split(' ')
  .entries()) {
  ENTITIES[g] = String.fromCodePoint(0x3b1 + i);
  if (g !== 'sigmaf') ENTITIES[g[0].toUpperCase() + g.slice(1)] = String.fromCodePoint(0x391 + i);
}
ENTITIES.epsiv = 'ϵ';
ENTITIES.varepsilon = 'ε';
ENTITIES.phiv = 'ϕ';
ENTITIES.thetav = 'ϑ';

function decodeEntities(s) {
  return s.replace(/&(#x[0-9a-fA-F]+|#\d+|[A-Za-z][A-Za-z0-9]*);/g, (whole, e) => {
    if (e[0] === '#') {
      const cp = e[1] === 'x' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return Number.isFinite(cp) && cp > 0 && cp < 0x110000 ? String.fromCodePoint(cp) : whole;
    }
    return ENTITIES[e] ?? whole;
  });
}

function parseXml(xml) {
  const root = { name: '#root', attrs: {}, children: [] };
  const stack = [root];
  const re =
    /<!\[CDATA\[([\s\S]*?)\]\]>|<!--[\s\S]*?-->|<\?[\s\S]*?\?>|<!DOCTYPE[^>]*>|<\/\s*([\w:.-]+)\s*>|<([\w:.-]+)((?:\s+[\w:.-]+\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*(\/?)>|([^<]+)|</g;
  let m;
  while ((m = re.exec(xml))) {
    const top = stack[stack.length - 1];
    if (m[1] !== undefined) top.children.push({ text: m[1] });
    else if (m[2] !== undefined) {
      const name = local(m[2]);
      for (let i = stack.length - 1; i > 0; i--) {
        if (stack[i].name === name) {
          stack.length = i;
          break;
        }
      }
    } else if (m[3] !== undefined) {
      const attrs = {};
      for (const a of m[4].matchAll(/([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) {
        attrs[local(a[1])] = decodeEntities(a[2] ?? a[3] ?? '');
      }
      const node = { name: local(m[3]), attrs, children: [] };
      top.children.push(node);
      if (!m[5]) stack.push(node);
    } else if (m[6] !== undefined) {
      top.children.push({ text: decodeEntities(m[6]) });
    }
  }
  return root;
}

function local(name) {
  const i = name.indexOf(':');
  return i >= 0 ? name.slice(i + 1) : name;
}

function findElement(node, name) {
  if (node.name === name) return node;
  for (const c of node.children ?? []) {
    if (!c.name) continue;
    const f = findElement(c, name);
    if (f) return f;
  }
  return null;
}

function textOf(node) {
  if (node.text !== undefined) return node.text;
  return (node.children ?? []).map(textOf).join('');
}

// ---------- MathML → LaTeX ----------

const VARIANTS = {
  normal: '\\mathrm',
  bold: '\\mathbf',
  'double-struck': '\\mathbb',
  script: '\\mathcal',
  'bold-script': '\\mathcal',
  fraktur: '\\mathfrak',
  'bold-fraktur': '\\mathfrak',
  'bold-italic': '\\boldsymbol',
  'sans-serif': '\\mathsf',
  'bold-sans-serif': '\\mathsf',
  'sans-serif-italic': '\\mathsf',
  monospace: '\\mathtt',
  italic: '',
};
const OVER_ACCENTS = {
  '^': '\\hat',
  ˆ: '\\hat',
  '̂': '\\hat',
  '~': '\\tilde',
  '˜': '\\tilde',
  '̃': '\\tilde',
  '¯': '\\bar',
  '‾': '\\bar',
  '̄': '\\bar',
  '̅': '\\bar',
  '―': '\\bar',
  _: '\\bar',
  '˙': '\\dot',
  '.': '\\dot',
  '̇': '\\dot',
  '¨': '\\ddot',
  '..': '\\ddot',
  '̈': '\\ddot',
  '→': '\\vec',
  '⃗': '\\vec',
  ˇ: '\\check',
  '̌': '\\check',
  '˘': '\\breve',
  '⏞': '\\overbrace',
  '︷': '\\overbrace',
  '←': '\\overleftarrow',
  '↔': '\\overleftrightarrow',
};
const UNDER_ACCENTS = {
  _: '\\underline',
  '̲': '\\underline',
  '¯': '\\underline',
  '‾': '\\underline',
  '⏟': '\\underbrace',
  '︸': '\\underbrace',
};
const WIDE = {
  '\\hat': '\\widehat',
  '\\tilde': '\\widetilde',
  '\\bar': '\\overline',
  '\\vec': '\\overrightarrow',
};
const MATRIX_ENVS = {
  '(': 'pmatrix',
  '[': 'bmatrix',
  '\\{': 'Bmatrix',
  '|': 'vmatrix',
  '\\|': 'Vmatrix',
};

/** Presentation MathML (a string, or a parsed node) → LaTeX. */
function mathmlToLatex(input) {
  const tree = typeof input === 'string' ? parseXml(input) : input;
  const math = findElement(tree, 'math') ?? tree;
  return tidy(mml(math, 0).tex);
}

const elements = (n) => (n.children ?? []).filter((c) => c.name);

/** A node → `{ tex, kind }`: kind is the class of an operator (rel, bin,
 * open, close, bigop, …) so a row can space and pair them. */
function mml(n, depth) {
  const kids = elements(n);
  const t = (tex, kind = 'ord') => ({ tex, kind });
  switch (n.name) {
    case 'semantics': {
      const tex = kids.find((k) => k.name === 'annotation' && /tex/i.test(k.attrs.encoding ?? ''));
      if (tex) return t(textOf(tex).trim());
      return kids[0] ? mml(kids[0], depth) : t('');
    }
    case 'annotation':
    case 'annotation-xml':
    case 'mphantom':
    case 'none':
    case 'mprescripts':
      return t('');
    case 'mi':
      return miTex(textOf(n).trim(), n.attrs.mathvariant);
    case 'mn':
      return t(styled(textOf(n).trim(), n.attrs.mathvariant, false));
    case 'mo':
      return moTex(textOf(n).trim());
    case 'mtext': {
      const s = textOf(n);
      if (!s.trim()) return t('', 'space');
      return t(`\\text{${s.replace(/([{}%$&#_])/g, '\\$1')}}`);
    }
    case 'ms':
      return t(`\\text{"${textOf(n)}"}`);
    case 'mspace':
      return t('', 'space');
    case 'msub':
    case 'msup':
    case 'msubsup': {
      const base = mml(kids[0] ?? { name: 'mrow', children: [] }, depth);
      const sub =
        n.name !== 'msup' ? mml(kids[1] ?? { name: 'mrow', children: [] }, depth + 1).tex : '';
      const supNode = n.name === 'msup' ? kids[1] : n.name === 'msubsup' ? kids[2] : null;
      return t(
        attachScripts(base.tex, sub, supNode ? supTex(supNode, depth) : ''),
        base.kind === 'bigop' ? 'bigop' : base.kind === 'op' ? 'op' : 'ord'
      );
    }
    case 'mfrac': {
      const a = mml(kids[0] ?? { name: 'mrow', children: [] }, depth).tex;
      const b = mml(kids[1] ?? { name: 'mrow', children: [] }, depth).tex;
      const lt = n.attrs.linethickness;
      if (lt !== undefined && /^0(\.0*)?([a-z]+)?$/.test(lt.trim()))
        return t(`\\genfrac{}{}{0pt}{}{${a}}{${b}}`);
      if (n.attrs.bevelled === 'true') return t(`${a}/${b}`);
      return t(`\\frac{${a}}{${b}}`);
    }
    case 'msqrt':
      return t(`\\sqrt{${row(kids, depth)}}`);
    case 'mroot':
      return t(`\\sqrt[${mml(kids[1], depth + 1).tex}]{${mml(kids[0], depth).tex}}`);
    case 'mover':
    case 'munder':
    case 'munderover':
      return underOver(n, kids, depth);
    case 'mtable':
      return t(table(n, null));
    case 'mfenced': {
      const open = n.attrs.open ?? '(';
      const close = n.attrs.close ?? ')';
      const seps = (n.attrs.separators ?? ',').replace(/\s/g, '');
      if (kids.length === 1 && kids[0].name === 'mtable') {
        return t(fencedTable(kids[0], fenceTex(open), fenceTex(close)));
      }
      const parts = kids.map((k, i) =>
        i < kids.length - 1 && seps
          ? `${mml(k, depth).tex}${seps[Math.min(i, seps.length - 1)]} `
          : mml(k, depth).tex
      );
      return t(`${fenceTex(open)}${parts.join('')}${fenceTex(close)}`);
    }
    case 'menclose': {
      const notation = n.attrs.notation ?? '';
      if (/radical/.test(notation)) return t(`\\sqrt{${row(kids, depth)}}`);
      if (/box|roundedbox/.test(notation)) return t(`\\boxed{${row(kids, depth)}}`);
      if (/updiagonalstrike|downdiagonalstrike/.test(notation))
        return t(`\\cancel{${row(kids, depth)}}`);
      return t(row(kids, depth));
    }
    case 'mmultiscripts': {
      const base = mml(kids[0], depth).tex;
      const split = kids.findIndex((k) => k.name === 'mprescripts');
      const post = kids.slice(1, split >= 0 ? split : undefined);
      const preList = split >= 0 ? kids.slice(split + 1) : [];
      const pairs = (list) => {
        let s = '';
        for (let i = 0; i + 1 < list.length; i += 2) {
          const sub = mml(list[i], depth + 1).tex;
          const sup = mml(list[i + 1], depth + 1).tex;
          s += attachScripts('', sub, sup);
        }
        return s;
      };
      const pre = pairs(preList);
      return t(`${pre ? `{}${pre}` : ''}${attachScripts(base, '', '')}${pairs(post)}`);
    }
    default:
      return t(row(kids, depth), kids.length === 1 ? mml(kids[0], depth).kind : 'ord');
  }
}

function styled(text, variant, letter) {
  const style = VARIANTS[variant ?? ''];
  if (!style || !text) return text;
  if (style === '\\mathrm' && !letter) return text;
  if (style === '\\mathbf' && /^[^A-Za-z0-9]$/.test(text)) return `\\boldsymbol{${text}}`;
  return `${style}{${text}}`;
}

function miTex(text, variant) {
  if (!text) return { tex: '', kind: 'ord' };
  const chars = Array.from(text);
  if (chars.length > 1) {
    if (FUNCTIONS.has(text)) {
      return {
        tex: KNOWN_FUNCTION_COMMANDS.has(text) ? `\\${text}` : `\\operatorname{${text}}`,
        kind: 'op',
        limits: LIMIT_FUNCTIONS.has(text),
      };
    }
    if (/^[A-Za-z]+$/.test(text))
      return {
        tex:
          variant && VARIANTS[variant] && variant !== 'normal'
            ? `${VARIANTS[variant]}{${text}}`
            : `\\mathrm{${text}}`,
        kind: 'ord',
      };
    return { tex: chars.map((c) => fromUnicode(c)?.tex ?? c).join(''), kind: 'ord' };
  }
  const id = fromUnicode(text) ?? { tex: text, kind: 'ord' };
  if (variant && VARIANTS[variant] !== undefined && VARIANTS[variant] !== '') {
    const letter = /^[A-Za-z]$/.test(text);
    if (variant === 'normal' && !letter)
      return { tex: id.tex, kind: id.kind === 'letter' ? 'ord' : id.kind };
    if (variant === 'bold' && !letter && !/^[0-9]$/.test(text))
      return { tex: `\\boldsymbol{${id.tex}}`, kind: 'ord' };
    return { tex: `${VARIANTS[variant]}{${id.tex}}`, kind: 'ord' };
  }
  return { tex: id.tex, kind: id.kind === 'letter' ? 'ord' : id.kind };
}

function moTex(text) {
  if (!text) return { tex: '', kind: 'space' };
  if (FUNCTIONS.has(text)) {
    return {
      tex: KNOWN_FUNCTION_COMMANDS.has(text) ? `\\${text}` : `\\operatorname{${text}}`,
      kind: 'op',
      limits: LIMIT_FUNCTIONS.has(text),
    };
  }
  const parts = Array.from(text)
    .map((c) => fromUnicode(c))
    .filter((p) => p && p.kind !== 'space');
  if (parts.length === 0) return { tex: '', kind: 'space' };
  if (parts.length === 1) {
    const p = parts[0];
    if (p.kind === 'accent') return { tex: text, kind: 'accent' };
    if (p.kind === 'radical') return { tex: '\\surd', kind: 'ord' };
    if (p.kind === 'prime') return { tex: "'", kind: 'prime' };
    return { tex: p.tex, kind: p.kind === 'letter' ? 'ord' : p.kind };
  }
  const tex = parts.map((p) => p.tex).join('');
  if (text === ':=' || text === '≔') return { tex: ':=', kind: 'rel' };
  return { tex, kind: parts.some((p) => p.kind === 'rel') ? 'rel' : parts[0].kind };
}

function supTex(node, depth) {
  const s = mml(node, depth + 1);
  if (s.kind === 'prime') return "'";
  if (/^('|\\prime)+$/.test(s.tex)) return s.tex.replace(/\\prime/g, "'");
  return s.tex;
}

/** base + scripts, braced only where needed, and a base that already has a
 * script of the same kind braced so it is not a double script. */
function attachScripts(base, sub, sup) {
  let b = base;
  if (
    (sub && hasTopLevel(b, '_')) ||
    (sup && hasTopLevel(b, '^')) ||
    (sup && /'$/.test(b) && sup !== "'")
  ) {
    b = `{${b}}`;
  }
  let out = b;
  if (sub) out += `_${braced(sub)}`;
  if (sup) out += /^'+$/.test(sup) ? sup : `^${braced(sup)}`;
  if (sub && /^'+$/.test(sup)) out = `${b}${sup}_${braced(sub)}`;
  return out;
}

function hasTopLevel(tex, ch) {
  let depth = 0;
  for (let i = 0; i < tex.length; i++) {
    const c = tex[i];
    if (c === '\\') {
      i++;
      continue;
    }
    if (c === '{') depth++;
    else if (c === '}') depth--;
    else if (c === ch && depth === 0) return true;
  }
  return false;
}

function underOver(n, kids, depth) {
  const base = mml(kids[0] ?? { name: 'mrow', children: [] }, depth);
  const scriptNode = (i) => kids[i] ?? { name: 'mrow', children: [] };
  const overNode =
    n.name === 'mover' ? scriptNode(1) : n.name === 'munderover' ? scriptNode(2) : null;
  const underNode = n.name === 'munder' || n.name === 'munderover' ? scriptNode(1) : null;
  const single = Array.from(textOf(kids[0] ?? { text: '' }).trim()).length <= 1;
  let tex = base.tex;
  let kind = base.kind;
  const limitsBase = base.kind === 'bigop' || base.kind === 'op' || base.limits;
  if (limitsBase) {
    const under = underNode ? mml(underNode, depth + 1).tex : '';
    const over = overNode ? mml(overNode, depth + 1).tex : '';
    return { tex: attachScripts(tex, under, over), kind };
  }
  if (underNode) {
    const u = textOf(underNode).trim();
    if (underNode.name === 'mo' && UNDER_ACCENTS[u]) tex = `${UNDER_ACCENTS[u]}{${tex}}`;
    else tex = `\\underset{${mml(underNode, depth + 1).tex}}{${tex}}`;
    kind = 'ord';
  }
  if (overNode) {
    const o = textOf(overNode).trim();
    let accent =
      overNode.name === 'mo' || (overNode.name === 'mi' && o.length === 1 && OVER_ACCENTS[o])
        ? OVER_ACCENTS[o]
        : null;
    if (accent && !single && WIDE[accent]) accent = WIDE[accent];
    if (accent) tex = `${accent}{${tex}}`;
    else tex = `\\overset{${mml(overNode, depth + 1).tex}}{${tex}}`;
    kind = 'ord';
  }
  return { tex, kind };
}

function fenceTex(f) {
  if (f === '' || f == null) return '';
  const id = fromUnicode(f);
  return id ? id.tex : f;
}

/** A table: `aligned` when its columns are right/left pairs or its cells
 * begin with relations, otherwise `matrix`, or the environment the fences
 * around it name. */
function table(n, env) {
  const rows = elements(n).filter((r) => r.name === 'mtr' || r.name === 'mlabeledtr');
  const cellRows = rows.map((r) => {
    const cells = elements(r).filter((c) => c.name === 'mtd');
    return (r.name === 'mlabeledtr' ? cells.slice(1) : cells).map((c) => row(elements(c), 0));
  });
  const align = n.attrs.columnalign ?? '';
  const isAligned =
    !env &&
    (/right\s+left/.test(align) ||
      cellRows.some((r) =>
        r.slice(1).some((c) => /^(=|<|>|\\[lg]eq?|\\approx|\\equiv|\\sim|\\to)/.test(c))
      ));
  const body = cellRows.map((r) => r.join(' & ')).join(' \\\\ ');
  const name = env ?? (isAligned ? 'aligned' : 'matrix');
  return `\\begin{${name}} ${body} \\end{${name}}`;
}

function fencedTable(tableNode, open, close) {
  if (open === '\\{' && !close) return table(tableNode, 'cases');
  const env = open && MATRIX_ENVS[open] && fenceMatches(open, close) ? MATRIX_ENVS[open] : null;
  if (env) return table(tableNode, env);
  return `\\left${open || '.'} ${table(tableNode, 'matrix')} \\right${close || '.'}`;
}

function fenceMatches(open, close) {
  return { '(': ')', '[': ']', '\\{': '\\}', '|': '|', '\\|': '\\|' }[open] === close;
}

/** A row of MathML children → LaTeX, spacing relations, and turning
 * `( mtable )` into a matrix environment. */
function row(kids, depth) {
  const segs = [];
  for (let i = 0; i < kids.length; i++) {
    const k = kids[i];
    const next = kids[i + 1];
    const after = kids[i + 2];
    if (k.name === 'mo' && next?.name === 'mtable') {
      const open = fenceTex(textOf(k).trim());
      const close = after?.name === 'mo' ? fenceTex(textOf(after).trim()) : '';
      if (open === '\\{' && (!close || !fenceMatches(open, close))) {
        segs.push({ tex: table(next, 'cases'), kind: 'ord' });
        i += 1;
        continue;
      }
      if (fenceMatches(open, close)) {
        segs.push({ tex: fencedTable(next, open, close), kind: 'ord' });
        i += 2;
        continue;
      }
    }
    const s = mml(k, depth);
    if (!s.tex) continue;
    segs.push(s);
  }
  return segmentsToTex(segs, depth > 0 ? 1 : 0);
}

// ---------- JATS ----------

/** A JATS article's formulas, as snippets ready to match: the TeX the
 * publisher gives (`tex-math`) when there is some, else its MathML. */
function jatsFormulas(xml) {
  const out = [];
  const re = /<(disp-formula|inline-formula)\b[^>]*>([\s\S]*?)<\/\1>/g;
  let m;
  while ((m = re.exec(xml))) {
    const [, kind, body] = m;
    const label = /<label[^>]*>([\s\S]*?)<\/label>/.exec(body);
    const numbers = label
      ? [
          decodeEntities(label[1].replace(/<[^>]+>/g, ''))
            .replace(/[()]/g, '')
            .trim(),
        ]
      : [];
    let tex = '';
    const texMath = /<tex-math\b[^>]*>([\s\S]*?)<\/tex-math>/.exec(body);
    if (texMath) tex = texFromTexMath(texMath[1]);
    if (!tex) {
      const math = /<(?:\w+:)?math\b[\s\S]*?<\/(?:\w+:)?math>/.exec(body);
      if (math) {
        try {
          tex = mathmlToLatex(math[0]);
        } catch {
          tex = '';
        }
      }
    }
    if (tex) out.push({ tex, env: kind, display: kind === 'disp-formula', numbers });
  }
  return out;
}

function texFromTexMath(raw) {
  let s = raw.replace(/^\s*<!\[CDATA\[([\s\S]*?)\]\]>\s*$/, '$1');
  s = decodeEntities(s);
  const doc = /\\begin\{document\}([\s\S]*?)\\end\{document\}/.exec(s);
  if (doc) s = doc[1];
  s = s.trim();
  const wrapped =
    /^\$\$([\s\S]*)\$\$$/.exec(s) ??
    /^\$([\s\S]*)\$$/.exec(s) ??
    /^\\\[([\s\S]*)\\\]$/.exec(s) ??
    /^\\\(([\s\S]*)\\\)$/.exec(s);
  if (wrapped) s = wrapped[1];
  const env = /^\\begin\{(equation|align|gather)\*?\}([\s\S]*)\\end\{\1\*?\}$/.exec(s.trim());
  if (env)
    s =
      env[1] === 'equation'
        ? env[2]
        : `\\begin{${env[1] === 'align' ? 'aligned' : 'gathered'}}${env[2]}\\end{${env[1] === 'align' ? 'aligned' : 'gathered'}}`;
  return s
    .replace(/\\label\{[^}]*\}/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

// ---------- commands ----------

async function handle(command, value) {
  if (command === 'copyLatex') {
    return copyMath({ kind: 'selection', text: typeof value === 'string' ? value : '' });
  }
  if (command === 'copyEquation') {
    const v = value && typeof value === 'object' ? value : {};
    return copyMath({
      kind: 'equation',
      page: v.page,
      bbox: v.bbox,
      number: v.number ?? null,
      text: '',
    });
  }
  throw new Error(`unknown command "${command}"`);
}

async function readSettings() {
  const get = async (key, fallback) => {
    try {
      const v = await call('settings.get', { key });
      return v === undefined || v === null ? fallback : String(v);
    } catch {
      return fallback;
    }
  };
  const [arxiv, journals, prefer] = await Promise.all([
    get('arxiv', 'true'),
    get('journals', 'true'),
    get('prefer', 'most exact'),
  ]);
  const online = !/^fonts/i.test(prefer);
  return { arxiv: online && arxiv !== 'false', journals: online && journals !== 'false' };
}

/** An API that may be refused (a permission not granted, no document open)
 * answers null rather than failing the command. */
async function tryCall(api, args) {
  try {
    return await call(api, args);
  } catch (err) {
    log('info', `${api}: ${String(err?.message ?? err)}`);
    return null;
  }
}

/** The pipeline. Each step is used only when it applies, most exact first;
 * the first that answers is shown. */
async function copyMath(request) {
  const started = Date.now();
  const settings = await readSettings();

  let page = Number.isInteger(request.page) ? request.page : null;
  let glyphs = null;
  let bbox = toBox(request.bbox);
  if (request.kind === 'equation') {
    if (page !== null && bbox) {
      const r = await tryCall('region.glyphs', { page, bbox: [bbox.x, bbox.y, bbox.w, bbox.h] });
      glyphs = Array.isArray(r?.glyphs) ? r.glyphs : null;
    }
  } else {
    const r = await tryCall('selection.glyphs');
    if (r && Array.isArray(r.glyphs)) {
      glyphs = r.glyphs;
      if (Number.isInteger(r.page)) page = r.page;
      if (glyphs.length > 0)
        bbox = union(glyphs.map((g) => ({ x: num(g.x), y: num(g.y), w: num(g.w), h: num(g.h) })));
    }
  }

  // A scan is refused before anything else: there is nothing in it to read.
  if (glyphs && looksLikeScan(glyphs)) return refuseScan();
  const fonts = glyphs && glyphs.length > 0 ? reconstruct(glyphs) : null;
  if (fonts && fonts.decoded === 0 && glyphs.some((g) => g.font !== '__rule')) return refuseScan();

  const display =
    request.kind === 'equation' || (fonts ? fonts.rows > 1 : /\n/.test(request.text.trim()));
  const show = (latex, source, confidence) =>
    call('ui.showMath', { latex, display: display || /\\\\/.test(latex), source, confidence });

  // 1. The document's own tags.
  if (page !== null && bbox) {
    const tags = await tryCall('document.mathTags', { page });
    const tagged = Array.isArray(tags) ? fromTags(tags, bbox) : null;
    if (tagged) return show(tagged, 'tags', 'high');
  }

  // 2 and 3. Source online.
  const queries = [];
  if (fonts?.latex) queries.push(texTokens(fonts.latex).map((t) => t.t));
  if (request.text) queries.push(textTokens(request.text));
  const number = request.number ?? null;
  const worthMatching = queries.some((q) => q.length >= 3) || Boolean(number);
  if ((settings.arxiv || settings.journals) && worthMatching) {
    const ids = (await tryCall('document.identifiers')) ?? {};
    const arxiv = normalizeArxiv(ids.arxiv, ids.arxivVersion);
    if (settings.arxiv && arxiv) {
      const snippets = await withDeadline(arxivSnippets(arxiv.id, arxiv.version), started);
      const found =
        snippets &&
        matchSnippets(snippets, queries, { number, display: request.kind === 'equation' });
      if (found) return show(found.tex, 'arxiv', found.exact ? 'high' : 'medium');
    }
    const doi = typeof ids.doi === 'string' && ids.doi.trim() ? ids.doi.trim() : null;
    if (settings.journals && doi) {
      const snippets = await withDeadline(journalSnippets(doi), started);
      const found =
        snippets &&
        matchSnippets(snippets, queries, { number, display: request.kind === 'equation' });
      if (found) return show(found.tex, 'jats', found.exact ? 'high' : 'medium');
    }
  }

  // 4. The glyphs.
  if (fonts?.latex) return show(fonts.latex, 'fonts', fonts.confidence);
  if (request.text.trim()) {
    const latex = textToLatex(request.text);
    if (latex) return show(latex, 'fonts', 'low');
  }
  throw new Error(
    request.kind === 'equation'
      ? 'No equation there: hover a display equation and click Copy LaTeX.'
      : 'Select some math first.'
  );
}

function refuseScan() {
  log('info', SCAN_MESSAGE);
  throw new Error(SCAN_MESSAGE);
}

function toBox(b) {
  if (Array.isArray(b) && b.length >= 4 && b.every(Number.isFinite))
    return { x: b[0], y: b[1], w: b[2], h: b[3] };
  if (b && typeof b === 'object' && ['x', 'y', 'w', 'h'].every((k) => Number.isFinite(b[k]))) {
    return { x: b.x, y: b.y, w: b.w, h: b.h };
  }
  return null;
}

function withDeadline(promise, started) {
  const left = NETWORK_BUDGET_MS - (Date.now() - started);
  if (left <= 0) return Promise.resolve(null);
  let timer;
  return Promise.race([
    promise,
    new Promise((resolve) => {
      timer = setTimeout(() => resolve(null), left);
    }),
  ]).finally(() => clearTimeout(timer));
}

// ---------- 1. tags ----------

/** The Formula tag that covers the selection, as LaTeX: its alt text when
 * that is LaTeX, else its MathML. */
function fromTags(tags, box) {
  let best = null;
  let bestOverlap = 0;
  for (const tag of tags) {
    const t = toBox(tag?.bbox);
    if (!t) continue;
    const ix = Math.min(t.x + t.w, box.x + box.w) - Math.max(t.x, box.x);
    const iy = Math.min(t.y + t.h, box.y + box.h) - Math.max(t.y, box.y);
    if (ix <= 0 || iy <= 0) continue;
    const overlap = (ix * iy) / Math.max(1e-6, Math.min(t.w * t.h, box.w * box.h));
    if (overlap > bestOverlap) {
      bestOverlap = overlap;
      best = tag;
    }
  }
  if (!best || bestOverlap < 0.5) return null;
  const alt = typeof best.alt === 'string' ? latexFromAlt(best.alt) : null;
  if (alt) return alt;
  if (typeof best.mathml === 'string' && best.mathml.trim()) {
    try {
      const tex = mathmlToLatex(best.mathml);
      if (tex) return tex;
    } catch (err) {
      log('warn', `MathML in a tag could not be read: ${String(err?.message ?? err)}`);
    }
  }
  return null;
}

/** Alt text is LaTeX when it looks like LaTeX: commands, scripts, braces, or
 * a short formula with no words in it. "x squared" is a description. */
function latexFromAlt(alt) {
  let s = alt.trim();
  const wrapped =
    /^\$\$([\s\S]+)\$\$$/.exec(s) ??
    /^\$([\s\S]+)\$$/.exec(s) ??
    /^\\\[([\s\S]+)\\\]$/.exec(s) ??
    /^\\\(([\s\S]+)\\\)$/.exec(s);
  if (wrapped) s = wrapped[1].trim();
  if (!s) return null;
  if (/\\[A-Za-z]+|[_^{}]/.test(s)) return s;
  const words = s.match(/[A-Za-z]{3,}/g) ?? [];
  if (/^[\w\s+\-=*/().,<>|'!]+$/.test(s) && words.every((w) => FUNCTIONS.has(w))) return s;
  return null;
}

// ---------- 2. arXiv ----------

function normalizeArxiv(id, version) {
  if (typeof id !== 'string' || !id.trim()) return null;
  let s = id
    .trim()
    .replace(/^arxiv:/i, '')
    .replace(/^https?:\/\/arxiv\.org\/(abs|pdf)\//, '')
    .replace(/\.pdf$/, '');
  let v = Number.isInteger(version) && version > 0 ? version : null;
  const m = /^(.*?)v(\d+)$/.exec(s);
  if (m) {
    s = m[1];
    v ??= Number(m[2]);
  }
  if (!/^(\d{4}\.\d{4,5}|[a-z-]+(\.[A-Z]{2})?\/\d{7})$/.test(s)) return null;
  return { id: s, version: v };
}

/** The math of an arXiv paper's source: from memory, from storage, or
 * fetched and unpacked. An empty list means "this paper has no TeX source"
 * and is remembered too, so a PDF-only submission is fetched once. */
function arxivSnippets(id, version) {
  const key = `arxiv:${id}${version ? `v${version}` : ''}`;
  if (sources.has(key)) return sources.get(key);
  const work = (async () => {
    const stored = await readStored(key);
    if (stored) return stored;
    const url = `https://arxiv.org/e-print/${id}${version ? `v${version}` : ''}`;
    const res = await call('net.fetch', { url, as: 'bytes', maxBytes: ARXIV_MAX_BYTES });
    if (!res || res.status < 200 || res.status >= 300)
      throw new Error(`arXiv answered ${res?.status}`);
    const bytes = toBytes(res.body);
    if (/pdf/i.test(res.contentType ?? '') || isPdf(bytes)) {
      await store(key, []);
      return [];
    }
    const files = await readEprint(bytes);
    const snippets = files ? parseSource(files) : [];
    await store(key, snippets);
    return snippets;
  })().catch((err) => {
    log('warn', `arXiv source for ${id}: ${String(err?.message ?? err)}`);
    sources.delete(key);
    return null;
  });
  sources.set(key, work);
  return work;
}

function toBytes(body) {
  if (body instanceof Uint8Array) return body;
  if (Array.isArray(body)) return Uint8Array.from(body);
  if (typeof body === 'string') return Uint8Array.from(body, (c) => c.charCodeAt(0) & 0xff);
  return new Uint8Array(0);
}

// ---------- 3. open-access journal version ----------

function journalSnippets(doi) {
  const key = `doi:${doi.toLowerCase()}`;
  if (sources.has(key)) return sources.get(key);
  const work = (async () => {
    const stored = await readStored(key);
    if (stored) return stored;
    const search = await call('net.fetch', {
      url: `https://www.ebi.ac.uk/europepmc/webservices/rest/search?query=DOI:${encodeURIComponent(`"${doi}"`)}&format=json&resultType=lite`,
      as: 'text',
    });
    if (!search || search.status < 200 || search.status >= 300)
      throw new Error(`Europe PMC answered ${search?.status}`);
    const results = JSON.parse(String(search.body))?.resultList?.result ?? [];
    const hit = results.find((r) => typeof r?.pmcid === 'string' && /^PMC\d+$/.test(r.pmcid));
    if (!hit) {
      await store(key, []);
      return [];
    }
    const full = await call('net.fetch', {
      url: `https://www.ebi.ac.uk/europepmc/webservices/rest/${hit.pmcid}/fullTextXML`,
      as: 'text',
    });
    if (!full || full.status < 200 || full.status >= 300)
      throw new Error(`Europe PMC answered ${full?.status}`);
    const snippets = jatsFormulas(String(full.body));
    await store(key, snippets);
    return snippets;
  })().catch((err) => {
    log('warn', `Journal version of ${doi}: ${String(err?.message ?? err)}`);
    sources.delete(key);
    return null;
  });
  sources.set(key, work);
  return work;
}

// ---------- cache ----------

async function readStored(key) {
  try {
    const raw = await call('storage.get', { key: CACHE_PREFIX + key });
    if (typeof raw !== 'string') return null;
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    if (!parsed.every((s) => typeof s?.tex === 'string' && Array.isArray(s.numbers))) return null;
    return parsed;
  } catch {
    return null;
  }
}

/** Only what fits in one storage value is stored; a bigger source stays in
 * memory for the session. */
async function store(key, snippets) {
  const value = JSON.stringify(
    snippets.map(({ tex, env, display, numbers }) => ({ tex, env, display, numbers }))
  );
  if (value.length > STORAGE_LIMIT) return;
  try {
    await call('storage.set', { key: CACHE_PREFIX + key, value });
  } catch {
    // Not remembered across sessions; still in memory.
  }
}

// ---------- text only ----------

const SUPERSCRIPT = Object.fromEntries(
  Array.from('⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁿⁱ').map((c, i) => [c, '0123456789+-=()ni'[i]])
);
const SUBSCRIPT = Object.fromEntries(
  Array.from('₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑₒₓₕₖₗₘₙₚₛₜᵢⱼ').map((c, i) => [c, '0123456789+-=()aeoxhklmnpstij'[i]])
);

/** No glyphs, only the selected text: each character through the Unicode
 * table, Unicode super- and subscripts as scripts. Structure is otherwise
 * lost, so this is a low-confidence answer. */
function textToLatex(text) {
  const segs = [];
  let word = '';
  const script = (map, mark) => {
    let run = '';
    return {
      add(ch) {
        run += map[ch];
      },
      flush() {
        if (!run) return;
        const last = segs[segs.length - 1];
        const tex = `${mark}${braced(run)}`;
        if (last) last.tex += tex;
        else segs.push({ tex: `{}${tex}`, kind: 'ord' });
        run = '';
      },
    };
  };
  const sup = script(SUPERSCRIPT, '^');
  const sub = script(SUBSCRIPT, '_');
  const flushScripts = () => {
    sub.flush();
    sup.flush();
  };
  const flush = () => {
    if (!word) return;
    const fn = functionWords(word);
    if (fn)
      segs.push({
        tex: fn
          .map((f) => (KNOWN_FUNCTION_COMMANDS.has(f) ? `\\${f}` : `\\operatorname{${f}}`))
          .join(''),
        kind: 'op',
      });
    else for (const c of word) segs.push({ tex: c, kind: 'ord' });
    word = '';
  };
  for (const ch of Array.from(text)) {
    if (SUPERSCRIPT[ch] || SUBSCRIPT[ch]) {
      flush();
      (SUPERSCRIPT[ch] ? sup : sub).add(ch);
      continue;
    }
    if (/^[A-Za-z]$/.test(ch)) {
      flushScripts();
      word += ch;
      continue;
    }
    flush();
    flushScripts();
    const id = fromUnicode(ch);
    if (!id || !id.tex || id.kind === 'space') continue;
    if (id.kind === 'accent' || id.kind === 'radical') {
      segs.push({ tex: id.kind === 'radical' ? '\\surd' : '', kind: 'ord' });
      continue;
    }
    segs.push({
      tex: id.tex,
      kind: id.kind === 'letter' ? 'ord' : id.kind === 'prime' ? 'ord' : id.kind,
    });
  }
  flush();
  flushScripts();
  return tidy(segmentsToTex(segs, 0));
}
