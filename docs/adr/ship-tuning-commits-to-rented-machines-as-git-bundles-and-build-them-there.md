# 調整対象のコミットはgit bundleで調整機へ渡し、調整機でビルドする

## ステータス

承認済み。決定日は2026年10月17日。

## コンテキスト

SPSAは、vast.aiで借りる物理コアの多い計算機（以下、調整機）で実行する（[調整機のADR](run-spsa-on-rented-machines-with-nps-scaled-time-controls.md)）。
調整機は、調整対象のコミットの調整用ビルドと、`minase spsa`を実行するrunnerのコミットの通常のビルドを使う。
調整対象のコミットは、多くの場合まだmasterへ統合していないブランチにある。
`.cargo/config.toml`は`target-cpu=native`を指定するので、開発と棋力測定に使う手元の機械（以下、測定機）でビルドした実行ファイルは測定機のCPU向けの命令を含む。

## 決定

測定機で、調整対象のコミットとrunnerのコミットを1つの`git bundle`にまとめて調整機へ送る。
調整機はそのbundleをcloneし、調整対象を`commit:`の形式の調整用ビルドに、runnerを通常のビルドにする。

## 検討した代案

GitHubへブランチをpushして調整機にcloneさせる案は、未統合のブランチを公開リポジトリへ出すので採らなかった。
測定機でビルドした実行ファイルを送る案は、測定機のCPU向けの命令を含むので採らなかった。

## 結果

ブランチを公開せずに、測定機と同じ`commit:`の照合を使える。
調整機でのビルドは機械ごとに`target-cpu=native`で行われるので、同じコミットでも実行ファイルのSHA-256は機械ごとに変わる。
再開時にこの違いを受け入れる照合は、[再開のADR](resume-from-the-manifest-and-accept-rebuilt-tuning-commits.md)が定める。
SPSAの後に同じ調整機で行う事前対局とSTCでも、候補のコミットを同じく`git bundle`で送る（[SPSAの手引き](../guides/spsa.md)の第8節）。
