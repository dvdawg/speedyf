#!/bin/bash
# Typeset the evaluation papers for engine::display_eval: body.tex and
# body2.tex, each in five layouts (article, IEEEtran conference, elsarticle
# 5p with fleqn, amsart with leqno, two-column article), with evalmark.sty
# colouring every display so its truth can be read off the PDF.
#
#   bash src-tauri/tests/displays/build.sh /tmp/displays
#
# writes the PDFs to the directory given (made if missing) and prints the
# list SPEEDYF_DISPLAY_EVAL takes.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:?usage: build.sh OUTDIR}"
mkdir -p "$out"
cp "$here/evalmark.sty" "$here/body.tex" "$here/body2.tex" "$out/"
cd "$out"
P='\usepackage{evalmark}\usepackage{amssymb,amsthm,booktabs,pgfplots,algorithm,algpseudocode}\pgfplotsset{compat=1.18}\newtheorem{theorem}{Theorem}\newtheorem{lemma}{Lemma}'
list=""
for B in body body2; do
  S="${B#body}"
  printf '%s\n' "\\documentclass[11pt]{article}\\usepackage[margin=1in]{geometry}$P\\begin{document}\\title{Single column}\\author{A. Author}\\maketitle\\input{$B}\\end{document}" > "single$S.tex"
  printf '%s\n' "\\documentclass[conference]{IEEEtran}$P\\begin{document}\\title{Two columns}\\author{A. Author}\\maketitle\\input{$B}\\end{document}" > "ieee$S.tex"
  printf '%s\n' "\\documentclass[5p,fleqn]{elsarticle}$P\\begin{document}\\begin{frontmatter}\\title{Elsevier fleqn}\\author{A. Author}\\end{frontmatter}\\input{$B}\\end{document}" > "els$S.tex"
  printf '%s\n' "\\documentclass[leqno]{amsart}$P\\begin{document}\\title{AMS leqno}\\author{A. Author}\\maketitle\\input{$B}\\end{document}" > "amsleq$S.tex"
  printf '%s\n' "\\documentclass[10pt,twocolumn]{article}\\usepackage[margin=0.75in]{geometry}$P\\begin{document}\\title{Two-column article}\\author{A. Author}\\maketitle\\input{$B}\\end{document}" > "twocol$S.tex"
  for f in "single$S" "ieee$S" "els$S" "amsleq$S" "twocol$S"; do
    pdflatex -interaction=nonstopmode "$f.tex" > /dev/null 2>&1 || true
    pdflatex -interaction=nonstopmode "$f.tex" > /dev/null 2>&1 || true
    list="$list;$out/$f.pdf"
  done
done
echo "${list#;}"
