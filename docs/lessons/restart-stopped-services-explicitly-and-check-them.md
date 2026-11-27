# 測定のために止めたサービスは、trapに頼らず明示的に起動し直し、起動したことを確かめる

## 症状

第3期の高速化のサイクル数の対比較では、測定のたびにlishogiのBotのコンテナを止め、終了時にbashの`trap '…' EXIT`で起動し直すスクリプトを使った。
測定はすべて正常に終わったが、Botは最初の測定が止めた17時40分から、利用者に指摘された22時20分まで、約4.7時間止まったままだった。その間に、Botを動かしたまま行う予定だった段階ゲートのSTCとLTCの一部も、Botを止めた状態で走った。

## 原因

作業に使ったエージェントの実行環境のbashでは、`EXIT`の`trap`が実行されなかった。`bash -c 'set -euo pipefail; trap "…" EXIT; echo body'`で確かめると、本体は動くがtrapの処理は動かなかった。
スクリプトは`docker start`の出力を捨てており、起動し直したかを確かめる手順もなかったので、気づかなかった。

## 以後の規則

測定のためにBotなどの常駐サービスを止めるスクリプトは、起動し直しを`trap`に頼らず、本体の成否を変数に受けてから明示的に起動し直して終了状態を返し、スクリプトの終了後に`docker ps`でサービスが動いていることを確かめる。

## 出典

- [第3期の高速化のSTCの記録](../measurements/movegen-speedup-3/movegen-speedup-3-stc.md)
- [直した対比較のスクリプト](../measurements/movegen-speedup-3/movegen-speedup-3-baseline/measure.sh)
