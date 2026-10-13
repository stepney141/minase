# SPSAは対局測定とは別のコマンドで回し、対局の実行部をライブラリで共有する

## ステータス

承認済み。決定日は2026年9月25日。

## コンテキスト

`minase match run`は、採否の測定を行う計器である（[コミット対コミットの測定](compare-commits-as-separate-engine-processes.md)）。
SPSAは、同じ調整用ビルドのエンジンに、θ+とθ−の係数を`setoption`で側ごとに与えて対局させる必要がある（[調整用ビルドのADR](expose-tunable-coefficients-only-in-tuning-builds.md)）。
対局の進行、異常の裁定、開始局面の生成、およびコミットのビルドは、`match_runner`が既に持っていた。
fishtestとOpenBenchは調整の管理をサーバーが行い、対局はチェス用の対局ツールで実行する（[SPSAの設計書の参考資料](../plans/spsa.md#参考資料)）。

## 決定

SPSAの反復は`minase spsa`という別のコマンドで実行し、`minase match run`にはSPSAのモードも、側ごとに`setoption`を送る引数も足さない。
エンジンの起動と通信、1局と1ペアの進行、開始局面の生成、およびコミットのビルドは、公開APIとは扱わない`#[doc(hidden)]`のライブラリのモジュール`minase::harness`として両コマンドが共有する。
`harness`のプレイヤー設定は`usiok`の後、`isready`の前に送る`setoption`の列を持ち、`match_runner`はこの列を常に空で使う。

## 検討した代案

`match_runner`にSPSAのモードや側ごとの`setoption`の引数を足す案は、係数を注入した状態で採否の測定を実行する余地をコマンドラインに残すので採らなかった。
fishtestやOpenBenchを調整サーバーとして使う案は、中将棋の規則を判定するminaseの審判層を対局に使えないので採らず、更新則の式だけを借りた。
2つのコマンドが同じソースファイルを`#[path]`で取り込む案は成立するが、ライブラリに置けば単体テストが1回で済み、`rng`と`stats`の置き方とも揃うので採らなかった。

## 結果

SPSAの対局は、採否の測定と同じ審判層、異常の裁定、およびペアの構造で進む。
SPSAのエンジン指定の`commit:`は`tuning` featureを付けてビルドし、通常ビルドとは別のキーでキャッシュする。
対局の実行部を変えると両コマンドの挙動が同時に変わるので、`harness`の変更は両方のテストで確かめる。
SPSAの対局結果は採否の根拠にしない（[SPSAの値の採否のADR](adopt-spsa-values-only-through-the-staged-gate.md)）。
