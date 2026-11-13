# Notation

Every symbol and acronym the open paper defines, each with the phrase that
defines it and the page it is defined on. Picking a row goes to the sentence
that defines it, not just the top of its page, and marks it for a moment so
you can see where to look. Useful in long theory papers, where by section five
you have lost track of what NESS stood for and which lambda was the control
parameter.

Load it from **Home → Extensions**, open a paper, then pick **Notation** in the
sidebar. **Notation: Rescan Document** reads the paper again.

## Settings

- **Scan pages**: the first 50, the first 200 (the default), or all of them.
- **Show acronyms**: list acronyms alongside symbols (on by default).

## What it reads as a definition

Acronyms, in either order:

- `nonequilibrium steady state (NESS)`, `deep neural networks (DNNs)`,
  `Markov chain Monte Carlo (MCMC; see [3])`
- `MCMC (Markov chain Monte Carlo)`

An acronym is two to ten characters, starts with a letter, has at least two
capitals and is mostly capitals and digits; a plural `s` is dropped. Its long
form has to spell it: either every letter appears in order with the first one
starting a word (Schwartz and Hearst's rule, which is what reads
"*n*on*e*quilibrium *s*teady *s*tate" as NESS), or the words' initials spell
it, skipping small words like _of_ and _the_, splitting hyphenated words, and
allowing one wrong letter ("carbon dioxide (CO2)"). So "(Fig. 2)", "(see
[12])", "(2024)", "(i)", "(a)" and "(II)" are never acronyms, and neither is a
capitalised word in parentheses whose letters the words before it do not
spell. The badge counts the uses after the definition, plurals included.

Symbols, in sentences shaped like:

| Shape                                         | Example                                                 |
| --------------------------------------------- | ------------------------------------------------------- |
| Let _X_ denote / be / represent / stand for … | Let λ denote the control parameter                      |
| where _X_ is / denotes / represents …         | where m is the mass, k the spring constant, and g the … |
| _X_ denotes / represents / stands for …       | σ denotes the noise strength                            |
| we write / use _X_ for …                      | We write x_i for the position of the ith particle       |
| define _X_ as / to be …                       | We define β as the inverse temperature                  |
| denote by _X_ the …                           | Denote by Ω the sample space                            |
| denote / write the … by / as _X_              | We denote the entropy by S                              |
| the …, denoted (by) _X_                       | The partition function, denoted Z(θ), …                 |
| _X_ := …                                      | F := -log Z                                             |
| _X_ is called …                               | H is called the Hamiltonian                             |
| with _X_ the …                                | with γ the damping rate                                 |

A symbol is one Latin or Greek letter, with accents (x̂), up to three sub- and
superscripts as the PDF's text gives them (`x_i`, `β_t`, `x1`, `E₀`, `x²`,
`f'`, `ω*`, or `x i` with the subscript set off by a space), and a short
argument (`Z(θ)`, `f(x, t)`). Mathematical italic and bold letters (𝜆, 𝑥) are
read as the plain letter, and Greek variants (ϑ, ϕ, ϵ) as their usual form.
Symbols are drawn as TeX, `$\lambda$`, which the panel typesets; Greek letters
and math symbols inside a defining phrase are typeset too.

The first definition of each symbol or acronym wins: a paper that reuses λ in
an appendix keeps the meaning it gave λ first. Symbols are listed in the order
they are defined, acronyms alphabetically.

## How it is built

| Feature                                  | Used for                                             |
| ---------------------------------------- | ---------------------------------------------------- |
| `document.pageText`                      | Reading each page's text                             |
| `document.pageRuns` + `navigate.toPlace` | Finding and marking the defining sentence            |
| `panels` + `ui.render`                   | The list, with TeX titles                            |
| `settings`                               | How many pages to read, and whether to list acronyms |
| `storage`                                | Caching the scan per document                        |

It reads eight pages at a time, draws progress every 32 pages, and answers
every invoke within the five-second budget; a scan still going after four
seconds keeps going in the background and draws when it finishes. A scan is
cached under the document's name, page count and the number of pages read, so
changing **Scan pages** reads again and reopening the panel does not.

## What it cannot do

- **Definitions written as equations only.** A symbol introduced in a display
  block ("$$H = \sum_i \ldots$$") with no sentence around it is invisible: the
  extension reads prose, not mathematics.
- **Lists of symbols.** "Let x, y and z be coordinates" and "where α and β
  are rates" define several symbols at once and are not read.
- **Symbols longer than one letter.** `Tr`, `KL`, `\mathrm{Var}` and the like
  are not symbols here; an all-capitals one may still be read as an acronym.
- **Text the PDF scrambled.** Subscripts extracted on a separate line, or a
  formula whose characters came out in a different order, do not match.
  Scanned pages without a text layer have no text to read.
- **An acronym introduced without its long form**, or one whose long form does
  not spell it ("(QCD) the strong force"), is not listed.
