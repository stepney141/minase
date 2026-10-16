# `unsafe`は学習器のクレートだけに許し、エンジンとライブラリでは禁じ続ける

## ステータス

置き換え済み。[`unsafe`は学習器のGPUの処理だけに許し、エンジンとライブラリでは禁じ続ける](../allow-unsafe-only-in-the-trainer-gpu-code.md)によって置き換えられた。

## コンテキスト

リポジトリのworkspaceは`[workspace.lints.rust]`で`unsafe_code = "forbid"`を定め、エンジン`minase`とライブラリ`minase-core`はこれを継承している。
[Rust製NNUE学習器](../../plans/archived/nnue-trainer-rust.md)はGPUで学習するために、CUDAのドライバとNVRTCをcudarcから使う。
cudarcのカーネル起動とページ固定メモリ（OSが移動しないと保証したホストメモリ）の確保は`unsafe`を要する。
教師データのファイルをmemmap2でメモリへマップする`map`も、マップしている間にファイルが変更されないことを呼び手が保証する前提の`unsafe`な関数である。

## 決定

学習器のクレート`crates/minase-trainer`だけはworkspaceのlintを継承せず、`unsafe`を許す。
`minase`と`minase-core`は`unsafe_code = "forbid"`を保つ。
学習器の中でも、`unsafe`はGPUの処理を置く`nnue::gpu`と、教師データをメモリへマップする`data`モジュールの`unsafe fn`である`Dataset::open`と内部の`MappedFile::open`、およびそれらの呼び出し箇所に限り、各ブロックに前提条件を書く（詳細は`nnue::gpu`と`Dataset::open`のdocコメントにある）。

## 検討した代案

学習器でも`unsafe`を禁じる案は、cudarcでカーネルを起動できず、GPUで学習できないので採らなかった。
workspace全体で`unsafe`の禁止を外す案は、エンジンとライブラリに`unsafe`を持ち込まない方針を崩すので採らなかった。

## 結果

学習器は`publish = false`であり、エンジンとライブラリのリリースと変更履歴には現れない。
`Dataset::open`の呼び手は、返された値が生きている間、渡したファイルを外部のプロセスも含めて変更も切り詰めもしないことを保証しなければならず、学習と測定のコマンドが終わるまで入力ファイルを書き換えてはならない。
