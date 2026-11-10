# 手引きの一覧

本ディレクトリには、人間とエージェントが手順どおりに実行する手引きを置く。
目的に応じて、次の表から読む手引きを選ぶ。
ファイルの形式、記録の欄、および実装の式はコードのdocコメントが定めるので、手引きはその場所を示すだけにとどめる。

## 開発全般

| 手引き | 内容 | 読むとき |
|---|---|---|
| [testing-minase-rules.md](testing-minase-rules.md) | minaseのテストの書く範囲、検証の層、期待値の根拠、決定性、およびマージ前の検査 | テストを書くとき、マージの前 |
| [testing-general-principles.md](testing-general-principles.md) | テスト一般の原則（良いテストの条件、依存の使い分け、カバレッジと変異テスト、不安定なテストの扱い）と出典 | テストを設計またはレビューするとき、手引きの規則の理由を確かめるとき |
| [debugging.md](debugging.md) | 症状を再現し、原因の局面と処理まで絞り込み、修正を確かめる手順 | 不具合を調べるとき |
| [codex-delegation.md](codex-delegation.md) | codexへ実装を委任する手順と既知の落とし穴 | 実装をcodexへ任せるとき |
| [gemini-proofreading.md](gemini-proofreading.md) | agyからGeminiを呼び出して文書を推敲させる手順と注意点 | 文書を書き終えたとき |
| [usi-extensions.md](usi-extensions.md) | USI原典と異なるコマンドの扱い、中将棋の表記、オプション、および独自の照会コマンドの一覧 | GUIや対局用のプログラムからminaseをUSIで動かすとき、独自コマンドの書式を調べるとき |
| [file-formats.md](file-formats.md) | ディスクに読み書きするファイル形式と、その仕様の場所の一覧 | ファイル形式の仕様を探すとき |
| [release.md](release.md) | 版数の決定からタグのpush、およびcrates.ioへの公開までの手順 | ライブラリまたはエンジンをリリースするとき |

## 棋力の測定と係数の調整

| 手引き | 内容 | 読むとき |
|---|---|---|
| [sprt.md](sprt.md) | 自己対局でエンジンの変更を測る標準手順と、測定記録の規約 | 探索、評価、または時間管理の変更を採否にかけるとき |
| [spsa.md](spsa.md) | 探索と時間管理の係数をSPSAで調整する手順 | 係数を調整するとき |

## 評価関数の学習と解説

| 手引き | 内容 | 読むとき |
|---|---|---|
| [nnue-training.md](nnue-training.md) | NNUEの標準学習レシピ、学習と検査と診断の手順、およびオプションの一覧 | NNUEを学習するとき、学習の引数を調べるとき |
| [evaluation-nnue.md](evaluation-nnue.md) | 探索が使うNNUEの計算式と、学習の目的関数の解説 | NNUEの式を確かめるとき |
| [pst-training.md](pst-training.md) | PSTとFMの学習手順 | PSTとFMを学習するとき |
| [evaluation-pst-fm.md](evaluation-pst-fm.md) | 比較用に残すPSTとFMの計算式と、学習の目的関数の解説 | PSTとFMの式を確かめるとき |

## AlphaZero型探索

| 手引き | 内容 | 読むとき |
|---|---|---|
| [alphazero-training.md](alphazero-training.md) | 深層強化学習の標準レシピ、サイクルの起動、経過の確認、定例の測定、止めて再開する手順、およびオプションの一覧 | 強化学習を回すとき（最初に読む）、引数を調べるとき |
| [alphazero-machines.md](alphazero-machines.md) | GPUの機械を借りて環境を導入し、計算を実行して返すまでの操作 | 機械を手で操作するとき、新しいGPU種別やレンタル機を試すとき |
| [alphazero-network-check.md](alphazero-network-check.md) | PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力を照合する手順 | ネットの大きさを変えたとき、照合が不合格になったとき |
