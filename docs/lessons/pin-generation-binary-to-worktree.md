# 長時間の生成は生成コミットを固定したworktreeで実行する

## 症状

世代1の教師データ生成は約20時間かかり、その間に本ツリーでは注入の修正やレビュー対応のコミットが続いた。
本ツリーのバイナリで生成すると、途中のビルドで生成器の挙動やヘッダの生成コミットが変わり、5ファイルの来歴が揃わない。

## 原因

`selfplay_gen`はヘッダへ`git rev-parse HEAD`を記録し、作業ツリーが汚れていれば停止する。
生成中に本ツリーを編集すると、この来歴記録の前提が崩れる。

## 以後の規則

数時間を超える生成や測定は、対象コミットを固定したgit worktreeでビルドしたバイナリから実行し、本ツリーの編集とコミットを生成から遮断する。
短い生成でも、実装を進めているツリー（codexなどのサブエージェントが編集するworktreeを含む）からは実行せず、生成のためだけに切り離したworktreeを作る。生成器は起動のたびに作業ツリーの汚れを検査するので、並行する編集が1つでもあれば後続のファイルの生成が止まる。

## 出典

- [plans/evaluation-gen1.md](../plans/archived/evaluation-gen1.md) の「学習の構成」
- [measurements/out-of-core-shuffle/out-of-core-shuffle-equivalence.md](../measurements/out-of-core-shuffle/out-of-core-shuffle-equivalence.md) の「入力」（実装中のworktreeから起動した生成が2ファイル目で止まった）
