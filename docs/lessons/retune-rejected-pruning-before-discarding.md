# 不採用の枝刈りは数え方を点検し、係数を調整し直してから捨てる

## 症状

段階4、段階8、段階12で、reverse futility pruning、late move pruning、razoringなど8項目の改良が、診断や他エンジンから選んだ係数のまま1回のSTCで`H0`となり、不採用になった。
late move pruningは得点率29.8%（約−149 Elo）と大きく負けたが、benchで減ったノードは18%だけだった。
8項目を戻して既存の探索係数とまとめてSPSAで調整し直した版は、Mに対してSTCとLTCでともに得点率約67%の`H1`となり、8項目を取り除いた版にもSTCで`H1`となった。

## 原因

late move pruningは、捕獲手も数える手番号を上限と比べ、killer手も切っていた。
中将棋は捕獲手が多いので、上限12の枠の約3分の2を捕獲手が占め、静かな手を1手も読まないうちに切り始めるノードが36%あった。
チェスでは捕獲手が少ないので、同じ数え方でも静かな手の枠が残る。
ほかの項目は、1段の診断や他エンジンの換算で選んだ係数が、既存の係数との相互作用を含めて最適から遠かった。

## 以後の規則

対局で負けた枝刈りは、手法を捨てる前に、数える対象と保護する手が中将棋の手の構成で意図どおり働くかを`bench`の計数で点検する。
係数を持つ改良の採否は、係数を既存の係数とまとめて調整し直した候補で判定し、帰属は既存の係数を揃えたまま改良だけを取り除いた版との対局で確かめる。

## 出典

- [plans/search-revival-spsa.md](../plans/search-revival-spsa.md)
- [measurements/search-revival-lmp-diag.md](../measurements/search-revival-lmp-diag.md)
- [measurements/search-revival-tstar-vs-m-ltc.md](../measurements/search-revival-tstar-vs-m-ltc.md)
- [measurements/search-revival-tstar-vs-tprime-stc.md](../measurements/search-revival-tstar-vs-tprime-stc.md)
