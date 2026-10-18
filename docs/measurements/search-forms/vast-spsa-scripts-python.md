# Pythonへ書き直したvast.aiのSPSAスクリプトの実機での確認

## 目的

[探索の関数形の見直しの統括計画](../../plans/search-forms.md)のフェーズ1のスクリプトを、2026年10月18日にBashからPythonへ書き直した（設計判断の「スクリプトの言語」）。
書き直した`launch.py`、`remote.py`、`fetch.py`、および`scripts/vast/common.py`が、本物のvast.aiの機械で、作成から削除までの正常の流れを通すことを確かめる。
棋力測定ではない。
異常時の止まり方は、偽の`vastai`、`ssh`、`scp`、および`rsync`を使うテストで確かめ、実機では繰り返していない。
Bashの版を実機で確かめた結果は[スクリプトの検証の記録](vast-spsa-scripts.md)にある。

## コマンドライン

```console
python3 scripts/vast-spsa/launch.py verify-py 51337989 30
python3 scripts/vast-spsa/fetch.py verify-py
```

`fetch.py`は、調整機の`logs/exit-status`ができてから起動した。
起動直後から動かすと、1回目の取得の後に10分待つからである。

`session.toml`は次のとおりとした。
パラメーターファイルは、[スクリプトの検証の記録](vast-spsa-scripts.md)と同じく、`99ccc6bb`の既定の係数から`FmScale`の行を除いた38係数である。
対局を短くするため、`local_nps`を測定機の実際の値（約126万NPS）より小さい150,000とし、持ち時間を縮めた。

```toml
commit = "99ccc6bb"
runner_commit = "master"
seed = 5900000000
iterations = 4
pairs_per_iteration = 1
rules = "engine-default"
precheck = false
local_nps = 150000
game_rss_bytes = 630000000
```

## エンジン

調整用ビルドは、調整機で`99ccc6bb`を`--features tuning`つきでビルドしたminaseである。
runnerは、調整機で2026年10月18日のmasterの先頭`af6d420b`をビルドした通常のminaseである。
`fetch.py`の`minase spsa apply`による検査は、測定機のworktreeでビルドしたrunnerで行った。

## 環境

調整機は、統括計画の選定条件で照会した出品のうち最も安い、AMD EPYC 7662の2基構成（ベルギー、物理128コア、論理256スレッド、1時間1.60米ドル、インスタンス56480286）である。
測定機は、Intel Core Ultra 7 265KFである。

## 結果

`launch.py`は、出品の検査、作成、稼働の確認、bundleと設定の送信、および開始の印の作成までを86秒で終えた。
調整機の`remote.py`は、runnerのビルド（59秒）、調整用ビルド、同時対局数127での負荷時の速度の測定（平均442,655 NPS、`f`は0.34、持ち時間`time=3400+34`）、および4反復のセッション（42秒）を実行し、終了コード0を記録した。
4反復はすべて適用され、エンジン異常、時間切れ、および拒否着手は0件だった。
`launch.py`の開始から`exit-status`ができるまでは約5分だった。

`fetch.py`は、SSHを通したrsyncで実行ディレクトリとログを取得し、4反復が欠けずにそろった`run/`に`apply`の検査を通して、削除を確かめてから終了コード0で終わった。
所要時間は31秒だった。
`launch.py`の開始から削除の確認までは約6分（10時43分41秒から10時49分26秒）、費用は約0.15米ドルだった。

## 結論

Pythonへ書き直したスクリプトは、本物の機械で、作成、ビルド、速度の測定、セッション、取得、検査、および削除の正常の流れを通した。
