# 先読み教師で再学習した学習PSTの長時間測定（LTC、中止）

## 目的

[先読み教師値](../plans/lookahead-teacher.md)の候補を、[STC](lookahead-teacher-pst-stc.md)のH1を受けて段階開始版と長時間の時間制御GSPRTで測る予定であった。

## コマンドライン

```console
target/release/match_runner \
  --run-dir data/matches/lookahead-teacher-pst-ltc --seed 45000921 \
  --candidate commit:a7e32a846e7327600d5ffe2b97e06abc672625d3 \
  --baseline commit:6c5c559a084704d66ff9b5366efa22eb946601fb \
  --each time=60000+200 gsprt
```

## エンジン

候補は `lookahead-pst` の `a7e32a8`、基準は段階開始版 `6c5c559`、規則は `engine-default`、両エンジンとも `Threads=1`、`USI_Hash` 256 MBである。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数19。
2026年9月23日0時9分に開始し、0時50分に利用者の指示で中止した。

## 結果

中止の時点で有効104ペア、ペンタノミアル度数[16, 2, 47, 4, 35]、LLR +1.06、判定保留であった。
中止した測定なので、判定には使わない。

## 結論

候補は、STCのH1をもって利用者の決定により既に採用されており、LTCは記録のために走らせていたが、測定機を空けるために中止した。
実行ディレクトリは保持し、再開はしない。
