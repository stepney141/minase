# 時計と併用しない固定時間の探索も締切まで反復を始め、締切で打ち切った反復の途中結果で指す

## ステータス

承認済み。決定日は2026年10月25日。

## コンテキスト

USIの`go movetime <ms>`で1手の思考時間を固定した探索は、lishogiのBotの通信対局（1手300秒、4スレッド）、Botの通常対局の初手（1,000 ms）、および開発用GUIでUSIのエンジンと指す固定時間の対局で使われる。
従来のこの探索は、softとhardをともに`movetime`としながら、予測した完了時刻が`movetime`に収まるときだけ次の反復を始め（[固定倍の予測のADR](predict-next-iteration-by-fixed-ratio.md)）、`movetime`で打ち切った反復の結果を捨てていた。
Botの通信対局では、minase 2.5.0の探索の47%が300秒で打ち切られ、主ワーカーが最後に反復を完了した時刻は平均192秒だった（[調査メモ](../research/movetime-prior-art-and-correspondence-usage.md)）。

固定倍の予測は、予測が過大でも使わなかった時間が時計に残り、後の手に回ることを前提にしている。
固定時間の探索では使い残した時間が後の手へ回らず、この構造は、使わなかった秒読みを持ち越せない秒読み期と同じである。
秒読み期については、締切まで反復を始め、打ち切った反復の途中結果で指す規則をすでに採っている（[秒読み期の締切と途中結果のADR](search-to-the-byoyomi-deadline-and-play-partial-results.md)）。
StockfishとYaneuraOuも、`movetime`の指定では通常の時間管理を外して指定時間まで探索し、打ち切った反復で読み終えた手を使う。
また、エンジンの開発者は2026年9月18日に、人間との対局で棋力を落とす変更を許容しないと決めている。
標準の採否測定（STCとLTC）は`go movetime`を送らないので、この変更の採否は固定時間の自己対局で測る必要があった。
本書の決定は[固定時間（movetime）の探索の締切と途中結果](../plans/movetime-deadline.md)で下した。

## 決定

時間の制限が`movetime`だけで、時計、深さ、およびノード数の制限を併用しない有限探索（先読みの`go ponder movetime`を含み、先読みでは的中の時刻から数える）では、softとhardをともに`movetime`とし、経過時間がこれに達していなければ次の反復を始める。
この探索では、予測完了時刻による開始の制限、最善手が安定したときの早期終了、および先読みの的中時の打ち切りの予測を適用しない。
根では直前の反復の最善手を最初に読み、`movetime`で反復を打ち切ったときは、秒読み期と同じ規則で途中結果を選んで指す。

規則の詳細は、`crates/minase/src/search/alphabeta/time.rs`の`movetime_deadline`、`uses_partial_results`、および`time_budget`と、`root.rs`の`RootResults`のdocコメントにある。

## 検討した代案

現行の予測による開始の制限を固定時間の探索にも残す案は、後の手へ回らない時間を捨て続けるので採らなかった。

`movetime`を時計、深さ、またはノード数と併用した探索にも同じ規則を適用する案は採らなかった。
Stockfishは併用した制限のいずれかに達した時点で止め、YaneuraOuは`movetime`だけを使うので、先行実装の扱いが分かれている。
また、これらの制限を併用して送るのは開発用GUIのCECPの経路だけである。

`movetime`から`ByoyomiMargin`のような余裕を引く案は、StockfishとYaneuraOuが`movetime`に達した時点で打ち切って余裕を引かず、通信の余裕は呼び出し側（Lishogi-Botの`move_overhead`）が確保するので採らなかった。

StockfishとYaneuraOuは、打ち切った反復の先頭の手が負けの評価値を持つ場合などに前の反復の結果へ戻す。
本書は秒読み期の規則を変えずに適用範囲を広げるので、この巻き戻しは加えなかった。
複数のワーカーの結果を評価値の投票で選ぶ方式も、秒読み期と共有する選択の規則を変えるので採らなかった。
どちらも、必要なら別の設計書で扱う。

## 結果

採否は、1手300 msの選別条件と1手1,000 msの判定条件の自己対局GSPRTで判定した。
選別条件は有効416ペアで`H1`（[movetime-deadline-screen-300](../measurements/movetime-deadline/movetime-deadline-screen-300.md)）、判定条件は有効325ペアで`H1`（[movetime-deadline-decision-1000](../measurements/movetime-deadline/movetime-deadline-decision-1000.md)）となり、どちらも異常は0件だった。
この2つの測定は、測定機が別の計算で埋まっていたので、別の機械（Threadripper 2990WX）で`Threads=1`の対局として行った。
Botの通信対局の4スレッドと1手300秒の条件は測っておらず、ワーカー間の選択の効果も測定に現れない。

持ち時間の時計による探索（秒読みが0の時計と秒読みが正の時計）と、`movetime`を時計、深さ、またはノード数と併用した探索は、予算、反復の開始判定、根の手の順序、および打ち切り時に指す手の選び方が変わらない。
固定深さのbenchの総ノード数も変わらない。
時間予算の真偽値のフィールド`TimeBudget::byoyomi_period`は、秒読み期と固定時間の探索の両方を表すので`deadline`へ改めた。

固定時間の測定のため、対局ハーネスに思考制限`movetime=<ms>,timemargin=<ms>`を加えた（[棋力測定の手引き](../guides/sprt.md)）。
以後、`go movetime`の探索だけに働く変更も、この思考制限の自己対局で採否を測れる。
