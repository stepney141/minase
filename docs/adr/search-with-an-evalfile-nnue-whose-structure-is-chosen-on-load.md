# 探索は、EvalFileで読んだNNUEだけを使い、その構造を読み込みのときに選んで、評価器の列挙型で着手ごとに分岐する

## ステータス

承認済み。決定日は2026年11月19日。[探索が使う評価関数を、USIのオプションEvalFileで指定した外部のファイルのNNUEの1つにし、PSTとFMのコードは比較用に残す](archive/use-an-nnue-read-from-evalfile-as-the-only-search-evaluation.md)を置き換える。

## コンテキスト

[以前の決定](archive/use-an-nnue-read-from-evalfile-as-the-only-search-evaluation.md)は、探索が使う評価関数をNNUEだけとし、ネットを実行ファイルに埋め込まず、USIのオプション`EvalFile`で指定した外部のファイルを読むとした。
同じ決定は、どのコミットでも探索が使う評価関数を1つに保ち、ネットの構造はコンパイル時の型の別名で1つに固定していた。
[入力特徴集合と構造を列挙型で表すADR](represent-nnue-feature-sets-and-structures-as-enums-and-select-the-structure-from-the-tatara-file.md)で、P型のLayerStackとHalfKP_vmのSimpleを同じコミットで扱うことになり、探索は読み込んだネットの構造に合わせて評価器を選ぶ必要が生じた。
tataraと同じ作者の将棋エンジンrshogiは、ネットと累算器の組を1つの列挙型`SearchEvaluator`で持ち、着手、着手の取り消し、および評価のたびに列挙子を照合して分岐する（コミット`a8e2b6b`の`crates/rshogi-core/src/nnue/search_evaluator.rs`）。
決定は[tatara化とHalfKP_vmの設計書](../plans/archived/nnue-tatara-framework-and-halfkp-vm.md)で下した。

## 決定

探索が使う評価関数はNNUEだけとし、ネットは実行ファイルに埋め込まない。
αβ探索は、`EvalFile`（CECPでは`option EvalFile=`）で受け取ったパスのファイルを、USIでは`isready`で、CECPでは`new`か探索の開始で読み、ファイルの構造記述からLayerStackかSimpleかを選ぶ。
既定のネットは持たず、`EvalFile`が未設定か、ファイルを読めないか、形式の検証に通らなければ、エラーを出して終了コード1で止まる。
`setoption name EvalFile`はパスを保存し、読み込み済みのネットと、前のネットの評価に基づく置換表およびhistory表を捨てる。
探索の評価器は、rshogiに倣って、ネットと、探索の根から深さごとに並べた累算器の組を1つの列挙型`SearchEvaluator`で持ち、着手、null move、および評価で列挙子を照合する。
HalfKP_vmのネットで探索するときは、探索の根で、探索の間に基準の王駒（王将、なければ太子。[入力特徴集合のADR](provide-p-and-halfkp-vm-input-features-and-drop-halfkp-hm-b8.md)）が2枚以上になり得ないことを確かめ、満たさなければ探索を始めずにエラーを返す。
もう1つの探索方式であるPV-MCTSの勝率尺度は、USIのオプション`NnCpPerLogit`で受け取り、NNUEのネットから決めない。
PST単独、PST＋FM、および駒得評価v0のコードは比較用に残し、探索からは呼ばない。

## 検討した代案

探索全体を評価器の具体型ごとに単相化し、探索の入口で1回だけ分岐する案は、速さの問題が測られる前に入れる最適化であり、測定なしの最適化を禁じる方針に反するので採らなかった。
載せ替える前と後のP型の`bench`の速さを比べ、列挙による分岐が目立つと確かめられた場合にだけ検討する。
ネットの埋め込みを続ける案は、HalfKP_vmの約494 MBのネットをgitにも実行ファイルにも置けないので採らなかった。
読み込みに失敗したときに前のネットで続ける案は、安全に続けられないときは止めるという方針に反するので採らなかった。
PV-MCTSの勝率尺度をNNUEの`fv_scale`から決める案は、1つの値のためにPV-MCTSの実行の条件をNNUEのネットに依存させるので採らなかった。
駒得評価v0、PST、NNUEを探索の中で切り替える案は、評価関数の比較をコミット対コミットの対局で行う方針に反するので採らなかった。
探索の中で切り替えるのは、同じNNUEの構造の違いだけである。

## 結果

評価器の列挙と各操作は`crates/minase/src/eval/nnue/network/evaluator.rs`に、読み込みと`EvalFile`の扱いは`crates/minase/src/search/alphabeta/engine.rs`と`crates/minase/src/protocol/`のdocコメントにある。
静かな手（駒を取らない手）の並べ替えの同点破りは、P型ではPSQTの和の変化（着手した側の視点の変化から相手側の視点の変化を引いた値）を使い、Simpleでは使わずに手の番号順を保つ。
これは[静かな手の並べ替えのADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)の同点破りの線形の経路を、PSQTに読み替えたものである。
HalfKP_vmが評価できないのは、基準の王駒が0枚の視点と2枚以上の視点である。前者は最後の王駒を取る手を評価せずに詰みの値として扱うことで、後者は根の検査で避けるので、探索がHalfKP_vmの評価できない局面に当たることはない。
探索の速さは、載せ替える前と後のP型で比べて記録した（[測定記録](../measurements/nnue-tatara-framework-and-halfkp-vm/)）。
