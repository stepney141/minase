# torchrunのワーカーは、torchrunのプロセスグループとは別に止める

## 症状

学習器の強制終了と再開の試験で、`torchrun`を`start_new_session=True`で起動し、チェックポイントが現れた時点で`os.killpg(<torchrunのPID>, SIGKILL)`を送った。
`torchrun`は止まったが、ワーカーは学習を続けてチェックポイントを書き続け、再開した学習は強制終了の時点の4回目ではなく、60回目や72回目から始まった。
残ったワーカーの検査も、`torchrun`と同じプロセスグループのプロセスだけを数えていたので、これを見逃した。

## 原因

PyTorch 2.11.0の`torchrun`は、各ワーカーを新しいセッションで起動する（`torch/distributed/elastic/multiprocessing/subprocess_handler/subprocess_handler.py`の66行の`start_new_session=True`）。
ワーカーは`torchrun`のプロセスグループに属さないので、そのグループへの信号は届かない。
`torchrun`が死ぬと、ワーカーはinitなどへ付け替えられ、親のPIDでも探せなくなる。

## 以後の規則

`torchrun`で起動した学習を止めるときは、止める前に`/proc`で親のPIDをたどって子孫のPIDをすべて集め、`torchrun`と各ワーカーのプロセスグループへそれぞれ信号を送る。
止めた後は、集めたPIDのどれもゾンビ以外の状態で残っていないことを確かめる。

## 出典

- [複数GPU化の設計書](../plans/alphazero-multi-gpu.md)のフェーズ2の試験（`tools/train/zero/tests/cli.py`の`kill_training`）
- [作業機の測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-workstation.md)の再開の確認（`docs/measurements/alphazero-multi-gpu/resume-check.py`）
