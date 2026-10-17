# strength-stage6-coefficients-stc

## 目的

棋力向上段階6の係数の再導出（[strength-stage6-coefficients-diag](strength-stage6-coefficients-diag.md)）で唯一基準を超えた`MIN_MOVES = 130`（残り手数見積りの下限を100から130へ）を、最終構成と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-coefficients-stc --seed 20680903 \
  --candidate commit:ff0a86f --baseline commit:8f4e41c \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミットff0a86f、基準は最終構成のコミット8f4e41c（aspiration windows、internal iterative reduction、最善手安定時の早期終了）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ30 GB）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16である。
本セッションからは他の対局や診断を走らせていないが、2026年9月11日20時00分と22時00分から22時19分にかけてカーネルのOOM killerが動作し（`journalctl`。`chrome`プロセスが回収されている）、同時に進行中の全対局が数秒から37秒停止した。

## 結果

1,408ペアを実行し、有効ペア1,369、破棄ペア39（手数上限）であった。
ペンタノミアル度数は[306, 26, 725, 36, 276]、LLRは−2.945で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、および拒否着手は0件だが、`time_forfeits`は128件（候補側56件、基準側72件）であり、経過時間は11,801秒である。

時間切れ128件は上記の2つの時間帯に集中し、時間切れとなった手はいずれもエンジンが完了反復の経過時間を中央値56msで報告しているのに対し、実測思考時間は1.5〜36.9秒（中央値12.3秒）であった。
思考時間が5秒を超える手はこれらの時間帯以外に存在しない。
時間切れは基準側に多いので、該当ペアを除いても候補の得点は下がる方向であり、`H0`の判定は変わらない。

## 結論

短時間GSPRTは`H0`であり、`MIN_MOVES = 130`は不採用とする。
実装はコミット36a8455で戻し、`MIN_MOVES`は100のままとする。
時間切れは外部のメモリ圧迫によるものであり、以後の測定は同時対局数を12へ下げて行う。
