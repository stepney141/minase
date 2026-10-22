# 反復の延長信号を使わず、最善手が安定したときだけ早く止める

## ステータス

置き換え済み。決定日は2026年9月12日。[目安時間softを、探索の開始時に求めた基礎予算に探索の不安定さの4因子の積を完了反復ごとに掛けて伸縮させ、最善手が安定したときの早期終了と短い詰みでの停止を併用する](../scale-the-soft-limit-by-search-instability-and-keep-the-stable-best-move-rule.md)によって置き換えられた。

## コンテキスト

時間管理は、反復の完了時に、標準予算softと上限hardから次の反復へ入るかを決める。
[棋力向上段階6](../../plans/archived/strength-stage6.md)は、この判断に3つの信号を1項目ずつ加えて測った。
1つ目は、aspiration windowsの窓が評価値の低下で外れたこと（fail-low）をその手の探索が終わるまで記録し、その間はsoftによる打ち切りの条件を外す延長である。
2つ目は、完了した反復の最善手が直前の反復と異なるときに、直後の判断1回だけsoftの条件を外す延長である。
3つ目は、直近4反復の最善手が同じときに、次の反復の予測完了時刻がsoft以下の場合に限って続ける早期終了である。
fail-lowによる延長はSTCが292有効ペアでLLR −2.945の`H0`だった（[測定記録](../../measurements/strength-stage6/strength-stage6-faillow-stc.md)）。
最善手交替時の延長はSTCが700有効ペアでLLR −2.960の`H0`だった（[測定記録](../../measurements/strength-stage6/strength-stage6-bmchange-stc.md)）。
最善手安定時の早期終了は、延長の信号を持たない形で実装し直して測り、STCが275有効ペアでLLR +2.988、LTCが574有効ペアでLLR +2.957の`H1`だった（[STC](../../measurements/strength-stage6/strength-stage6-stable-stc.md)、[LTC](../../measurements/strength-stage6/strength-stage6-stable-ltc.md)）。

## 決定

反復の継続判断に、fail-lowと最善手の交替による延長を使わない。
直近4反復の最善手が同じ場合だけ、softの条件を「経過時間がsoft未満」から「予測完了時刻がsoft以下」へ置き換え、hardの条件はそのまま守る。

## 検討した代案

fail-lowによる延長と最善手交替時の延長は、上記のとおりSTCで`H0`だったので採らなかった。
fail-highによる延長は、fail-highが評価値の上がる兆候であり最善手を急いで確定しても損が小さいので、項目として立てなかった。

## 結果

反復数4を選んだ理由は、`crates/minase/src/search/alphabeta/time.rs`の`STABLE_ITERATIONS`のdocコメントにある。
継続判断の関数は延長の真偽値を受け取らない。
延長を再び提案する場合は、別の信号として段階ゲートにかける必要がある。
この規則は持ち時間の期間に限る。秒読みの期間では、予測と最善手の安定性を使わず、経過時間がhard未満であるかだけで次の反復を始めるかを決める（`should_start_next_iteration`）。
