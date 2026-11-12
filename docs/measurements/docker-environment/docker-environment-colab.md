# Colab機での導入スクリプトの確認

## 目的

[Dockerイメージの設計書](../../plans/archived/docker-environment.md)の「検証」の節の項目11の記録である。
Google Colabで借りる機械（以下、Colab機）はコンテナを使えないので、実行環境をイメージではなく`tools/train/zero/setup.sh`で導入する。
この設計書は`setup.sh`を、すべての導入先を`tools/train/zero/`の下の`.runtime/`と`.venv/`に置く形へ改め、学習器の拡張モジュール`mnsd_reader`のビルドを新しい`sync-env.sh`へ分けた。
この2つのスクリプトで、導入、ビルド、およびニューラルネットワーク（以下、ネット）の出力の照合が通るかを、NVIDIA L4の機械で確かめた。

## コマンドライン

確認日は2026年11月12日である。
機械は[借りる機械の操作の手引き](../../guides/alphazero-machines.md)の2節と3節の手順で借り、ソースの書庫、初期ネット、基準の出力、および確認のスクリプトをrsyncで送った。

```console
colab new -s dockerenv-check --gpu L4
git archive --format=tar -o /mnt/synology_nas/minase/alphazero/src/minase-ffe28348a42ec902066d9ad1cf91a4c4ab12e536.tar ffe28348a42ec902066d9ad1cf91a4c4ab12e536
rsync -a -e <ssh> <書庫> root@colab:/content/minase.tar
rsync -a -e <ssh> model.onnx pytorch.json colab-check.sh root@colab:/content/
<ssh> root@colab 'setsid nohup bash /content/colab-check.sh /content/minase.tar /content/model.onnx /content/pytorch.json > /content/check.out 2>&1 < /dev/null &'
colab stop -s dockerenv-check
```

`<ssh>`は、手引きの3節の、`colab ssh --proxy-mode`をProxyCommandにしたsshであり、`<書庫>`は`git archive`で作ったファイルである。
借りた機械での処理は[colab-check.sh](docker-environment-colab/colab-check.sh)が行う。
このスクリプトは端末から切り離して起動し、最後まで通ったことを出力で確かめてから、`colab stop`で機械を止めた。
スクリプトは、`/content/src`への書庫の展開、`setup.sh`、`env.sh`の読み込み、`sync-env.sh`、`cargo build --release --bin minase`、`nn-eval`による出力の書き出し、および`nn_compare.py`による照合を順に実行し、工程の間で時刻を出す。
照合は、同じ288局面のネットの出力を、学習器のPyTorchの出力を基準にして、minaseの推論器の出力と比べる処理である。minaseの出力は、ONNX RuntimeのCPUとTensorRTのFP16の2つの実行プロバイダについて、バッチの大きさ1、7、256で書き出すので、出力と比較はそれぞれ6つになる。

## エンジン

ソースはコミット`ffe28348`である。
ネットは、強化学習の2回目の実行の初期ネット（NASの`cycles-rl2/cycle-0000/model.onnx`、20ブロック256チャネル、SHA-256は`7a9f3ec7…d436d43d76`）であり、照合の基準は同じディレクトリの`check/pytorch.json`である。

## 環境

機械はColabのL4（NVIDIA L4、23,034 MiB、ドライバ580.82.07、仮想CPUは12個）である。
この日の`colab usage`が示したL4の消費レートは、Colabの課金の単位である計算単位（compute unit、CU）で1.94 CU/時であり、手引きに書いてある2026年10月27日の1.54 CU/時より高かった。
借りた時点の機械には、`/usr/local/bin/uv`とPython 3.13.16がもとから入っており、Rustは入っていなかった。

## 結果

確認のスクリプトは最後まで通り、6つの照合はすべて許容誤差に収まった。

| 工程 | 時間 |
|---|---|
| 書庫（60 MB）、ネット（112 MB）、基準の出力の転送 | 27秒 |
| `setup.sh`（uv、Rustのツールチェーン、Python、依存パッケージ、ONNX Runtime） | 1分16秒 |
| `sync-env.sh`（版を決めるファイルの検査と`mnsd_reader`のビルド） | 34秒 |
| minaseのリリースビルド | 2分29秒 |
| `nn-eval`の6つの出力（TensorRTのエンジンの作成を含む） | 1分31秒 |
| 6つの照合 | 1秒 |

導入したuv、cargo、およびrustcは、どれも`/content/src/tools/train/zero/.runtime/`の下のものであった。
`/root/.cargo`と`/root/.rustup`は作られなかった。
`/root/.local/share/uv`は機械にもとからあるディレクトリ（2026年11月5日の日付で、中身は空のロックファイルだけ）であり、導入では変わらなかった。
uvは、機械にもとからあるPython 3.13.16を使わず、`.runtime/python/`にPython 3.13.16を入れた。
導入後の大きさは、`.runtime/`が1.3 GB、`.venv/`が8.5 GBであった。

照合での基準との差の最大は、次のとおりである。
方策は、CPUでは合法手のlogitで、TensorRTでは合法手の確率で比べる。

| 実行プロバイダ | バッチ | 方策の差の最大 | 価値の差の最大 |
|---|---|---|---|
| ONNX RuntimeのCPU | 1、7、256 | logitで1.2×10⁻⁴ | 8.8×10⁻⁸ |
| TensorRTのFP16 | 1 | 確率で3.2×10⁻⁵ | 2.6×10⁻⁴ |
| TensorRTのFP16 | 7 | 確率で3.4×10⁻⁵ | 2.5×10⁻⁴ |
| TensorRTのFP16 | 256 | 確率で3.4×10⁻⁵ | 2.7×10⁻⁴ |

CPUの欄のlogitの差は、PyTorchの出力との差そのものである。照合は、ONNXが返すlogitがFP16へ丸められていることを見込んで、この差からFP16の1目盛りを引いた値を許容誤差1×10⁻⁴と比べる。

セッションは19時15分58秒（日本時間）に借り、19時23分28秒に止めた。
借りていた時間は7分30秒であり、`colab usage`の残高は247.87 CUから247.72 CUへ0.15 CU減った。

機械の出力の全体は[check.out](docker-environment-colab/check.out)にある。

## 結論

改めた`setup.sh`と`sync-env.sh`は、Colab機で導入から照合まで通り、導入先は環境のディレクトリの中に収まった。
