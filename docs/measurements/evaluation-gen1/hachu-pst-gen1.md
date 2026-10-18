# 再学習PST対HaChuの固定局数Elo

## 目的

採用した再学習の学習PST（候補）を外部エンジンHaChu（基準）と対等な時間制御で200ペア対局させ、前マイルストーンの学習PSTのHaChu戦（Elo −282）からの改善幅を記録する。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir data/matches/hachu-pst-gen1 --seed 20260829 \
  --candidate commit:af200b4 --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 \
  --concurrency 16 elo --pairs 200
```

シードは前マイルストーンのHaChu戦と同じ`20260829`であり、開始局面を揃えている。
実行に使った`match_runner`は、対局ハーネスの効率化（`--run-dir`と再開、`match_report`）を含む作業ツリーからビルドしたバイナリ（SHA-256 `da126857f6961975f74107e1f9823a0bf26ff1f69dcfe69c323178914c033f22`）であり、その内容は後にコミット`3b35793`としてmasterへ入った。

## エンジン

候補は再学習した学習PSTのコミット`af200b4c4b38e0493dabcfa23c2b4c0b4558bf2b`であり、バイナリのSHA-256は`4f0db5ba4a19fb03766803f352bc046884840232abb3b701a4ebb300c0c3cc9e`である。
基準はH. G. Muller作のHaChuで、Debianパッケージ収録のオリジナル版コミット`df26f4a`（RULES.md［E5］）を`../hachu-debian/hachu`としてビルドし、既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）でCECPプロトコルにより対局させた。
規則セットはHaChuの既定設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminase側に与えた。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）であり、OSはLinuxである。
候補のワーカー数は`Threads=1`（HaChu側のスレッド数は記録なし）、`USI_Hash`は候補256 MB、HaChuには`memory 256`を送り、同時対局数は16、手数上限は4,096手、応答タイムアウトは120秒とした。

## 結果

2026年8月28日23時40分から29日3時30分（JST）に200ペア（破棄0）を完走した。
ペンタノミアル度数は`[89, 0, 84, 0, 27]`、Eloは−111、95%信頼区間は−150〜−75である。
`rejected_moves`、`time_forfeits`、`illegal_moves`はいずれも0件、`crashes`は2件、経過は13,768秒（3.8時間）である。
`crashes`の2件はいずれもHaChu側で、ペア40第2局（minase後手）の2,002手目とペア164第1局（minase先手）の2,001手目にHaChuのプロセスが終了したものであり、規約どおりHaChuの反則負けとして算入した。
どちらも2,000手を超えた時点で起きており、HaChuが着手履歴を固定長配列`gameMove[MAXMOVES]`（MAXMOVES=2000）に保持することから、履歴の上限を超えたことが原因と考えるのが最も自然である。
前マイルストーンでも1,199手の対局でHaChuが不正着手を返しており、HaChuは長い対局で自身の記憶範囲を超えると考えられる。
minase側の異常は0件である。

## 結論

前マイルストーンの学習PST（コミット`7e13888`）の同条件の−282（95%信頼区間−330〜−241、hachu-pst-gen0.md）から約170 Elo改善し信頼区間は重ならないので、学習分布外の規則と学習に使っていない相手に対しても再学習PSTの強さの増加が表れているが、この値は参考値であり完了条件には含めない。
