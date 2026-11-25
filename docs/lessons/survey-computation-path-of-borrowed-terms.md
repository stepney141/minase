# 先行実装から評価項目や教師生成の処理を借りるときは計算経路と前提も調べる

## 症状

棋力向上段階9では、先行エンジンが使う王の周囲の攻撃域と守備駒に関する3つの評価項目を、升ごとの疑似利きの逆引きで実装した。重みがすべて0の構成でも、毎秒の探索ノード数は項目1と項目3の計算で55.1%、3項目すべてで82.2%下がり、この2項目は学習と採否測定へ進む前に見送りとなった。参考にした先行エンジンは、同じ項目を実用的な速度で動かしている。

教師生成では、先行するチェスのエンジンに合わせて、王手を受けている局面の除外と大差の対局の打ち切りを入れた。その教師で学んだ評価関数のネットを使うv3系は、外部の中将棋エンジンHaChuとの対局で、優勢から勝ち切るまでの手数がv2.6.0より長くなった。原因を調べる中で、2つの処理の前提がminaseで成り立つかを採用時に確かめていなかったことが分かり、処理を外して比べる設計書の起案に至った。

## 原因

事前の調査では、先行エンジンが何をしているかを調べたが、それが依存する土台と前提を調べなかった。
評価項目では、Fairy-Stockfish、やねうら王、およびHaChuが、駒ごとの利きの集合や升ごとの利き数の表を評価の外で作るか他の処理と共有して費用を払っていた。minaseにはその土台がなく、1回22 nsから33 nsの逆引きを1評価あたり最大96回呼んだ。現行の評価は1ノードが約0.47 μsと軽いので、低下の割合が大きく現れた。
教師生成では、王手の局面を除くチェスのエンジンは、探索中に王手を受けた局面で評価関数を呼ばない。minaseの静止探索は王手を受けた局面でも静的評価で探索を打ち切るので、除いた種類の局面の評価を探索で使っていた。大差の対局の打ち切りも、勝勢から勝ち切るまでが短いチェスを前提にしており、中将棋では打ち切った後の局面が多く教師から欠ける。

## 以後の規則

先行実装から機能や処理を借りる設計書と設計判断の記録（ADR）では、機能ごとに、先行実装が依存するデータ構造、その費用を払う時点、他の処理との共有、およびその処理が前提にする探索の振る舞いと対局の性質を一次情報で調べ、minaseで同じ土台と前提が成り立つかを照合してから採否と定式化を決める。
成り立たなければ、土台を先に作る案、前提に合わせて変える案、および借りない案の費用と影響を実装前に比べる。

## 出典

- [plans/strength-stage9.md](../plans/archived/strength-stage9.md) の目的の節の第1の発見と、設計判断の土台の表。
- [measurements/strength-stage9/strength-stage9-cost-bench.md](../measurements/strength-stage9/strength-stage9-cost-bench.md)
- [proposals/archived/evaluation-terms-survey.md](../proposals/archived/evaluation-terms-survey.md)
- [audits/mating-speed-against-hachu-2026-11-25.md](../audits/mating-speed-against-hachu-2026-11-25.md)
- [research/teacher-generation-filters-prior-art.md](../research/teacher-generation-filters-prior-art.md)
- [plans/nnue-teacher-without-adjudication.md](../plans/nnue-teacher-without-adjudication.md)
