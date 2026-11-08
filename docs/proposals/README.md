# 提案と議論の索引

本ディレクトリは、minaseの方式についての提案と、その検討の議論を収める。
提案は、[調査メモ](../research/README.md)が確かめた外部の事実や、[監査報告](../audits/)が記したある時点のminaseの診断を根拠に、候補と試す順序を示す。
採否は提案の中では決めず、[設計書](../plans/README.md)の測定か[ADR](../adr/README.md)で決める。
表題に「調査」を含む文書でも、minaseへの採用の候補や試す順序が主題なら本ディレクトリに置く。

採否を待つ提案は本ディレクトリの直下に置き、根拠や前提が変わったら本文を直す。
候補の採否がすべて決まったか、前提が成り立たなくなった提案は、[archived/](archived/)へ移して凍結する。
移すときは、冒頭に短い段落を置いて決着の内容と、それを決めた設計書、ADR、または測定記録へのリンクを書き、本書の表の行を「決着した提案」の表へ移して「決着」の列を埋める。

本書の表は、作成日（初回のコミット日）の古い順に並べる。
提案に含まれる外部の事実が独立した調査として参照される場合は、その部分を調査メモとして分けて書く。

## 採否を待つ提案

| 作成日 | 文書 | 内容 |
|---|---|---|
| 2026年10月28日 | [やねうら王の合法手生成の構造をMCTSの葉の全件生成へ応用する調査](yaneuraou-movegen-for-leaf-legal-moves.md) | 審判層の削減後にMCTSの探索の命令数の37.9%を占める`Game::legal_moves`の全件生成について、費用の6割が利きの計算ではなく着手ごとの成りと獅子の捕獲制限の判定にあることをプロファイルで示し、やねうら王が成りの可否を駒ごとの集合演算で決めて着手ごとに盤面を読まない構造から、成りの判定の集合化、獅子の捕獲制限の検査の限定、および出力バッファの再利用の3候補と、過去に棄却した候補E2との関係を整理する。 |
| 2026年11月2日 | [AlphaZero型探索がαβ探索に大敗する原因とDL系将棋ソフトから移せる点の調査](alphazero-strength-gap.md) | サイクル32の2勝192敗を学習量、探索回数、およびネットの大きさに分けて先行例と比べ、王駒を取られる手を避けきれない盲点と自己対局の外の局面での価値の過大評価を示し、dlshogi、ふかうら王、AobaZero、KataGoなどの技法を費用と効果の報告とともに並べる。詳細は[規模](alphazero-strength-gap/scale.md)、[探索](alphazero-strength-gap/search.md)、および[価値](alphazero-strength-gap/value-bias.md)の3本にある。 |
| 2026年11月8日 | [AlphaZero型探索の自己対局の局面の分布を広げる対策の候補](alphazero-selfplay-distribution.md) | ネットがαβとの対局の局面で価値を読み違える問題について、初期配置の組み替え、評価が割れる局面の採取、探索回数に差をつけた自己対局などの候補を既定の決定との関係とともに並べ、開始局面の決定、系列の費用、着手の時期、および判定の物差しを、設計書を起案する前に決めることとして示す。 |

## 決着した提案

| 作成日 | 文書 | 内容 | 決着 |
|---|---|---|---|
| 2026年9月15日 | [合法手生成の高速化計画への追加候補](archived/movegen-speedup-ideas.md) | 2026年9月13日時点の高速化計画に対して、静止探索で不要な捕獲を生成前に除く案と段階的に生成する案を、処理件数の診断から提案する。 | 2026年9月15日に[plans/archived/movegen-speedup.md](../plans/archived/movegen-speedup.md)で、生成前の除外、段階生成、およびSEEの早期終了を採用した。 |
| 2026年9月21日 | [12×12盤へのmagic bitboardの適用](archived/magic-bitboard-feasibility.md) | 斜線のmagicと横線の小表の試作がいずれも算術方式を上回らなかった結果を記録する。付属の計算スクリプトは[magic-bitboard-feasibility/](archived/magic-bitboard-feasibility/)にある。 | 2026年9月21日の[試作の測定](../measurements/adhoc/magic-bitboard-prototype.md)で不採用とした。 |
| 2026年9月22日 | [評価項目の追加候補の調査](archived/evaluation-terms-survey.md) | [lishogi対局yO464dzlの敗因](../audits/lishogi-game-yO464dzl-loss-analysis-2026-09-22.md)を受けて、王の遮蔽と開いた筋から王基準の関係表までの評価項目を導入の優先順に10段階へ分ける。 | 2026年9月23日に[plans/archived/strength-stage9.md](../plans/archived/strength-stage9.md)で上位7項目をいずれも採用しなかった。 |
| 2026年9月23日 | [評価項目の自己対局による係数学習](archived/evaluation-terms-selfplay-feasibility.md) | 評価項目の係数を自己対局結果から学習する定式化と、既存データからの識別可能性を検討する。 | 2026年9月23日に[plans/archived/strength-stage9.md](../plans/archived/strength-stage9.md)で項目1を見送り、最初の比較が成り立たなくなった。 |
| 2026年9月25日 | [探索で使う局面に合わせた評価関数の学習](archived/search-aware-evaluation.md) | 過去のNNUEの不採用を踏まえ、候補自身が選ぶ局面の再評価を先に検証し、利きの補助学習とモデル構造を条件付きで比較する案を示す。 | 2026年9月26日に[plans/archived/search-aware-evaluation.md](../plans/archived/search-aware-evaluation.md)で不採用とした。 |
| 2026年9月27日 | [評価関数を改善する3つの観点](archived/evaluation-improvement-strategy.md) | 評価モデルの表現、教師局面と教師値、学習目標と更新方法ごとに、確認された問題、有望な方法、次の検証を整理する。 | 2026年10月2日に、次の検証として起案した3本の設計書がいずれも不採用で完了した。 |
| 2026年9月27日 | [評価関数の表現と更新費用の候補](archived/evaluation-architecture-candidates.md) | 局所2駒表、低ランク分解、王駒の射線表、小型NNUEの入力、数式、容量、差分更新を具体化する。 | 2026年10月2日に[plans/archived/relational-correction.md](../plans/archived/relational-correction.md)で不採用とした。 |
| 2026年10月8日 | [fearless_simdによる安全なSIMDの適用可能性](archived/fearless-simd-feasibility.md) | `unsafe`なしで実行時に命令集合を選ぶRustのSIMDライブラリfearless_simd 1.1.0が当時のminaseを速くできるかを、FMの4つの核の機械語の読解と移植可能ビルドとの速度比較で見積もり、標準ビルドでは自動ベクトル化が既に同じ命令を出しているため試作を見送る。 | 2026年10月8日に試作を見送り、隠れ層の推論も自動ベクトル化で足りた。 |
| 2026年10月8日 | [NNUEの再挑戦に向けたアーキテクチャの考察](archived/nnue-architecture-discussion.md) | P型NNUEの2回の不採用を記録から読み直し、敗因は入力特徴ではなく教師と学習手順にあると判定したうえで、PST＋FMを完全に置き換える単一ネットの構成と、NNUE自身を生成器にした世代反復で強くする計画の全体像、先例の一次資料、および自己対局測定の偏りへの疑いを記す。 | 2026年10月12日に[plans/archived/nnue-replacement.md](../plans/archived/nnue-replacement.md)で初代のネットとして採用し、その後の構成の見直しで置き換えた。 |
| 2026年10月9日 | [手の順序付けの改善余地に関する調査と議論](archived/move-ordering-discussion.md) | 当時の順序付けでβカットの遅れが件数では置換表の記録手がない残り深さ1のノードに集中する一方、初手βカット率ではノード数の損失を測れないことを示し、Claudeとcodexの議論を経た8候補の順位とStockfishの方式を記す。 | 2026年10月10日に[plans/archived/move-ordering.md](../plans/archived/move-ordering.md)で7候補を採用した。 |
| 2026年10月12日 | [NNUEの13,680入力を採る根拠と限界](archived/nnue-input-features.md) | 入力が保持する情報と有限幅での学習を区別し、チェスや将棋の入力との特徴数の差が主に王の位置の掛け合わせによることを示し、既存の教師データで太子の頻度、自玉の升の分布、および自玉が動く着手の割合を数えて、王の位置を掛けたHalfKP型が太子とデータ量の面では成り立つことを確かめたうえで、既存の測定とStockfish、やねうら王、Fairy-Stockfishの一次資料から、採用理由と未検証事項を整理する。 | 2026年10月12日に[ADR use-p-type-input-features-for-the-nnue](../adr/use-p-type-input-features-for-the-nnue.md)でP型を採った。 |
| 2026年10月28日 | [やねうら王の王手判定と合法手生成の使い分けを審判層の裁定へ応用する調査](archived/yaneuraou-check-detection-for-referee.md) | やねうら王が探索の中で合法手の全件生成を避け、1手の合法性を1手の検査で、王手の有無を利きの逆引きで、連続王手の千日手を整数のカウンタで判定する方法をソースで確かめ、審判層の着手の合法性検査とR1の攻撃的着手の判定を動かした駒1枚の生成と利きの逆引きへ置き換える根拠と、駒枯れの猶予の判定を王駒1枚の生成へ置き換える根拠、および合法手なしと詰みの判定を駒ごとの生成で打ち切る案を示す。 | 2026年10月28日に[plans/archived/referee-adjudication-movegen.md](../plans/archived/referee-adjudication-movegen.md)で5件とも採用した。 |
