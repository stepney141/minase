# 探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通した版だけを統括計画の前のmasterとLTCで比べる

## ステータス

承認済み。決定日は2026年10月18日。

## コンテキスト

minaseの機能採否は、STCで選別し、LTCの`H1`だけで採用を決める（[段階ゲートのADR](screen-at-stc-and-decide-at-ltc.md)、[棋力測定の手引き](../guides/sprt.md)）。
STCで`H0`となった候補は採用せず、LTCへも進めない。
SPSAの値も、最終値を既定値へ書き込んだ候補として同じ段階ゲートで採否を決める（[SPSAの値の採否のADR](adopt-spsa-values-only-through-the-staged-gate.md)）。

[探索の関数形の見直しの統括計画](../plans/search-forms.md)は、minaseの探索と時間管理の式をStockfishとYaneuraOuの形へ寄せる7本の設計書をサブタスクとして束ねる。
各サブタスクは、式の形を変える実装をしたうえで、その時点の全係数を30,000ペアのSPSAで調整する。
以下、SPSAのためにvast.aiで借りる計算機を調整機、開発と棋力測定に使う手元の機械を測定機と呼ぶ。
SPSAは調整機で行い、その後の事前対局とSTCも、同じ調整機で持ち時間を換算せずに行う（[調整機のADR](run-spsa-on-rented-machines-with-nps-scaled-time-controls.md)）。
調整機の1コアは測定機より遅いので、換算しない持ち時間の対局では、1手の探索量が測定機の同じ持ち時間より少なく、試走の機械では約1/1.46だった。

サブタスク1の[枝刈りとLMRの関数形の見直し](../plans/archived/search-pruning-forms.md)では、SPSAの値を反映した候補を、設計書を始めた時点のmaster`99ccc6bb`と`time=10000+100`のSTCで比べた。
判定は776ペアで`H0`、LLRは−2.962であり、時間切れとエンジン異常は0件だった（[STCの記録](../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
有効766ペアの度数から求めたEloは約−17であるが、固定局数の推定ではない。
2026年10月18日、このSTCの実行中に、エンジンの開発者は、後続のサブタスクで強さが改善される可能性があるので、サブタスク1をSTCの結果によらず採用すると決めた。
続けて、各サブタスクではSPRTを行わなくてもよいとして、この扱いを7本すべてに広げた。

## 決定

統括計画の7本のサブタスクは、それぞれのSTCの判定によらず採用する。
各サブタスクでは、時間切れとエンジン異常がないことを確かめるために事前対局を行い、異常が出れば原因を直してから進む。
STCは採否に使わない記録として行い、サブタスクごとのLTCは行わない。
7本を終えた後のmasterの先頭を候補、統括計画を始める前のmasterの先頭`99ccc6bb`を基準として、標準のLTC`time=60000+200`のGSPRTを調整機で持ち時間を換算せずに行い（統括計画のフェーズ10）、`H1`でなければ、どのサブタスクから見直すかをエンジンの開発者が決める。
これらは2026年10月18日のエンジンの開発者の決定であり、適用するのは統括計画の7本のサブタスクだけとする。

## 検討した代案

各サブタスクでSTCとLTCの段階ゲートを行い、その判定で採否を決める案は採らなかった。
1本あたり約1.5〜6.5時間かかり、後のサブタスクで改善しうる形を早い段階で捨てるからである。
STCも行わない案は、フェーズ10で強くなっていなかった場合に、どのサブタスクで後退したかの手がかりがなくなるので採らなかった。
7本を通した確認を置かない案は、土台の後退を後のサブタスクの改善が取り戻すだけの場合にも気づけないので採らなかった。

## 結果

7本の途中のmasterは、統括計画の前のmasterより弱い可能性を含んだまま、次のサブタスクの起点になる。
サブタスク1の変更は、STCで`H0`のまま採用した。
サブタスク1による新しい決定は、[静的評価とcorrection history](archive/key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)、[improving](archive/judge-improving-on-the-corrected-static-eval-and-use-it-in-pruning-and-lmr.md)、[浅いノードの前向き枝刈りとreverse futility pruning](archive/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)、[LMR](archive/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)、[late move pruning](archive/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)、[置換表の打ち切りとnull move](archive/allow-null-move-and-tt-cutoff-at-pv-nodes-and-use-node-types-at-zero-window-nodes.md)、および[IIR](reduce-depth-by-one-without-tt-move-except-at-all-nodes-and-on-the-previous-pv.md)のADRに記録した。

各サブタスクのSTCは、調整機で持ち時間を換算せずに行うので、測定機で行った過去の段階ゲートとは条件が異なり、結果を比べられない。
7本を通して元のmasterより強くなったかは、フェーズ10の1回のLTCだけで確かめる。
フェーズ10で`H1`となっても、サブタスクごとの寄与は分からない。
一部の設計書が予定する秒読みつきの持ち時間での確認の扱いは、本決定では定めず、そのサブタスクを始めるときに決める。

この決定は統括計画の7本に限った例外であり、段階ゲートのADR、SPSAの値の採否のADR、および棋力測定の手引きの原則は、それ以外の変更の採否に引き続き適用する。
