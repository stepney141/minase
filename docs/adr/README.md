# 設計判断の記録（ADR）

本ディレクトリには、minaseの設計判断を1決定1ファイルで記録するADR（Architecture Decision Record）を置く。
ADRは、採った方式、棄却した代案、およびその根拠を、決定した時点の文脈とともに残す文書である。
仕様の正はコードとそのdocコメントにあり、ADRは仕様を繰り返さず、コードを読むだけでは分からない「なぜその方式なのか」を記録する。
形式は、ADRの共同リポジトリの[日本語版の解説](https://github.com/architecture-decision-record/architecture-decision-record/tree/main/locales/ja-001)と、そこに掲げられたMichael Nygardのテンプレートに基づく。

## ADRに書く決定

ADRに書くのは、次の2種類の決定である。
1つは、現行の実装が従っている方式の選択のうち、コードを読むだけでは理由が分からないものである。
もう1つは、測定または調査で棄却した方式のうち、同じ提案が繰り返されるのを防ぐべきものである。

次の内容はADRに書かず、それぞれの置き場所へ書く。
中将棋の規則とその解釈は [RULES.md](../rules/RULES.md) へ、作業の進め方の規則は [lessons/](../lessons/README.md) へ、マイルストーンの一時的な計画は [plans/](../plans/README.md) へ書く。
棋力測定の結果はADRの本文に写さず、[measurements/](../measurements/) の測定記録へリンクする。

## ファイル名

ファイル名は、決定の内容を表す現在形の動詞句を、小文字の英語とハイフンで書き、拡張子を`.md`とする。
たとえば、mimallocを使う決定は`use-mimalloc-in-executables.md`とする。
番号は付けず、決定の順序は本書の索引の日付で示す。

## テンプレート

ADRは次の見出しを順に持つ。

```markdown
# <決定の内容を表す題名>

## ステータス

承認済み。決定日は2026年10月13日。

## コンテキスト

## 決定

## 検討した代案

## 結果
```

「コンテキスト」には、決定を迫った状況、制約、および判断の材料を書く。
材料が測定であれば測定記録へ、調査であれば調査メモへリンクし、決定を下したマイルストーンの設計書へのリンクも置いてよい。
「決定」には、採った方式を1〜3文で書く。
「検討した代案」には、棄却した方式を1つずつ、棄却の理由とともに書く。
「結果」には、決定によって生じる利点、費用、および今後の作業への制約を書く。

## ステータス

ステータスは、提案、承認済み、置き換え済み、廃止のいずれかとする。
提案は、決定がまだ下されていない状態である。
承認済みは、現行の実装が従っている状態である。
置き換え済みは、後のADRが同じ問題について別の決定を下した状態であり、「置き換え済み。[<後のADRの題名>](<ファイル名>)によって置き換えられた。」と書く。
廃止は、決定の対象そのものが実装からなくなった状態である。

## 不変性

承認済みのADRは、ステータスの行を除いて本文を書き換えない。
決定を変えるときは、新しいADRを書き、そのコンテキストで古いADRにリンクしたうえで、古いADRのステータスを置き換え済みに書き換える。
誤字とリンク切れの修正だけは、この規則の例外とする。
本文を後から追記して更新し続ける運用は、記述が時系列に積み重なって現在の決定が読み取れなくなるので採らない。

## 既存の設計書からの移行

ADRを導入する前に完了した設計書は、設計判断をその本文に持っている。
これらの設計判断は、サブシステムごとにADRへ移す。
移すのは、現在も有効な決定と、再提案を防ぐべき棄却の決定だけであり、すでに置き換えられた決定の連鎖は再現しない。
たとえば既定の評価関数は11回置き換わったが、ADRには現行の方式を採った決定だけを書き、それ以前の経緯は「コンテキスト」の節から凍結した設計書へリンクする。

## 用語

ADRに繰り返し現れる用語を、ここで定義する。各ADRは、これらの語を説明なしに使う。

採否の測定は、候補と基準の2つのコミットを自己対局で比べる逐次検定で行う。手順と判定の規則の正は[棋力測定の手引き](../guides/sprt.md)にある。
GSPRTは、同じ開始局面で先後を入れ替えた2局を1ペアとし、ペアごとの得点で対数尤度比（LLR）を更新する逐次検定である。
`H1`は候補が基準より強いと判定された結果、`H0`は10 Eloの改善があるとは言えないと判定された結果を指す。
STCは持ち時間10秒と1手0.1秒の加算の短い時間制御、LTCは持ち時間60秒と1手0.2秒の加算の長い時間制御であり、採用はLTCの`H1`で決める。
有効ペアは、手数の上限に達して破棄されたペアを除いた、統計に算入されたペアである。
benchは、`minase dev bench`で決まった局面の集合を探索し、ノード数と時間を測る診断である。
SPSAは、多数の係数を自己対局の結果から同時に調整する手法であり、手順は[SPSAの手引き](../guides/spsa.md)にある。

探索の用語は次のとおりである。
零窓とは、βがα＋1である探索窓をいい、零窓で探索するノードを非PVノードという。
PVノードは、窓の幅が1より大きく、最善の応手列である主変化の候補を探すノードである。
詰み帯とは、評価値の絶対値が`MATE_THRESHOLD`以上で、詰みまでの手数を表す値の範囲をいう。
疑似利きとは、獅子の捕獲制限などの獅子の特殊規則を考慮せず、駒の動きだけで求めた利きをいう。
歩兵価値とは、探索用駒価値における歩兵の値であり、探索の係数の多くはこれに対する百分率で表す。

## 索引

決定日の古い順に並べる。

- 2026年7月31日 [perftの数値を合法手生成の正しさの基準にしない](verify-movegen-without-perft-counts.md)
- 2026年8月2日 [独立したリファクタリングをマイルストーンにしない](fold-refactoring-into-milestones.md)
- 2026年8月22日 [Zobristハッシュを64ビットに保つ](keep-zobrist-keys-at-64-bits.md)
- 2026年8月22日 [`try_make_move`は全合法手との照合で検査する](check-try-make-move-against-all-legal-moves.md)
- 2026年8月22日 [実行ファイルでmimallocを使う](use-mimalloc-in-executables.md)
- 2026年8月22日 [探索木の中では規則の裁定を近似し、厳密な裁定をルートと審判層に委ねる](approximate-adjudication-inside-search-tree.md)
- 2026年8月22日 [王手延長を実装しない](omit-check-extension.md)
- 2026年8月23日 [探索ワーカー数の既定値を1にする](keep-default-threads-at-one.md)
- 2026年8月23日 [置換表のエントリを2語の原子値にする](use-atomic-two-word-transposition-entries.md)
- 2026年8月23日 [補助ワーカーは深さを飛ばし、最も深い反復を完了したワーカーの結果を採る](diversify-lazy-smp-workers-by-skipping-depths.md)
- 2026年8月23日 [静止探索の結果を深さ0で置換表へ保存する](store-quiescence-results-at-depth-zero.md)
- 2026年9月5日 [SEEは獅子規則に依存する交換段階だけを判定不能とする](mark-only-lion-rule-dependent-exchanges-unknown-in-see.md)
- 2026年9月5日 [静止探索に専用の深さ上限を置かない](omit-quiescence-depth-limit.md)
- 2026年9月6日 [null moveのverification searchを実装しない](omit-null-move-verification-search.md)
- 2026年9月6日 [浅いノードの前向き枝刈りは王駒に利きが届くノードでは行わない](skip-static-eval-pruning-when-a-royal-is-attacked.md)
- 2026年9月7日 [LMRの減深量は対数表に3段階のhistory補正を加えて求める](reduce-lmr-by-log-table-with-stepped-history-adjustment.md)
- 2026年9月7日 [静かな手のhistoryはbutterfly表だけで引く](keep-only-the-butterfly-history-table.md)
- 2026年9月8日 [探索用駒価値は学習PSTから導いた値に固定する](derive-search-piece-values-from-the-learned-pst.md)
- 2026年9月12日 [aspiration windowsは外れた側だけを広げ、fail-high時に浅く読み直さない](widen-only-the-failed-side-of-aspiration-windows.md)
- 2026年9月12日 [反復の延長信号を使わず、最善手が安定したときだけ早く止める](stop-early-on-stable-best-move-without-extensions.md)
- 2026年9月12日 [置換表の記録手がないノードを1だけ浅く読む](reduce-depth-by-one-without-tt-move.md)
- 2026年9月12日 [置換表を1スロット1エントリの直接マップのまま保つ](keep-transposition-table-direct-mapped.md)
- 2026年9月12日 [静的評価を置換表に保存しない](do-not-store-static-eval-in-tt.md)
- 2026年9月17日 [探索内の反復検出は対局履歴と探索経路の線形走査で行う](check-repetition-by-linear-scan.md)
- 2026年9月19日 [ノード数は着手を適用する直前に1つずつ共有カウンタへ数える](count-nodes-per-move-on-shared-counter.md)
- 2026年9月21日 [correction historyは駒種別の枚数を鍵にし、futilityの判定にだけ使う](key-correction-history-by-material.md)
- 2026年9月21日 [浅い非PVノードでSEEの損が余裕値を超える捕獲手を展開しない](prune-losing-captures-by-see-at-shallow-nodes.md)
- 2026年9月21日 [獅子の2段階の非捕獲移動だけを浅く読む減深を実装しない](omit-lion-two-step-quiet-move-reduction.md)
- 2026年9月21日 [発動率の低い探索手法を実装しない](skip-low-activation-search-techniques.md)
- 2026年9月28日 [null moveと置換表の打ち切りをPVノードでも行う](allow-null-move-and-tt-cutoff-at-pv-nodes.md)
- 2026年9月28日 [反復の先読み（upcoming repetition detection）を採らない](reject-upcoming-repetition-detection.md)
- 2026年9月29日 [null moveより後の局面だけで反復を検出する](detect-repetition-only-after-latest-null-move.md)
- 2026年9月29日 [王駒がすべて狙われた局面でも静止探索でstand-patを使う](use-stand-pat-when-royals-are-attacked.md)
- 2026年9月29日 [静止探索のdelta pruningに成り益を加えない](keep-delta-pruning-without-promotion-gain.md)
- 2026年10月4日 [improvingはfutilityの余裕値を縮めることにだけ使う](use-improving-only-for-futility-margins.md)
- 2026年10月4日 [late move pruningは静かな手だけを数え、履歴の条件を付けない](count-only-quiet-moves-for-late-move-pruning.md)
- 2026年10月4日 [単独で不採用だった探索の8項目を、SPSAで再調整して一括で採用する](adopt-revived-search-items-as-a-bundle.md)
- 2026年10月4日 [静止探索は実際に読んだ捕獲手の数で打ち切る](limit-searched-captures-in-qsearch.md)
- 2026年10月10日 [butterfly historyと捕獲履歴を対局内で減衰させて持ち越す](carry-history-tables-within-a-game.md)
- 2026年10月10日 [lishogiのBotの`ByoyomiMargin`を3,000 msにする](set-bot-byoyomi-margin-to-3000-ms.md)
- 2026年10月10日 [先頭の捕獲手だけをSEEで判定し、損ならkiller手を先に返す](try-killers-before-a-losing-first-capture.md)
- 2026年10月10日 [捕獲履歴の寄与を歩兵価値の定数倍に抑え、静止探索の順序にも使う](bound-capture-history-and-use-it-in-qsearch.md)
- 2026年10月10日 [根の2手目以降を前回反復の部分木ノード数で並べる](order-root-moves-by-previous-subtree-nodes.md)
- 2026年10月10日 [静かな手の順序を減点つきの重力式と線形経路の差分の同点破りで決める](order-quiets-with-malus-and-pst-tiebreak.md)
