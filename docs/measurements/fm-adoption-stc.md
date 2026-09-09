# FM補正項の採用比較（STC）

## 目的

[Factorization Machineによる2駒関係評価](../plans/factorization-machine.md)の採否測定として、FM候補（潜在次元32、λ=1.0、λres=1e-4）を開始版と短時間条件のGSPRTで対局させ、長時間条件へ進めるかを判定する。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir data/matches/fm-adoption-stc --seed 20570903 \
  --candidate commit:168e0dc4a8615695f22edf11a5c56da37b04166c \
  --baseline commit:b0153cf27df655a32cb9a255fd380b21bba002b5 \
  --each time=10000+100 gsprt --max-pairs 3000
```

worktree `minase-fm`（ブランチ`fm-eval`）で実行した。

## エンジン

候補はコミット`168e0dc`（第3フェーズの速度改善まで含む、`nets/pst.bin`のSHA-256 `962e22432f622e526c4f4bfcbad2f053994002afb6a6a4381a9ad25b4a337cb1`）、基準は開始版`b0153cf`である。
規則セットは`engine-default`（L0、P0、R1、E0）である。

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）、メモリ30 GB、両エンジンとも`Threads=1`、`USI_Hash`256 MB、同時対局数19、`time=10000+100`である。
測定中に他の対局は走らせていない。

## 結果

2026年9月9日に115ペアを実行し、有効ペア115、破棄ペア0であった。
ペンタノミアル度数は[75, 0, 35, 0, 5]、LLRは−2.949で`decision: H0`である。
候補の得点率は19.6%（2連敗75ペア、1勝1敗35ペア、2連勝5ペア）であり、約−240 Eloに相当する。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は797秒である。

## 結論

STCが`H0`であり、段階ゲートの振分け規則によりLTCへ進めず、FM候補は不採用とする。
得点率19.6%は「改善があるとは言えない」ではなく明確な劣化であり、[候補の診断](fm-candidate-diagnostics.md)で検証損失と教師誤差が全局面帯で改善していたことと対照的である。
