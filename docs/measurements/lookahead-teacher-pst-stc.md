# 先読み教師で再学習した学習PSTの短時間測定（STC）

## 目的

[先読み教師値](../plans/lookahead-teacher.md)の利用者の指示による候補（段階開始版の学習PSTを先読み教師で学習し直した重み）を、段階開始版と対等条件の短時間GSPRTで測り、長時間測定へ進めるかを振り分ける。

## コマンドライン

```console
target/release/match_runner \
  --run-dir data/matches/lookahead-teacher-pst-stc --seed 42000921 \
  --candidate commit:a7e32a846e7327600d5ffe2b97e06abc672625d3 \
  --baseline commit:6c5c559a084704d66ff9b5366efa22eb946601fb \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はブランチ `lookahead-pst` の `a7e32a8`（段階開始版 `6c5c559` に `nets/pst.bin` の差し替えだけを加えたコミット。重みの学習は[先読み教師での学習PSTの再学習](lookahead-teacher-pst-training.md)）、基準は段階開始版 `6c5c559` である。
規則セットは `engine-default`（L0、P0、R1、E0）、両エンジンとも `Threads=1`、`USI_Hash` 256 MBである。
runnerは `6c5c559` の `match_runner`（SHA-256 `1d4c4f6a…`）で、H1はelo=10、α=β=0.05である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は自動値の19、他の負荷はない。
2026年9月22日23時11分から23時60分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 325（破棄16、手数上限） |
| ペンタノミアル度数 | [44, 10, 166, 14, 91] |
| LLR | 2.958 |
| 判定 | H1 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 2,988秒 |

## 結論

候補は短時間測定でH1であり、異常0件なので、長時間測定 `lookahead-teacher-pst-ltc`（基本シード45000921）へ進める。
