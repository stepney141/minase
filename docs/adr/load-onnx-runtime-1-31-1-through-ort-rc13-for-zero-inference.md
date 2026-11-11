# AlphaZero型探索の推論は、ortのrc.13のまま、ONNX Runtime 1.31.1の実行時ライブラリを読み込む

## ステータス

承認済み。決定日は2026年11月10日。

## コンテキスト

AlphaZero型探索のネットの推論は、ortクレートの`load-dynamic`で、`tools/train/zero/env.sh`が`ORT_DYLIB_PATH`に指定するONNX Runtimeの共有ライブラリを読み込む（[推論の方式のADR](run-zero-network-inference-through-ort-load-dynamic.md)）。
`env.sh`は、これまでONNX Runtime 1.28.3のCUDA 13版を固定していた。
1.28.3では、CUDAの実行プロバイダを使うと、出力を書き終えた後の終了処理でglibcがヒープの破壊を検出して異常終了（終了コード134）することがあった（[ネットの入出力の照合の記録](../measurements/alphazero-network/random-net-output-check.md)）。
vast.aiのRTX 4090の機械では、TensorRTの実行プロバイダも、新しいキャッシュでエンジンを作る`minase dev nn-eval`の約6割が同じ形で異常終了した。
実行器の照合はこの終了コード134で止まるので、1サイクルを最後まで回せなかった（[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)）。
ドライバの版と共有ライブラリの読み込みの順序を変えても異常終了は減らず、原因は特定できなかった。
そこでClaudeが、ortは据え置いたまま、ONNX Runtimeの実行時ライブラリだけ版を上げる方式を提案した。
ortのrc.13は`api-28`の機能でONNX RuntimeのC APIの版28を要求するが、ONNX RuntimeのC APIは、新しい版の実行時ライブラリでも古い版の呼び出しを受け付けるからである。
エンジンの開発者は、2026年11月10日に、1.31.1で異常終了が消えれば版を上げると決めた。

## 決定

`env.sh`は、ONNX Runtime 1.31.1のCUDA 13版（`onnxruntime-linux-x64-gpu_cuda13-1.31.1.tgz`、SHA-256は`b2146862e691f7e338f5aff9c7a9a5567c2ca57cf01442a1adb3a13bf62d9095`）を固定する。
ortは`=2.0.0-rc.13`と`api-28`のまま、TensorRTは10.14.1.48のままとする。

## 検討した代案

1.28.3のまま、出力のJSONが完全なら照合の終了コード134を成功とみなす案は採らなかった。
異常終了を成功として扱うと、本当に失敗した実行と区別できなくなるからである。

`minase dev nn-eval`の終了処理を飛ばす案は採らなかった。
終了処理を飛ばす`_exit`を呼ぶには`unsafe`が要るがminaseは`unsafe_code = "forbid"`であり、仮に飛ばしてもヒープを壊している原因が残るからである。

ortをrc.14以降へ上げる案は採らなかった。
rc.14以降はminaseの`rust-version`を1.92へ上げる必要があり（推論の方式のADR）、実行時ライブラリを差し替えるだけで異常終了が消えたからである。

2回目の学習をColabのG4の機械だけで回し、異常終了が起きるvast.aiのRTX 4090の機械を使わない案は採らなかった。
G4が借りられない日に、代わりに使える機械がなくなるからである。

## 結果

同じRTX 4090の機械（ドライバ580.95.05）で2つの版を交互に実行すると、新しいキャッシュでエンジンを作る`minase dev nn-eval`のバッチ1は、1.28.3では10回中10回、1.31.1では10回中0回異常終了した。
生成器で8局を生成する実行は、どちらの版でも4回とも正常に終わった。
TensorRTの出力は、作業機のRTX 3060 Tiでは2つの版で288局面のすべてが一致した。
4090でも、1.31.1の10回と1.28.3の9回の計19回は288局面のすべてが一致した。1.28.3の残りの1回は、作り直したエンジンが他と違い、価値の差が最大0.0029であった。
作業機では、1.31.1で実行器の照合と同じ6つの比較がすべて許容誤差に収まり、CUDAの実行プロバイダも4回とも正常に終わった。
4枚の4090の機械でも、1.31.1で、生成から照合までの1サイクルが最後まで回った（vast.aiの測定の機械H）。
異常終了が1.31.1で起きなくなった理由は特定していない。
ただし、2026年11月11日に機械Hと同じ出品を借り直した機械（ドライバ610.57.04）では、1.31.1でも、出力をすべて書いた後の終了処理で`corrupted double-linked list`を出す異常終了が起きた。新しくエンジンを作った`minase dev nn-eval`は4回中2回、4,096局の生成は25回中9回で、同じ日に借りた別の4090の機械（ドライバ580.159.03）では1回も起きなかった。RTX PRO 6000 Blackwell Server Editionを1枚積んだ機械（ドライバ595.71.05）でも、生成の6回中2回で起きた（[生成の速さの測定](../measurements/alphazero-inference-transfer/alphazero-inference-transfer-generation-speed.md)の「終了処理での異常終了」）。作業機（RTX 3060 Ti、ドライバ615.78.08）では、同じ日のエンジンの作成36回と生成18回のどれも正常に終わった。
1.31.1は異常終了をなくしたのではなく、機械によっては残る。起きる機械と起きない機械の違いはドライバの版だけでは説明できず、この時点では原因を調べていなかった。
原因は、同じ日の[調査](../measurements/adhoc/zero-teardown-abort-root-cause.md)で、ortのrc.13がONNX Runtimeの環境を解放する順序にあると分かった。ONNX Runtimeの版もドライバの版も原因ではない。解放済みの領域への書き込みは1.31.1でも実行のたびに起きており、1.31.1で異常終了の回数が減ったのは、ヒープの配置が変わって破壊が検出されにくくなったためと考えられる。ortを`=2.0.0-rc.13`のままとする決定は、[ortの版と環境の寿命のADR](hold-the-onnx-runtime-environment-until-process-exit-through-ort-rc15.md)が置き換えた。実行時ライブラリを1.31.1に固定する決定は、そのまま有効である。
ColabのG4の機械では、1.31.1をまだ動かしていない。`env.sh`はColabの機械とも共有なので、G4での最初のサイクルが1.31.1の最初の実行になり、照合が通らなければ実行器はそこで止まる。

ONNX Runtimeがエンジンのキャッシュの名前を決める処理（`tensorrt_execution_provider_utils.h`の`TRTGenerateId`と、`tensorrt_execution_provider.cc`の`_sm<計算能力>`）は、1.28.3と1.31.1で同じである。
このため、キャッシュの鍵の規則（[キャッシュのADR](key-tensorrt-engine-caches-by-network-checksum-gpu-and-max-batch.md)）は変えない。
`TRTGenerateId`はONNX Runtimeの版をハッシュに含むので、1.28.3で作ったエンジンは1.31.1では読まれず、作り直される。
1.31.1を使う前の測定記録は1.28.3で取ったものであり、それらを再現するときは、その記録のコミットの`env.sh`で1.28.3を導入する。
