# 推論の転送を減らした後の、RTX 4090を4枚積んだ機械での1サイクル

## 目的

[推論の転送を減らす設計書](../../plans/archived/alphazero-inference-transfer.md)のフェーズ6の確認の記録である。
設計書は、ニューラルネットワーク（以下、ネット）の推論の入出力を4点で改めた。入力をuint8で送ること、方策を盤面によらず合法手になり得る16,108ラベルへ詰めること、その方策をFP16で受け取ること、および出力を呼び出しごとにピン留めメモリへ結び付けることである（[入出力の受け渡しのADR](../../adr/exchange-zero-inference-tensors-as-uint8-inputs-and-packed-fp16-policies-in-pinned-memory.md)）。
この確認では、4点をすべて入れたコミットで、強化学習の1サイクル（自己対局の生成、学習、および学習したネットの出力の照合）が、借りた機械で最後まで回ることを確かめた。
生成と学習は、新しい入出力の形で書き出し直した初期ネットから始めた。

## コマンドライン

測定日は2026年11月11日である。
vast.aiで、RTX 4090を4枚積んだ機械（インスタンス60170262、出品58773738、Intel Xeon Platinum 8352V、使えるCPUは69個、出品の`pcie_bw`は24.0 GB/s、1時間1.904米ドル）を借り、起動スクリプト[launch-vast.py](../alphazero-multi-gpu/launch-vast.py)で1サイクルを回した。起動スクリプトは、作業機で動いて機械を借り、サイクルの各工程を機械の上で起動する実行器`minase_zero run-cycles`を走らせ、成果物を受け取ってから機械を削除する。
系列は`transfer-vast`で、2回目の強化学習の実行の初期ネット（NASの`cycles-rl2/cycle-0000/`）から分岐させた。
`<C>`はコミット`0aae829ba0f5fe676b132c2e04c46a7780bd00cc`、`<A>`はNASの`/mnt/synology_nas/minase/alphazero`である。
生成の基本シードの加算値（`--generation-seed-offset`）の4×10⁸は、使ったシードの区間を記録する台帳の既存の区間と、通常の生成の区間でも、生成し直すときに使う区間（基本シードにさらに5×10⁸を足した区間）でも重ならない値として選んだ。

```console
python3 docs/measurements/alphazero-multi-gpu/launch-vast.py --offer 58773738 --label minase-transfer-cycle \
  --image nvidia/cuda:13.0.3-base-ubuntu24.04 --hours 2 --commit <C> --root <A> --salvage-dir <A>/salvage \
  -- --cycles-dir transfer-vast --base-cycles-dir cycles-rl2 --branch-from 0 \
  --cycle1-reference-summary <A>/cycles/cycle-0001/selfplay/summary.json \
  --generation-commit <C> --trainer-commit <C> --first-cycle 1 --last-cycle 1 --cycle-minutes 60 \
  --start-position shuffled-setup --generation-seed-offset 400000000 --target-positions 1000000 --chunks 20 \
  --simulations 32 --threads-per-gpu 4 --games-per-thread 128 --replay-buffer-first-cycle 1 \
  --replay-buffer-positions 280520080 --batch-size 1024 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 \
  --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 \
  --train-seed-base 20261100 --minase <作業ツリー>/target/release/minase --poll-seconds 60
```

引数は、生成と学習のコミット、系列の名前、および基本シードの加算値を除いて、[複数GPU化のvast.aiの測定](../alphazero-multi-gpu/alphazero-multi-gpu-vast.md)の1サイクルと同じである。

## エンジン

生成器と学習器はコミット`0aae829b`（上の`<C>`）である。推論器と学習器の書き出しのコードは、入出力の4点の変更を積み終えたコミット`43441d00`から変わっていない。
初期ネットの`model.onnx`のSHA-256は`7a9f3ec7…`で、作業機で同じチェックポイントから書き出し直したものである（[作業機での確認](alphazero-inference-transfer-workstation.md)の「初期ネットの書き出し直し」）。

## 環境

コンテナのイメージは`nvidia/cuda:13.0.3-base-ubuntu24.04`で、PyTorch 2.11.0（CUDA 13.0）、ONNX Runtime 1.31.1、およびTensorRT 10.14.1を使った。
NVIDIAのドライバは580.178.04である。

## 結果

実行器は、生成、学習、および照合を終えて、終了コード0で終わった。
起動スクリプトは、最後に受け取ったファイルのSHA-256が機械の上のファイルと一致することを確かめてから、機械を削除した。

| 工程 | 時間 | 内容 |
|---|---|---|
| 生成 | 920秒（15.3分） | 29,150局、訓練局面1,071,664 |
| 学習 | 715秒（11.9分） | 3,969回の更新、うち更新そのものは468秒 |
| 照合 | 191秒（3.2分） | 6通りの比較がすべて終了コード0 |
| 1サイクル | 31.4分 | 生成の開始から照合の後の確定まで |

生成器が自分で測った経過時間は856秒で、その間に作った訓練局面は、目標に届いた後に確定した塊の分も含めて約131万、毎秒約1,531局面であった。
複数GPU化の測定では、同じ引数の1サイクルが、機械Bで生成1,142秒、全体で約36分であった。機械が違うので厳密な比較ではないが、生成は約0.81倍の時間で終わった。
生成器と`minase dev nn-eval`は、どれも正常に終了し、終了処理での異常終了は起きなかった。

学習したネット（SHA-256 `523dd14b…`）の照合の差の最大は、次のとおりである。

| 実行プロバイダ | logitの差 | 確率の差 | 価値の差 |
|---|---|---|---|
| CPU（バッチ1、7、256とも同じ） | 3.9×10⁻³ | 7.5×10⁻⁴ | 1.3×10⁻⁶ |
| TensorRT | 4.6×10⁻² | 4.0×10⁻³ | 1.0×10⁻² |

CPUのlogitの差は丸める前の参照との差であり、FP16への丸めによるものである。CPUの比較は、参照をFP16へ丸めてFP16の1目盛りを許す方式で、許容誤差1×10⁻⁴を満たした。
TensorRTの確率の差と価値の差は、許容誤差の1×10⁻²と5×10⁻²の内側である。

機械は作成から削除まで0.79時間で、口座の残高の減少は1.44米ドルであった。
1サイクルの31.4分は、この機械の料金で約1.00米ドルにあたる。

出力は、NASの`transfer-vast/cycle-0001/`と、サイクルの記録`transfer-vast.jsonl`にある。

## 結論

入出力の4点の変更をすべて入れたコミットで、書き出し直した初期ネットからの生成、学習、および照合の1サイクルが、RTX 4090を4枚積んだ機械で最後まで回った。
1サイクルは31.4分、約1.00米ドルであった。
