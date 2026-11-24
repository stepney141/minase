# NNUEの学習器を、構造によらない学習ループと構造ごとの実装に分け、構造ごとのサブコマンドで呼ぶ

## ステータス

承認済み。決定日は2026年11月19日。

## コンテキスト

NNUEの学習器は、1種類の構造（P型またはHalfKP-hm-b8の入力と、要素積と2段の隠れ層の出力部）を前提にした1つのモデルを持ち、GPUかCPUかだけを列挙で切り替えていた。
前の世代のネットを`--base`で受け取り、駒価値、勝率尺度、および規則セットを引き継いでいた。
重みは世代ごとに初期化していたので、`--base`が学習の出発点を決めていたわけではない（[世代反復の設計書](../plans/archived/nnue-generation-iteration.md)）。

P型のLayerStackとHalfKP_vmのSimpleを同じ学習器で学ぶことになった（[構造のADR](compose-p-as-a-tatara-layerstack-with-a-psqt-and-halfkp-vm-as-a-simple-network.md)）。
いまの1つのモデルに構造の分岐を足していくと、構造ごとのパラメータの配列とカーネルが1つのモデルに混ざる。
tataraは、学習率の予定、教師の供給、検証、記録、および保存の時期を受け持つ学習ループから、構造ごとの実装をトレイト`TrainerBackend`越しに呼ぶ（コミット`87f1955`の`crates/nnue-train/src/trainer.rs`）、コマンドも構造ごとのサブコマンドに分ける。
学習器の記録をtataraの`experiment.json`に揃えるかについては、2026年11月18日にエンジンの開発者が、いまの3つのファイル（`training.jsonl`、`summary.json`、`config.json`）を保つと決めた（[設計書の依存関係](../plans/archived/nnue-tatara-framework-and-halfkp-vm.md#依存関係)）。

## 決定

学習器を、構造によらない学習ループ（`crates/minase-trainer/src/nnue/train/`）と、LayerStackとSimpleの構造ごとの実装（`nnue/layerstack/`と`nnue/simple/`）に分け、境界をトレイト`TrainerBackend`にする。
学習ループは、学習率の予定、教師の保管と間引き、教師値の組み立て、バッチの供給、検証と保存の時期、時間の上限、再開の照合、および記録（`training.jsonl`、`summary.json`、`config.json`）を受け持つ。
構造ごとの実装は、1バッチの更新と未回収の損失の回収、検証の1バッチ、構造に固有の統計、再開用のチェックポイント、および書き出しを受け持ち、CPUとCUDAの実装を構造の中の列挙で持つ。
コマンドは`minase-trainer nnue train layerstack`と`nnue train simple`に分け、構造に意味のない引数は受け付けない。
`--base`は廃止し、規則セットと教師の勝率尺度を必須の引数`--rules`と`--teacher-cp-per-logit`で受け取って、全ての教師ファイルのヘッダの規則セットと照合する。

## 検討した代案

いまの1つのモデルに構造の分岐を足していく案は、構造ごとの配列とカーネルが混ざるので採らなかった。
`--base`を残して規則セットと勝率尺度だけを引き継ぐ案は、tataraの形式に規則セットと実数の勝率尺度の欄がなく、前の世代のネットから引き継げないので採らなかった。
GPUかCPUかを構造と同じ軸の列挙にする案は、構造を足すたびにデバイスの組の数だけ実装が増えるので採らず、デバイスを構造の中の別の軸として残した。
NNUE、PST、FMの学習器を分ける決定（[ADR](split-training-pipelines-by-evaluation-function.md)）は変えない。

## 結果

学習ループはパラメータの配列を知らず、Simpleを足すときにループを変えずに済んだ。
CPUの参照実装は、順伝播と局面ごとの逆伝播の積をGPUと同じ順の単精度で、局面方向の和を倍精度で計算する。
CPUの参照を倍精度の順伝播で書いたところ（[教訓](../lessons/match-cpu-reference-precision-to-the-gpu-before-comparing-adamw-updates.md)）、打ち消し合って0に近くなる勾配の成分で、AdamWが和の相対誤差を学習率程度の更新の差へ拡大し、CPUとGPUの3回の更新が許容差（`|GPU − CPU| ≤ 10⁻⁶ ＋ 10⁻⁵×|CPU|`）を超えたからである。
書き出しで量子化の範囲を超える値はエラーで止め、いまの「平均の絶対誤差が勝率尺度の1/40を超えたら止める」条件は外して、量子化誤差は記録だけにする。
棋力は対局で判断する。
再開の照合は、軌跡を変える設定だけで行い、予算（総superbatch数、時間の上限、スレッド数）を含めない（[教訓](../lessons/resume-hash-excludes-budgets.md)）。
