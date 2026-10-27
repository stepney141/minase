# AlphaZero型探索のGPUはColab機を公式CLIで借り、状態は作業機のNASにだけ置く

## ステータス

承認済み。決定日は2026年10月27日。

## コンテキスト

[AlphaZero型探索の統括計画](../plans/alphazero.md)の強化学習（RL）は、GPUを使う自己対局と学習を数か月にわたって繰り返す。
エンジンの開発者は、Google Colab（以下Colab）の有料プランに毎月付与される計算単位（compute unit、以下CU）を先に使い、足りない分をvast.aiかRunpodで借りる機械で補うと決めている。
Colabのよくある質問によれば、コンテナ化による規約の回避はすべてのランタイムで禁じられ、SSHによる遠隔操作と「chess training」の制限は、無償または残高のないランタイムに限られる。
Colabの仮想機械（以下Colab機）は、セッションを止めるとファイルごと消える。

[実現性の実測](../measurements/alphazero-environment/alphazero-feasibility.md)では、Googleが公開するCLI`colab`で作業機からL4、A100、およびG4を借りられ、H100は借りられなかった。
転送は、`colab upload`が5 GBのファイルでHTTP 500を返して失敗し、`colab ssh --proxy-mode`を経由したrsyncは5 GBを約6.2 MB/秒で送り、1.2 GBを約9.7 MB/秒で受け取った。
無人で負荷をかけたL4のセッションは、12時間8分の間切断されなかった。

## 決定

Colab機は、作業機からCLI`colab`で借り、実行し、止める。
ファイルの転送は`colab ssh --proxy-mode`を経由したrsyncで行い、学習データ、チェックポイント、およびネットは、作業機にNFSでマウントしたNAS（`/mnt/synology_nas/minase/alphazero/`）にだけ置く。
受け取ったファイルは、検査和を照合してから名前を変えて確定させ、確定したファイルだけを再開に使う。

## 検討した代案

ノートブックのUIを手で操作する案は、数か月のRLを手作業に頼ることになるので採らなかった。
Google Driveを永続的な保存先にする案は、`colab drivemount`が端末での人手の操作を要し、自動化に使えないので採らなかった。
`colab upload`と`colab download`で転送する案は、大きなファイルの送信が失敗したので採らなかった。
Colab機の上でコンテナを使う案は、Colabの規約が禁じ、実行環境もないので採らなかった。

## 結果

Colab機が途中で止まっても、確定したファイルから別の機械で再開できる。
転送は作業機の回線の速さに縛られ、新しい機械を借りるたびに学習窓のデータとチェックポイントを送り直す時間がかかる。
NASを1か所の保管先とし、複製は持たない。
手順は[Colab機の操作の手引き](../guides/alphazero-machines.md)にある。
