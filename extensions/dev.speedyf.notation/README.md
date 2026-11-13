# Notation

Every symbol and acronym the open paper defines, each with the phrase that
defines it and the page it is defined on. Picking a row goes to the sentence
that defines it, not just the top of its page, and marks it for a moment so
you can see where to look. Useful in long theory papers, where by section five
you have lost track of what NESS stood for and which lambda was the control
parameter.

Load it from **Home -> Extensions**, open a paper, then pick **Notation** in the
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
spell. The badge gives the page it is defined on and counts the uses after
the definition, plurals included: `p. 1, used 12x`.

Symbols, in sentences shaped like:

| Shape                                           | Example                                                   |
| ----------------------------------------------- | --------------------------------------------------------- |
| Let _X_ denote / be / represent / stand for ... | Let $\lambda$ denote the control parameter                |
| where _X_ is / denotes / represents ...         | where m is the mass, k the spring constant, and g the ... |
| _X_ denotes / represents / stands for ...       | $\sigma$ denotes the noise strength                       |
| we write / use _X_ for ...                      | We write x_i for the position of the ith particle         |
| define _X_ as / to be ...                       | We define $\beta$ as the inverse temperature              |
| denote by _X_ the ...                           | Denote by $\Omega$ the sample space                       |
| denote / write the ... by / as _X_              | We denote the entropy by S                                |
| the ..., denoted (by) _X_                       | The partition function, denoted $Z(\theta)$, ...          |
| _X_ := ...                                      | F := -log Z                                               |
| _X_ is called ...                               | H is called the Hamiltonian                               |
| with _X_ the ...                                | with $\gamma$ the damping rate                            |

Definitions joined in one sentence are listed separately: "where D is the
diameter of the point set and $\rho$ is the distance between the convex hulls",
"Let F be an infinite set and C a collection of subsets of F", "x the ..., y the
..., and z the ...". A sentence is split at "and", a comma or a semicolon only
where a symbol follows with _is_, _are_, _denotes_, _represents_, _stands for_
or straight away _the_, _a_ or _an_; any other "and" is part of the phrase ("if
and only if").

A symbol is one Latin or Greek letter, with accents ($\hat{x}$), up to three sub- and
superscripts as the PDF's text gives them (`x_i`, $\beta_t$, `x1`, $E_0$ or
$x^2$ written with Unicode sub- and superscripts, `f'`, $\omega^*$, or `x i` with the subscript set off by a space), and a short
argument ($Z(\theta)$, `f(x, t)`). Mathematical italic and bold letters (the Unicode
math-italic lambda or x) are
read as the plain letter, and Greek variants ($\vartheta$, $\phi$, $\epsilon$) as their usual form.
Symbols are drawn as TeX, `$\lambda$`, which the panel typesets, with the page
they are defined on in the badge. Math inside a defining phrase is typeset too:
symbols the paper defines, single letters used as variables (not "a" or "A"
before a word, or "I" before a verb), Greek letters and math characters, with
neighbouring math and the operators and numbers between joined into one
formula, so "if and only if S(C) < +infinity" (with the Unicode infinity sign)
shows `$S(C) < +\infty$`.

A phrase runs to the end of its clause, and never ends on a word left waiting
for more (_of_, _on_, _for which_, _the_, _to be_ ...) or inside brackets: it
runs on to the next natural end, up to 200 characters, or is cut back to its
last complete word and marked `...`. Where the sentence itself stops short -
extraction dropped the inline math in "a set of elementary events on $\Omega$" - the
phrase keeps its words and shows `...` for what is missing. A symbol with an
argument defined as the same thing as the bare symbol (`X(l)` and `X`) is
listed once.

The first definition of each symbol or acronym wins: a paper that reuses $\lambda$
in an appendix keeps the meaning it gave $\lambda$ first. Symbols are listed in the order
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
- **Lists of symbols sharing one phrase.** "Let x, y and z be coordinates"
  and "where $\alpha$ and $\beta$ are rates" define several symbols at once and are not
  read.
- **Symbols longer than one letter.** `Tr`, `KL`, `\mathrm{Var}` and the like
  are not symbols here; an all-capitals one may still be read as an acronym.
- **Text the PDF scrambled.** Subscripts extracted on a separate line, or a
  formula whose characters came out in a different order, do not match.
  Scanned pages without a text layer have no text to read.
- **An acronym introduced without its long form**, or one whose long form does
  not spell it ("(QCD) the strong force"), is not listed.
