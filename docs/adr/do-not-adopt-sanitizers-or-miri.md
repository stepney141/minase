# サニタイザとMiriを検証の道具に採らない

## ステータス

承認済み。決定日は2026年9月27日。

## コンテキスト

[デバッグ機能の整備](../plans/archived/debugging-tools.md)は、先行エンジンが持ちminaseに欠けている調査の道具を洗い出した（[調査メモ](../research/engine-debugging-survey.md)）。
StockfishのCIは、ThreadSanitizer、UndefinedBehaviorSanitizer、およびvalgrindで試験を実行する（同じ[調査メモ](../research/engine-debugging-survey.md)）。
AddressSanitizer、valgrind、およびMiriが主に検出するのは、unsafeなコードによるメモリの誤用である。
ライブラリ`minase-core`とエンジン`minase`は、ワークスペースのlintで`unsafe_code = "forbid"`を宣言している。
共有の置換表と並列探索は原子操作だけで状態を共有するので、自作のコードのデータ競合は言語が排除している。
同マイルストーンが対象とした不具合は、差分更新の漏れや置換表の値の扱いの誤りのような論理的な不整合だった。

## 決定

サニタイザとMiriを、minaseの検証と調査の道具として常設しない。
論理的な不整合はフィーチャ`invariants`の整合検査で、並行処理の論理的な競合は回帰テストの連続実行で確かめる。

## 検討した代案

サニタイザとMiriを試験の実行に加える案は、unsafeなコードを持たない2つのcrateでは主な検出対象が生じず、論理的な不整合には直接の効果が小さいので採らなかった。

## 結果

整合検査は[重い整合検査をフィーチャ`invariants`に置く](enable-invariant-checks-in-tests-by-feature.md)に従い、並行処理の確認は[並行処理の回帰テストの教訓](../lessons/repeat-concurrency-regression-tests.md)に従う。
グローバルアロケータのmimallocなど、依存ライブラリの安全性は`unsafe_code = "forbid"`の宣言の対象外である。
依存ライブラリの不具合が疑われる場面が現れた時点で、この決定を見直す。
学習器のcrate`minase-trainer`はGPUの呼び出しと教師データのメモリ写像にunsafeなコードを使う（[`unsafe`は学習器のクレートだけに許す](archive/allow-unsafe-only-in-the-trainer-crate.md)）ので、この決定の前提はそのcrateには当てはまらない。
