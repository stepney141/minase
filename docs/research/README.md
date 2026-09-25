# 調査メモの索引

本ディレクトリは、外部資料、他エンジンのソースコード、および保存棋譜を調べた調査メモを収める。
各文書は結論を先に書いて一次資料へリンクし、設計書が方式の根拠として参照する。
本書は1行1文書の索引であり、新しい調査メモを追加したときは該当する節へ行を加える。

## 棋力向上手法の全般

個別の設計書に入る前の段階で、未導入の手法を横断的に洗い出した調査である。

- [minaseに未導入の棋力向上手法の調査](unplanned-strength-techniques-survey.md)は、チェスと将棋のエンジンで効果が測定され、minaseのソースにも設計書にも現れない手法50件を、導入の見込みとともに列挙する。

## 探索

探索の各機構について、StockfishやYaneuraOuなどの実装を一次資料で照合した調査である。

- [前向き枝刈りの第3層に関する既存実装の調査](forward-pruning-prior-art.md)は、棋力向上段階8の4項目（correction history、improving、SEE pruning、history pruning）の更新条件と閾値を、StockfishとYaneuraOuのソースで確認する。
- [探索内の反復の扱いに関する既存実装の調査](search-repetition-prior-art.md)は、13本の実装が探索木の中で同一局面の再現を検出して返す値と、置換表との折り合いの付け方を比較する。
- [探索ノード数の定義と既存エンジンとの照合](node-count-definition.md)は、minaseが末端局面を二重に数えていたことを示し、Stockfishとやねうらおうに合わせたノード数の定義の根拠を記す。
- [SPSAで調整した係数をソースへ反映する先行例](spsa-parameter-application.md)は、やねうら王、fishutils、およびapeironが調整結果をソースの定数へ自動転記する方法を確認する。
- [SPSAの対局数を減らす手法の調査](spsa-acceleration-methods.md)は、雑音のある零次最適化の収束の下界と、2SPSA、Adam、CLOP、ベイズ最適化などの改良手法を一次資料で比べ、minaseの雑音の水準では摂動幅と学習率の較正に改善の余地があると結論する。
- [SPSAの反復予算の決め方](spsa-iteration-budget.md)は、対局予算から反復数を決め、停止後に原因と採否を分けて扱う方法をまとめる。
- [チェスエンジンのSPSA調整で使われた対局数](chess-engine-spsa-session-sizes.md)は、StockfishとOpenBench上の公開記録から係数の数と実績局数を集める。
- [AlphaZero型の深層強化学習の先行実装調査](alphazero-prior-art.md)は、lc0とdlshogiを中心に、学習アルゴリズム、MCTSの実装、Rustからの推論ランタイム、および複数の探索方式の共存方法を調べる。

## 評価関数と教師データ

評価関数の構成、追加する評価項目、および学習に使う教師の作り方を扱う調査である。
評価項目の3文書は、項目の候補、計算の土台、係数の学習方法の順に読むとつながる。

- [局面の進行度に応じた駒位置評価](tapered-pst.md)は、PSTを序中盤用と終盤用に分けて補間する方式の先例を、HaChuを含む一次資料で確認する。
- [評価項目の追加候補の調査](evaluation-terms-survey.md)は、lishogi対局yO464dzlの敗因を受けて、王の遮蔽と開いた筋から王基準の関係表までの評価項目を導入の優先順に10段階へ分ける。
- [評価項目が依存する土台の調査](evaluation-infrastructure-survey.md)は、先行エンジンが評価項目をどのデータ構造の上でどの時点の費用として得ているかを調べ、段階9でNPSが82.2%下がった原因と照合する。
- [評価項目の自己対局による係数学習](evaluation-terms-selfplay-feasibility.md)は、評価項目の係数を自己対局結果から学習する定式化と、既存データからの識別可能性を検討する。
- [手作り評価項目の係数を自己対局から学ぶ先行研究](handcrafted-evaluation-selfplay.md)は、Texelと、BealとSmithの1999年の論文を中心に、手作りの評価項目の係数を自己対局から学ぶ先例を確認する。
- [チェス評価関数の教師と混合比](chess-evaluation-targets.md)は、教師評価値と対局結果の混合比について、Stockfish、Texel、Ethereal、およびViridithasの設定と比較記録を調べる。
- [将棋の教師生成と学習ツールの調査（GenSfenとtatara）](teacher-generation-prior-art.md)は、やねうら王のGenSfenとNNUE学習ツールtataraのソースから、将棋の教師の質を作る要素を整理する。
- [探索で使う局面に合わせた評価関数の学習](search-aware-evaluation.md)は、過去のNNUEの不採用を踏まえ、候補自身が選ぶ局面の再評価を先に検証し、利きの補助学習とモデル構造を条件付きで比較する案を示す。
- [複数駒の関係と探索結果を学ぶ評価関数の一次資料](relational-evaluation-primary-sources.md)は、Stockfishとやねうら王の関係特徴、探索内部を教師にする研究、および学習時だけ高価な特徴を使う方法の根拠と限界を確認する。

## 合法手生成の高速化

合法手生成と利きの計算を速くする案について、処理件数の診断と一次資料をまとめた調査である。

- [合法手生成の高速化計画への追加候補](movegen-speedup-ideas.md)は、2026年9月13日時点の高速化計画に対して、静止探索で不要な捕獲を生成前に除く案と段階的に生成する案を、処理件数の診断から提案する。
- [Magic bitboardの一次資料](magic-bitboard-primary-sources.md)は、Stockfish、やねうら王、およびFairy-Stockfishの利きの求め方を調べ、12×12盤では方向ごとに表を分割する方式を先に比較する価値があると示す。
- [12×12盤へのmagic bitboardの適用](magic-bitboard-feasibility.md)は、斜線のmagicと横線の小表の試作がいずれも現行の算術方式を上回らなかった結果を記録する。付属の計算スクリプトは[magic-bitboard-feasibility/](magic-bitboard-feasibility/)にある。

## 時間管理と投了

持ち時間の配分と投了の判断を、他エンジンの実装と保存済み対局記録から調べた調査である。

- [秒読みつき時間制御における既存エンジンの持ち時間配分](time-management-byoyomi-survey.md)は、将棋、チェス、囲碁のエンジン7本が持ち時間と秒読みをどう配るかを比較し、持ち時間を温存するエンジンがないことを示す。
- [保存済み対局記録による投了閾値の診断](usi-resignation-diagnosis.md)は、自己対局記録104,224局へ仮想の投了条件を当て、誤投了を0件にできる閾値がないことを示す。

## 対局の分析

lishogiでの実戦と外部エンジンとの対局棋譜から、minaseの弱点を調べた分析である。

- [lishogi対局 yO464dzl の敗因分析](lishogi-game-yO464dzl-loss-analysis.md)は、517手で負けた対局の敗因を、評価関数が王の安全度を測れないことに求める。
- [lishogi対局 WPgmFqSa の時間配分](lishogi-game-WPgmFqSa-time-management.md)は、160手目で本時間を使い切った対局のログから、反復の開始条件と序盤の配分を見直す根拠を示す。
- [HaChuとMinaseの勝ち方・負け方](hachu-minase-playing-style-analysis.md)は、HaChuとの400局の保存棋譜から、両エンジンが勝つ局と負ける局の典型的な経過を比較する。

## 通信プロトコルと外部エンジン

USI、CECP、およびHaChuの仕様調査は[protocols/](protocols/)にまとめ、[protocols/README.md](protocols/README.md)を索引とする。
