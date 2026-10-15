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
決定を下した主体を書く場合は「エンジンの開発者」と書き、エンジンを使う側の「利用者」と区別する。

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

採否の測定は、候補と基準の2つのコミットを自己対局で比べる逐次検定で行う。手順と判定の規則は[棋力測定の手引き](../guides/sprt.md)が定める。
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

中将棋と評価関数の用語は次のとおりである。
じっとは、獅子などが隣接升へ進んで同じ着手の中で元の升へ戻る非捕獲の着手であり、盤面を変えないが単なる手番の放棄ではない合法な1手である（[RULES.md](../rules/RULES.md)第3条第13項と第6条第4項）。
規則コードは、L、P、R、またはEで始まる、ローカルルールを選ぶ記号であり、意味はRULES.md第29条から第32条が定める。たとえばE1は、詰みでは終局せず最後の王駒が実際に取られるまで対局を続ける規則である。
PSTは駒の種類と升ごとの値の表による評価、FMはFactorization Machineによる2駒の関係の補正であり、2026年10月12日にNNUE（差分更新できる小型のニューラルネットワーク）へ置き換わるまで、この2つが既定の評価関数を構成した。

## 索引

決定日の古い順に並べる。

- 2026年7月30日 [先獅子は着手の適用で捕獲升を記録し、足の判定は合法手生成で行う](record-lion-trigger-on-move-and-judge-foot-in-movegen.md)
- 2026年7月30日 [経由升で取った歩兵と仲人の足は、取られる前の利きで判定する](judge-pawn-foot-by-control-before-capture.md)
- 2026年7月31日 [perftの数値を合法手生成の正しさの基準にしない](verify-movegen-without-perft-counts.md)
- 2026年7月31日 [着手を経由升の捕獲だけを区別する正準形で表す](represent-moves-in-canonical-form.md)
- 2026年8月2日 [独立したリファクタリングをマイルストーンにしない](fold-refactoring-into-milestones.md)
- 2026年8月9日 [局面と合法手生成器を対局の履歴から独立させる](keep-game-history-out-of-position-and-movegen.md)
- 2026年8月9日 [日本中将棋連盟式の千日手（R0）を実装せず、既定の反復規則をR1にする](do-not-implement-r0-repetition.md)
- 2026年8月10日 [次局の規則を、対局開始前に受理した最初の局面設定で確定する](commit-pending-rules-on-first-position.md)
- 2026年8月11日 [エンジンの異常は起こした側の負けとし、手数上限に達したペアは破棄する](score-engine-faults-as-losses-and-discard-max-ply-pairs.md)
- 2026年8月11日 [棋力測定はコミットからビルドした別プロセスのエンジン同士で行う](compare-commits-as-separate-engine-processes.md)
- 2026年8月14日 [USI経路では探索の手を対局へ適用せず、局面の同期に失敗した後は着手を返さない](keep-searched-moves-out-of-the-usi-game.md)
- 2026年8月14日 [探索の起動と停止を対局状態機械に入れず、プロトコルの各モジュールが直接呼ぶ](call-search-from-protocol-modules.md)
- 2026年8月15日 [テストの期待値を規範文書から導き、現在の実装から導かない](derive-test-expectations-from-normative-documents.md)
- 2026年8月22日 [Zobristハッシュを64ビットに保つ](keep-zobrist-keys-at-64-bits.md)
- 2026年8月22日 [`try_make_move`は全合法手との照合で検査する](check-try-make-move-against-all-legal-moves.md)
- 2026年8月22日 [実行ファイルでmimallocを使う](use-mimalloc-in-executables.md)
- 2026年8月22日 [採否はペア単位のペンタノミアルGSPRTで判定する](judge-adoption-by-pentanomial-gsprt.md)
- 2026年8月22日 [探索木の中では規則の裁定を近似し、厳密な裁定をルートと審判層に委ねる](approximate-adjudication-inside-search-tree.md)
- 2026年8月22日 [王手延長を実装しない](omit-check-extension.md)
- 2026年8月23日 [探索ワーカー数の既定値を1にする](keep-default-threads-at-one.md)
- 2026年8月23日 [置換表のエントリを2語の原子値にする](use-atomic-two-word-transposition-entries.md)
- 2026年8月23日 [補助ワーカーは深さを飛ばし、最も深い反復を完了したワーカーの結果を採る](diversify-lazy-smp-workers-by-skipping-depths.md)
- 2026年8月23日 [静止探索の結果を深さ0で置換表へ保存する](store-quiescence-results-at-depth-zero.md)
- 2026年8月25日 [規則セットを排他群ごとの列挙型で表し、着手規則と対局規則の2層に分ける](represent-rule-sets-as-algebraic-data-types.md)
- 2026年8月26日 [教師値の探索は深さではなくノード数で打ち切る](use-fixed-node-searches-for-teacher-values.md)
- 2026年8月26日 [評価関数に王と2駒の組の表（KPP）を使わない](omit-kpp-tables-from-evaluation.md)
- 2026年8月26日 [詰み帯、捕獲または成りの最善手、および対局内の再出現の局面を教師データに記録しない](exclude-mate-tactical-and-repeated-positions-from-teacher-data.md)
- 2026年8月29日 [教師データの自己対局にランダム着手を注入しない](skip-random-move-injection-in-teacher-games.md)
- 2026年9月1日 [同時対局数の既定値を物理コア数から計算する](derive-default-concurrency-from-physical-cores.md)
- 2026年9月1日 [機能採否はSTCで選別し、LTCだけで採用を決める](screen-at-stc-and-decide-at-ltc.md)
- 2026年9月4日 [GSPRTの対立仮説H1を10 Eloにする](set-gsprt-h1-to-10-elo.md)
- 2026年9月4日 [LTCの加算を0.2秒にする](set-ltc-increment-to-200-ms.md)
- 2026年9月4日 [次の反復の完了時刻を、累積経過時間の固定倍で予測する](predict-next-iteration-by-fixed-ratio.md)
- 2026年9月4日 [残り手数の見積りを手数の関数にし、softをhardで抑える](estimate-moves-to-go-from-ply.md)
- 2026年9月4日 [精度を損なう測定の短縮案を採らない](reject-measurement-shortcuts-that-lose-accuracy.md)
- 2026年9月5日 [SEEは獅子規則に依存する交換段階だけを判定不能とする](mark-only-lion-rule-dependent-exchanges-unknown-in-see.md)
- 2026年9月5日 [静止探索に専用の深さ上限を置かない](omit-quiescence-depth-limit.md)
- 2026年9月6日 [null moveのverification searchを実装しない](omit-null-move-verification-search.md)
- 2026年9月6日 [浅いノードの前向き枝刈りは王駒に利きが届くノードでは行わない](skip-static-eval-pruning-when-a-royal-is-attacked.md)
- 2026年9月7日 [LMRの減深量は対数表に3段階のhistory補正を加えて求める](reduce-lmr-by-log-table-with-stepped-history-adjustment.md)
- 2026年9月7日 [静かな手のhistoryはbutterfly表だけで引く](keep-only-the-butterfly-history-table.md)
- 2026年9月8日 [探索用駒価値は学習PSTから導いた値に固定する](derive-search-piece-values-from-the-learned-pst.md)
- 2026年9月8日 [線形の評価の2組の重みを盤上総駒数で補間する](interpolate-the-linear-path-by-total-piece-count.md)
- 2026年9月12日 [aspiration windowsは外れた側だけを広げ、fail-high時に浅く読み直さない](widen-only-the-failed-side-of-aspiration-windows.md)
- 2026年9月12日 [反復の延長信号を使わず、最善手が安定したときだけ早く止める](stop-early-on-stable-best-move-without-extensions.md)
- 2026年9月12日 [置換表の記録手がないノードを1だけ浅く読む](reduce-depth-by-one-without-tt-move.md)
- 2026年9月12日 [置換表を1スロット1エントリの直接マップのまま保つ](keep-transposition-table-direct-mapped.md)
- 2026年9月12日 [静的評価を置換表に保存しない](do-not-store-static-eval-in-tt.md)
- 2026年9月14日 [学習する評価関数の出力Kを初期値の重みファイルの値に固定し、データから推定し直さない](fix-the-output-k-of-trained-weights.md)
- 2026年9月14日 [教師データの自己対局の手数上限を4,000手にする](cap-teacher-games-at-4000-plies.md)
- 2026年9月14日 [駒除去差分の符号を保たせる片側絶対値の補助損失を学習に加える](penalize-wrong-signed-piece-removal-differences.md)
- 2026年9月15日 [プロファイル誘導最適化（PGO）でビルドしない](do-not-build-with-pgo.md)
- 2026年9月15日 [静止探索の捕獲は、取る駒の価値の群ごとに駒別の利きから生成する](generate-qsearch-captures-by-value-group-per-piece.md)
- 2026年9月17日 [releaseビルドのコード生成単位を1つにし、パニックは巻き戻しのままにする](build-release-with-one-codegen-unit.md)
- 2026年9月17日 [効果が測定の幅に収まった、または数%以下と見込まれた生成とSEEの高速化を実装しない](skip-low-yield-movegen-and-see-speedups.md)
- 2026年9月17日 [探索内の反復検出は対局履歴と探索経路の線形走査で行う](check-repetition-by-linear-scan.md)
- 2026年9月17日 [置換表の行を先読みしない](do-not-prefetch-transposition-table-entries.md)
- 2026年9月17日 [静止探索は置換表の照会より先にstand-patと候補の有無を判定し、獅子の経由升捕獲から空升へ進む手を読まない](check-tt-after-stand-pat-and-trim-lion-captures-in-qsearch.md)
- 2026年9月18日 [序盤の係数は予算式の秒読みの項にだけ掛ける](scale-only-the-byoyomi-term-in-the-opening.md)
- 2026年9月18日 [持ち時間を後半の手のために温存せず、序盤から分担して使う](spend-main-time-from-the-opening.md)
- 2026年9月18日 [直前局面は逆向きに構成した候補を順方向の合法手生成器で検証して求める](verify-predecessor-candidates-with-forward-movegen.md)
- 2026年9月18日 [秒読みのない時計では、予算を分担そのものにして途中結果で指す時間管理を採らない](reject-fair-share-budget-with-partial-results.md)
- 2026年9月19日 [ノード数は着手を適用する直前に1つずつ共有カウンタへ数える](count-nodes-per-move-on-shared-counter.md)
- 2026年9月21日 [`position`の着手列は、前回受理した列との差分だけを適用する](apply-position-by-token-difference.md)
- 2026年9月21日 [correction historyは駒種別の枚数を鍵にし、futilityの判定にだけ使う](key-correction-history-by-material.md)
- 2026年9月21日 [予想手は採用結果の読み筋の2手目だけから取り、相手番の局面は先読みしない](take-ponder-move-from-the-pv.md)
- 2026年9月21日 [先読みが的中したら同じ探索を続け、時間予算を的中の時点から測る](continue-search-and-measure-budget-from-ponderhit.md)
- 2026年9月21日 [浅い非PVノードでSEEの損が余裕値を超える捕獲手を展開しない](prune-losing-captures-by-see-at-shallow-nodes.md)
- 2026年9月21日 [獅子の2段階の非捕獲移動だけを浅く読む減深を実装しない](omit-lion-two-step-quiet-move-reduction.md)
- 2026年9月21日 [発動率の低い探索手法を実装しない](skip-low-activation-search-techniques.md)
- 2026年9月22日 [SPSAの結果は係数表の既定値のリテラルだけを書き換えて反映する](apply-spsa-results-by-rewriting-default-literals.md)
- 2026年9月23日 [升ごとの利き数の表を差分更新で持たず、利きに基づく評価特徴を入れない](omit-the-incremental-attack-count-table.md)
- 2026年9月23日 [教師の探索値を同じ対局の将来の探索値の幾何加重平均に置き換える](use-lookahead-teacher-values.md)
- 2026年9月23日 [教師データを来歴ファイルで分類し、MNSDの形式は変えない](classify-teacher-data-by-provenance-files.md)
- 2026年9月23日 [深い探索の付け直し、人間の棋譜の局面、および棋譜から始めた自己対局を教師に加えない](keep-deep-rescoring-and-human-games-out-of-the-teacher.md)
- 2026年9月25日 [SPSAの値は1組の候補として段階ゲートで採否を決め、調整はSTCで行う](adopt-spsa-values-only-through-the-staged-gate.md)
- 2026年9月25日 [SPSAの反復はθ+とθ−の8ペアの直接対局とし、確率的な丸めと非同期の更新を使う](play-theta-plus-against-theta-minus-in-eight-pair-iterations.md)
- 2026年9月25日 [SPSAは対局測定とは別のコマンドで回し、対局の実行部をライブラリで共有する](run-spsa-as-a-separate-command.md)
- 2026年9月25日 [投了はUSI層が採用結果の評価値で単発に判定する](decide-resignation-in-the-usi-layer.md)
- 2026年9月25日 [棋力測定の対局ではエンジンの投了を無効にする](disable-resignation-in-strength-measurements.md)
- 2026年9月25日 [秒読みの項の係数と時計の安全余裕はSPSAで調整しない](exclude-byoyomi-terms-and-safety-margin-from-spsa.md)
- 2026年9月25日 [調整する係数は調整用ビルドでだけ外から変えられるようにする](expose-tunable-coefficients-only-in-tuning-builds.md)
- 2026年9月26日 [PST全体を学び直すときは、教師の混合比を0.75に保つ](keep-the-mixing-ratio-at-0-75-for-pst-retraining.md)
- 2026年9月26日 [SPSAの利得は範囲の1/6の摂動幅まで減衰させ、標準の規模を3,000ペアにする](decay-spsa-gains-from-one-sixth-of-the-range.md)
- 2026年9月26日 [モジュールを関心事ごとのファイルに分け、非公開の欄を使える範囲をディレクトリの親子で表す](split-modules-by-concern-and-scope-privacy-by-nesting.md)
- 2026年9月26日 [探索中に静的評価を呼んだ局面を学習の局面に混ぜない](do-not-train-on-search-tree-positions.md)
- 2026年9月26日 [移動した項目を旧パスで再公開しない](do-not-re-export-moved-items-at-old-paths.md)
- 2026年9月26日 [近傍の2駒の相対位置の表による評価の補正を実装しない](skip-relative-piece-pair-correction.md)
- 2026年9月27日 [サニタイザとMiriを検証の道具に採らない](do-not-adopt-sanitizers-or-miri.md)
- 2026年9月27日 [プロトコルの入出力の記録を起動引数`--io-log`で有効にする](record-protocol-io-with-a-launch-argument.md)
- 2026年9月27日 [局面、評価、置換表の表示をUSIの独自コマンドにする](show-position-evaluation-and-tt-as-usi-commands.md)
- 2026年9月27日 [探索統計を既定で無効のフィーチャ`search-stats`で数え、固定の項目だけを常設する](count-search-statistics-behind-a-feature.md)
- 2026年9月27日 [重い整合検査をフィーチャ`invariants`に置き、`cargo test`では常に有効にする](enable-invariant-checks-in-tests-by-feature.md)
- 2026年9月28日 [null moveと置換表の打ち切りをPVノードでも行う](allow-null-move-and-tt-cutoff-at-pv-nodes.md)
- 2026年9月28日 [反復の先読み（upcoming repetition detection）を採らない](reject-upcoming-repetition-detection.md)
- 2026年9月29日 [null moveより後の局面だけで反復を検出する](detect-repetition-only-after-latest-null-move.md)
- 2026年9月29日 [王駒がすべて狙われた局面でも静止探索でstand-patを使う](use-stand-pat-when-royals-are-attacked.md)
- 2026年9月29日 [静止探索のdelta pruningに成り益を加えない](keep-delta-pruning-without-promotion-gain.md)
- 2026年9月30日 [静止探索の出力を回帰する学習と、捕獲局面を加える学習を採らない](do-not-regress-quiescence-search-outputs.md)
- 2026年10月1日 [PSTの学習に浅い探索の手の順位の損失を加えない](reject-move-rank-loss-in-pst-training.md)
- 2026年10月2日 [PSTに手作りの局所2駒表の補正項を加えない](reject-local-two-piece-correction-for-pst.md)
- 2026年10月4日 [crateごとに版数を付けてリリースし、crates.ioへの公開は手で行う](release-each-crate-separately-and-publish-by-hand.md)
- 2026年10月4日 [improvingはfutilityの余裕値を縮めることにだけ使う](use-improving-only-for-futility-margins.md)
- 2026年10月4日 [late move pruningは静かな手だけを数え、履歴の条件を付けない](count-only-quiet-moves-for-late-move-pruning.md)
- 2026年10月4日 [ライブラリをMIT、エンジンをGPL-3.0-or-laterで配布する](license-library-under-mit-and-engine-under-gpl.md)
- 2026年10月4日 [リポジトリのルートを仮想マニフェストにし、crateを`crates/`の下に並べる](arrange-crates-under-a-virtual-workspace.md)
- 2026年10月4日 [単独で不採用だった探索の8項目を、SPSAで再調整して一括で採用する](adopt-revived-search-items-as-a-bundle.md)
- 2026年10月4日 [規則と表記をライブラリ`minase-core`に置き、探索と評価をエンジン`minase`に置く](split-rules-library-from-engine.md)
- 2026年10月4日 [静止探索は実際に読んだ捕獲手の数で打ち切る](limit-searched-captures-in-qsearch.md)
- 2026年10月5日 [再開は実行条件を`manifest.json`だけから読み、runnerと測定機を照合しない](resume-from-the-manifest-without-checking-the-runner.md)
- 2026年10月5日 [補助ツールを実行ファイル`minase`のサブコマンドにし、エンジンはサブコマンドなしで起動する](run-developer-tools-as-engine-subcommands.md)
- 2026年10月5日 [評価関数の候補は探索係数を調整し直さずに採否を測る](judge-evaluation-candidates-without-retuning-search-coefficients.md)
- 2026年10月10日 [butterfly historyと捕獲履歴を対局内で減衰させて持ち越す](carry-history-tables-within-a-game.md)
- 2026年10月10日 [lishogiのBotの`ByoyomiMargin`を3,000 msにする](set-bot-byoyomi-margin-to-3000-ms.md)
- 2026年10月10日 [先頭の捕獲手だけをSEEで判定し、損ならkiller手を先に返す](try-killers-before-a-losing-first-capture.md)
- 2026年10月10日 [捕獲履歴の寄与を歩兵価値の定数倍に抑え、静止探索の順序にも使う](bound-capture-history-and-use-it-in-qsearch.md)
- 2026年10月10日 [根の2手目以降を前回反復の部分木ノード数で並べる](order-root-moves-by-previous-subtree-nodes.md)
- 2026年10月10日 [静かな手の順序を減点つきの重力式と線形経路の差分の同点破りで決める](order-quiets-with-malus-and-pst-tiebreak.md)
- 2026年10月12日 [NNUEの入力特徴集合に王の位置で条件付けないP型を使う](use-p-type-input-features-for-the-nnue.md)
- 2026年10月12日 [NNUEの出力を、PSTを移した線形の経路と隠れ層のない直接内積の和にする](compose-the-nnue-of-a-pst-path-and-a-direct-screlu-head.md)
- 2026年10月12日 [NNUEの学習器は第1層の勾配をホストで作る逆索引から固定の順で足し、アトミック加算を使わない](backpropagate-the-first-layer-through-a-reverse-index.md)
- 2026年10月12日 [NNUEの学習器をRustのホストとNVRTCで実行時にコンパイルするCUDA C++のカーネルで書く](write-trainer-kernels-in-cuda-cpp-via-nvrtc.md)
- 2026年10月12日 [NNUEは探索値だけの先読み教師と二値交差エントロピーで学ぶ](train-the-nnue-on-search-value-lookahead-teachers.md)
- 2026年10月12日 [NNUEを入力特徴集合と出力部の型引数で組み立てる](parameterize-the-nnue-by-feature-set-and-head-types.md)
- 2026年10月12日 [P型のNNUEは左右鏡映の対で第1層の重みを半幅だけ回して共有する](share-p-type-mirror-weights-by-half-width-rotation.md)
- 2026年10月12日 [`unsafe`は学習器のクレートだけに許し、エンジンとライブラリでは禁じ続ける](allow-unsafe-only-in-the-trainer-crate.md)
- 2026年10月12日 [学習器のAdamWはPyTorchと同じバイアス補正つきの式にする](use-pytorch-adamw-with-bias-correction.md)
- 2026年10月12日 [探索が使う評価関数をNNUEの1つにし、PSTとFMのコードは比較用に残す](use-nnue-as-the-only-search-evaluation.md)
- 2026年10月13日 [PST、FM、および同時学習の学習器はCPUで動かし、測定なしにGPUのカーネルを書かない](train-pst-and-fm-on-the-cpu.md)
- 2026年10月13日 [Pythonの学習器は一度だけ照合した後に撤去し、Rustの学習器だけを正とする](retire-the-python-trainer-after-a-one-time-cross-check.md)
- 2026年10月13日 [学習器は評価関数ごとに独立したパイプラインを持ち、データの読み込みと汎用の部品だけを共有する](split-training-pipelines-by-evaluation-function.md)
- 2026年10月13日 [設計書を持つ作業はすべてROADMAPの状態表に載せる](list-every-planned-work-in-the-roadmap.md)
- 2026年10月13日 [教師データの由来をMNSDのレコードに置き、来歴ファイルと全体の検査和をやめる](record-the-origin-in-mnsd-records.md)
- 2026年10月13日 [各ファイル形式の版を1つにし、過去のファイルを読み続けない](keep-one-version-per-file-format.md)
- 2026年10月13日 [学習器のチェックポイントをNPZに置き、設定そのものを収めて比べる](store-trainer-checkpoints-as-npz.md)
- 2026年10月14日 [教師データを対局単位の連鎖で記録するMNSDの1形式にまとめる](record-teacher-data-as-chained-games.md)
- 2026年10月14日 [教師データの着手を整列した合法手の番号で表し、方策を量子化した確率で持つ](index-moves-into-sorted-legal-moves-and-store-policies-as-probabilities.md)
- 2026年10月14日 [詰み帯、捕獲または成りの最善手、および対局内の再出現の局面を、除外の理由を付けて教師データに記録する](record-excluded-positions-with-a-reason.md)
- 2026年10月14日 [`unsafe`は学習器のGPUの処理だけに許し、エンジンとライブラリでは禁じ続ける](allow-unsafe-only-in-the-trainer-gpu-code.md)
- 2026年10月16日 [NNUEの学習率をcosine annealingで減らす](anneal-the-nnue-learning-rate-with-a-cosine-schedule.md)
