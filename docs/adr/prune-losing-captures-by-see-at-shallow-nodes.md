# 浅い非PVノードでSEEの損が余裕値を超える捕獲手を展開しない

## ステータス

承認済み。決定日は2026年9月21日。

## コンテキスト

[棋力向上段階4](../plans/strength-stage4.md)で入れたfutility pruningは、駒の増減がない手にだけ静的評価を上界として使えるので、捕獲手を対象にできなかった。
[棋力向上段階8](../plans/strength-stage8.md)の[起案時の発動率診断](../measurements/strength-stage8-activation-diag.md)では、浅い非PVノードで検査する捕獲手の39.8%が、規則に依存せずSEE（到達升での駒の取り合いを見積もる静的交換評価）で負と判定された。
余裕値は、チェスや本将棋の値を流用せず、深さごとに「失う良い結果の割合が10%以下の候補のうち枝刈りが最も多いもの」として[診断](../measurements/strength-stage8-pruning-bench.md)から選んだ。
残り深さ2では余裕値0、50、100の失う割合が10%を超えたので、歩兵2枚分の200を選んだ。
STCは470有効ペアでLLR +2.9685の`H1`、LTCは791有効ペアでLLR +2.9905の`H1`であり、異常は0件だった（[STC](../measurements/strength-stage8-see-capture-stc.md)、[LTC](../measurements/strength-stage8-see-capture-ltc.md)）。

## 決定

[浅いノードの前向き枝刈りの適用条件](skip-static-eval-pruning-when-a-royal-is-attacked.md)を満たすノードで、SEEの値が深さ別の余裕値の符号を反転した値を下回る捕獲手を展開しない。
置換表の記録手と相手の最後の王駒を取る手は対象にせず、獅子の捕獲規則に依存してSEEが判定不能になる手は展開する。

## 検討した代案

静止探索と同じ余裕値0の判定を全深さで使う案は、残り深さ2で失う割合が13.1%となり基準を超えたので採らなかった。
余裕値を深さによらず1つにする案は、診断で深さごとに適格な値が異なったので採らなかった。

## 結果

余裕値は`crates/minase/src/search/alphabeta/params.rs`の`SeeMargin1`から`SeeMargin3`であり、段階8の後にSPSAで調整された。
SEEの判定契約は`crates/minase/src/search/alphabeta/see.rs`の`see_prunes`にある。
判定不能の手を枝刈りしないので、獅子が絡む交換の誤った枝刈りは起こらないが、その分だけ発動は減る。
