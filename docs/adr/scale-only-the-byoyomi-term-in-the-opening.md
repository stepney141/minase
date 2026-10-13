# 序盤の係数は予算式の秒読みの項にだけ掛ける

## ステータス

承認済み。決定日は2026年9月18日。

## コンテキスト

GitHub issue #7は、持ち時間5分と秒読み10秒の対局で、minaseが最初の数手に各10秒以上を使い、持ち時間をすぐ使い切ると報告した。
[持ち時間の効率的な使用](../plans/time-management-efficiency.md)の[初形の実測](../measurements/time-management-efficiency-probe.md)では、この条件の初手の標準予算softは9,333 msであり、秒読みだけの局面のsoftは8,000 msだった。
つまり、初手の予算の大半は秒読みの項`0.8·byoyomi`から来ていた。
既存エンジンの調査では、Apery、技巧、およびdlshogiが序盤の消費を手数に応じた係数または除数で絞り、Aperyは20手目まで秒読みの項を加えない（[秒読みつき時間制御における既存エンジンの持ち時間配分](../research/time-management-byoyomi-survey.md)）。
エンジンの開発者は、人間との対局のためのこの変更で棋力を落とさないことを条件にした。

## 決定

予算式のsoftの秒読みの項に、序盤の係数`w = min(1, (ply + 4) / 40)`を掛ける。
持ち時間を使い切った後の手、hardの式、加算の項、および持ち時間の分担には係数を掛けない。
式は`clock_budget_with_margin`のdocコメントにある。

## 検討した代案

Aperyのように序盤で秒読みの項を外す案は、項の有無を切り替える分岐が増える一方、序盤を絞る効果は係数1つで足りるので採らなかった（[同じ調査](../research/time-management-byoyomi-survey.md)）。
持ち時間の分担を含む予算全体に係数を掛ける案は、加算つきの時間制御でも着手を変え、棋力が変わらないことを確かめていないので採らなかった。
持ち時間を使い切った後の手にも係数を掛ける案は、未使用の秒読みを持ち越せない時計では序盤を絞る利益がないので採らなかった。

## 結果

秒読みのない時計では`byoyomi = 0`なので式が変わらず、標準の測定条件の棋力も変わらない。
このため採否はGSPRTではなく、秒読みのない入力で予算値が変わらない単体テスト、固定深さのbench総ノード数の一致、および時計の診断で判定した。
[時計の診断](../measurements/time-management-opening-coefficient-diag.md)では、issueの条件の初手は11,891 msから4,010 msになり、時間切れは0件だった。
秒読みつきの参考測定では、有効290ペアの点推定が−1.2 Elo、95%信頼区間が−28.4〜+26.0 Eloだった（[記録](../measurements/time-management-opening-coefficient-elo.md)）。
masterを候補、変更を含む統合ブランチを基準にした非劣性のGSPRTは有効3,574ペアでLLR −2.989の`H0`であり、10 Elo規模の低下は検出されなかった（[記録](../measurements/time-management-opening-coefficient-noninferiority.md)）。
始値0.1と長さ40 plyはAperyの値を借りた設計上の判断値であり、測定で調整した値ではない（[同じ調査](../research/time-management-byoyomi-survey.md)）。
Lishogi-Botは各対局の最初の1手を`movetime 1000`で探索させるので、lishogiでは係数が2手目以降の長考を絞る（[USI原典仕様とlishogi系拡張の調査](../research/protocols/usi-lishogi.md)）。
参考測定は評価関数がPSTとFMだった時期の構成で行ったので、係数の値を変える提案には現行の構成で秒読みつきの測定をやり直す必要がある。
