# hachu-time4x-gsprt

## 目的

HaChuに現行のminase（v2.2.0、コミット`203ba2a`）の4倍の持ち時間を与えた条件で、minaseが有意に強いかをGSPRTで判定する。
持ち時間の比は、[条件格子測定](../plans/hachu-condition-grid.md)の慣例に従い、HaChuを従来の比較条件60秒＋1手1秒加算に固定し、minaseをその1/4の15秒＋0.25秒加算とした。
この比は条件格子の[time1of4](hachu-grid-time1of4.md)セル（2026年9月、コミット`156816d`で−26.1 Elo）と同じである。
採否の測定ではなく、現行版の強さの確認である。

## コマンドライン

対局ハーネスは、master `203ba2a`に固定したworktree `data/worktrees/fm-scale-spsa-runner` でビルドしたバイナリ（SHA-256 `d082213736b3383325908c05368b040c8e467c349f36cb068385673eb6b4ad7e`）から起動した。

```console
data/worktrees/fm-scale-spsa-runner/target/release/minase match run \
  --run-dir data/matches/hachu-time4x-gsprt --seed 20571008 \
  --candidate commit:203ba2a7ff47cc9de9ccf11ba4196965644c7c4b --candidate-limit time=15000+250 \
  --baseline "cecp:/home/stepney141/board-games/hachu-debian/hachu" --baseline-limit time=60000+1000 \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --candidate-hash 256 --baseline-hash 256 \
  --concurrency 16 --max-ply 4096 --response-timeout 120 \
  gsprt --max-pairs 1000
```

`--max-pairs 1000`は、minaseとHaChuが互角だった場合に判定が数千ペアへ延びることを防ぐ上限であり、到達しなかった。
候補と基準の時間制御が異なるため`minase match report`は使えず、標準出力の最終サマリと`pairs/`の保存記録を記録の値とする。

## エンジン

候補はminaseのコミット`203ba2a7ff47cc9de9ccf11ba4196965644c7c4b`（v2.2.0、バイナリのSHA-256 `14bafab53504110127c49e780a6acdabdc494453da21e025fac954ec4663646e`）であり、`Threads`は1である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`で`gcc -O2`によりビルド）であり、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。
仮説は既定のH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,830,336バイト）、OSはLinuxである。
候補の`Threads`は1、HaChuはワーカー数を報告しない。
`USI_Hash`は256 MB、HaChuの`memory`は256 MB、同時対局数は明示の16、手数上限は4,096手、応答タイムアウトは120秒である。
時間制御は候補が15秒＋0.25秒加算、基準が60秒＋1秒加算である。
2026年10月8日19時59分から20時58分に実施した。
同じ機械ではlishogi Botとminase GUIのコンテナが稼働しており、開始時点ではBotが対局中（エンジン2プロセス、各約3コア）だった。開始直前まで別の測定（FM倍率のSPSA）が走っていたが、本測定の開始はその終了を待ってから行い、測定中にハーネス以外の対局、生成、学習は走らせていない。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 107（破棄0） |
| ペンタノミアル度数 | [1, 0, 13, 0, 93] |
| LLR | +2.970 |
| 判定 | H1 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 3,528.7秒（`summary.json`の累計は3,523.5秒） |

判定時に進行中だったペア108とペア109も`pairs/`に保存されているが、統計には取り込まれていない。
218局の終局理由は投了203局、王駒の捕獲15局である。
minaseの0勝2敗となったペア63は、両局とも王駒の捕獲による通常の終局であり、異常ではない。
1局の手数の中央値は315手、1局の実時間の中央値は233秒、CPU時間の中央値はminaseが41秒、HaChuが163秒である。
保存された着手時間から候補の時計を再構成すると、全218局を通じた残り時間の最小値は853 ms（局ごとの最小値の中央値は4,147 ms）であり、時間切れの余裕はあった。

参考として、107ペアのペンタノミアル度数を`minase::stats::estimate_elo`（固定局数Eloの推定器）に掛けると、正規化した平均得点は0.930、Eloは+449.1（95%信頼区間+371.4〜+577.5）である。
これは逐次検定の停止時点の標本から計算した値であり、停止規則による偏りを含むので採否や進捗指標には使わない。

## 結論

HaChuに4倍の持ち時間を与えても、現行のminase（v2.2.0）は107ペア、正規化した平均得点0.930、異常0件でH1（有意に強い）と判定され、2026年9月の条件格子で互角だった同じ時間比で明確に勝ち越した。
