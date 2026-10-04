# 既存エンジンとminaseのビットボードの比較

## 結論

Stockfishとやねうら王のビットボードを一次資料で調べ、minaseの実装と比べた。
minaseのビットボードは、升の集合を固定長の整数で表し、走り駒の利きを算術で求める点で、両エンジンと同じ系統に属する。
一方で、盤の大きさと中将棋の規則から、次の3点で異なる設計をとる。

1. 集合の語構成と升番号の付け方が異なる。minaseは144升を16升幅の段で3語に収め、横線は1語に収まるが、縦線と斜線は最大3語にまたがる。
2. 走りの利きを方向単位で計算する。中将棋の走りの方向の組合せは小さな合成駒種へまとめられないため、Stockfishとやねうら王のように直線の両方向を一括して合成駒集合と掛け合わせる形をとれない。
3. 王手とピンに関する派生構造を持たず、代わりに仮想盤面と獅子のための表を持つ。王駒を相手の利きへ置く着手を合法とする規則（RULES.md第8条第3項から第5項）と、足を仮想的な盤面で判定する規則（第13条第4項）の帰結である。

3点目が最も大きな差であり、実装の省略ではなく規則から生じている。
既存エンジンから取り入れる余地は、未測定の3案（方向ごとの走り駒集合、占有の反転による逆方向の処理、および筋優先の語配置）に残る。

## 調べた版

| 実装 | コミット | 備考 |
|---|---|---|
| Stockfish | `17a6c8f1eb0da45c2ca405321919519bf4e211ba` | 手元の複製である。 |
| やねうら王 | `c1b80eaa09fe13d5f12b1599d1ae4d53c224de30` | 手元の複製に改変はない。 |
| Fairy-Stockfish | `2e591089558a5afa72ab5a22192208e71848a30c` | 大盤版（`LARGEBOARDS`）を中心に調べた。 |
| HaChu | `822d512180b7d94bb85a55f871445b30393f7f8a` | Debianの収録版であり、升番号の比較にだけ用いた。 |
| minase | `a05478a67e8280d6d019549cfb2f734b76597385` | master |

外部実装の行番号は上記の版による。
Rustの将棋ライブラリyasaiも調べたが、ビットボードの定義を外部クレート`shogi_core`に委ねており、そのソースを確認できなかったため比較から外した。

## 集合の表現

4実装のビットボードは、升の集合を固定長のビット列で表す点で共通するが、語の数と升番号の付け方が異なる。

| 項目 | Stockfish | やねうら王 | Fairy-Stockfish（大盤版） | minase |
|---|---|---|---|---|
| 型 | `u64`の1語 | `u64`の2語と`__m128i`の共用体 | `unsigned __int128` | `[u64; 3]` |
| 升数と格納ビット数 | 64升、64ビット | 81升、128ビット | 最大12筋×10段の120升、128ビット | 144升、192ビット |
| 升番号 | 段優先（A1=0、`(r<<3)+f`） | 筋優先の縦型（1一=0） | 段優先 | 段優先の16升幅（`rank<<4 \| file`） |
| 空けておくビット | なし | 語0のbit 63 | なし | 各段の筋12から15、計48ビット |

各実装の定義は、Stockfishが[types.h 113行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/types.h#L113)、やねうら王が[bitboard.h 30行から55行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.h#L30-L55)、Fairy-Stockfishが[types.h 113行から222行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/types.h#L113-L222)、minaseが[bitboard.rs](../../crates/minase-core/src/board/bitboard.rs)と[square.rs](../../crates/minase-core/src/board/square.rs)にある。

やねうら王とminaseは、盤外のビットを空けておく点が共通する。
やねうら王は、bit 63を空ける理由を「香の利きや歩の打てる場所を求めやすくする」と説明し、Aperyなどのmagic bitboard派の考案によると記す（[bitboard.h 44行から46行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.h#L44-L46)）。
筋優先の配置では1本の筋が必ず1語に収まるので、香と飛車の縦の利きを1語の減算で求められる。
2語の境界は、升が7筋以内かどうかで決まる（[bitboard.h 141行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.h#L141)）。

minaseの空きビットは段ごとに4ビットあり、HaChuの番兵つき盤配列（hachu.c 68行から72行の盤幅の定義と、93行の`STEP(X,Y) = BW*(X)+(Y)`）と同じ形の升番号を、ビット位置にもそのまま使うことから生じる。
1語に4段がちょうど収まるので、横線は必ず1語に収まるが、縦線と斜線は最大3語にまたがる（[magic bitboardの適用の調査](magic-bitboard-feasibility.md)）。
中将棋では、縦の走りを持ち横の走りを持たない駒が香車、反車、竪行、白駒、鯨鯢、飛鹿、飛牛の7種あるのに対し、その逆は横行と奔猪の2種にとどまる（RULES.md第9条、第10条）。
やねうら王と同じ筋優先の3語配置のほうが有利かどうかは測定しておらず、現時点では仮説にとどまる。

Fairy-Stockfishは128ビットの大盤版を持つが、盤の上限は12筋×10段である（[types.h 494行から564行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/types.h#L494-L564)）。
手元の版には中将棋、獅子、および居喰いの定義もないため、12×12の中将棋はこの実装では扱えない。

## 走り駒の利きの求め方

走り駒の利きの求め方は、表を引く系統と算術で求める系統に大きく分かれる。
表を引く系統は、占有から添字を作って事前計算した利きを引く。
AVX2を有効にしないx86版などのその他のビルドでStockfishが使う乗算マジック（添字は`((occ & mask) * magic) >> shift`、[attacks.h 146行から165行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/attacks.h#L146-L165)）と、Fairy-StockfishのマジックまたはPEXT（Parallel Bits Extract、マスクしたビットを下位へ詰める命令）がこれにあたる。
算術で求める系統は、減算の桁借りで最初の遮蔽駒までを切り出す。
StockfishのARM版とAVX2（Advanced Vector Extensions 2）を有効にしたx86版のHyperbola Quintessence（[attacks.h 30行から139行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/attacks.h#L30-L139)）、やねうら王のQugiy方式、およびminaseがこちらに属する。

Stockfishの現行版は、PEXTで添字を作る経路を削除している（コミット`99489f57dddb121e1db887d35561ea58abd4158a`「Simplify out pext attacks」、2026年7月3日）。
[Magic bitboardの一次資料](magic-bitboard-primary-sources.md)がPEXTの経路を持つと記したのは、それ以前の版（`e0bfc4b`）である。

### 同じ算法を使う縦の利き

minaseの[`sliding_control`](../../crates/minase-core/src/attacks/tables.rs)は、方向ごとの利き線表を引いて占有との積をとる。
生値が増える方向では、3語を借り伝播つきで減算して`ray & (b ^ (b - 1))`を作る。
生値が減る方向では、最上位の遮蔽ビットを求め、その升から先の利き線を除く。

やねうら王の香の利きも、有効な`#if 1`側の実装（[bitboard.h 871行から931行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.h#L871-L931)）で同じ組合せを使う。
一方の手番は`em ^ (em - 1)`の減算（893行、907行）で、他方は`MSB64`による上位のマスク（920行、927行）で利きを求める。
飛車の縦の利き（[942行から977行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.h#L942-L977)）も同じ形である。
したがって縦の利きに限れば、両者はほぼ同じ算法であり、違いは語が1つか3つかにある。
なお、`#if 0`側（815行から869行）に残る加算による版は無効化されている。

### 逆方向の処理と計算の単位

同じ算術系統の中でも、逆方向の処理と計算の単位で差が出る。

やねうら王のQugiy方式は、飛車の横の利き（[bitboard.cpp 856行から890行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.cpp#L856-L890)）と角の利き（[893行から922行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/bitboard.cpp#L893-L922)）で占有をバイト反転する。
反転によって減る方向を増える方向へ変えるので、1回の減算で両方向を処理できる。
角は、SIMD（1命令で複数の値を処理する命令群）の256ビット型で4方向をまとめて計算する。
minaseの減る方向は、最上位ビットの探索と2回目の表引きに依存する。
第2期の高速化の段階4では、減る方向を`leading_zeros`と番兵表で書き直す案を試したが、親比1.0039倍で採用しなかった（[第2期の設計書](../plans/movegen-speedup-2.md)、[段階4の測定](../measurements/movegen-speedup-2-stage4-bench-depth5.md)）。
バイト反転による案は試しておらず、効果は測定していない。

計算の単位も異なる。
Stockfishとやねうら王は、飛車と角の両方向を一括して計算し、合成した駒集合と掛け合わせる。
Stockfishの`attackers_to`は`rookAttacks & pieces(ROOK, QUEEN)`の形で書かれ（[position.cpp 643行から648行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/position.cpp#L643-L648)）、やねうら王も`GOLDS`、`HDK`、`BISHOP_HORSE`、および`ROOK_DRAGON`などの合成駒種を持つ（[types.h 563行から568行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/types.h#L563-L568)）。
中将棋では走りの方向の組合せが28通りの動きの定義に分かれるため、このような小さな合成駒種へまとめられない。
そこでminaseは、1方向ずつ利きを計算し、最初の遮蔽駒の動きの定義を8ビットの方向マスクで照合する（[control.rs](../../crates/minase-core/src/movegen/control.rs)の`ordinary_attackers_to`、[tables.rs](../../crates/minase-core/src/attacks/tables.rs)の`slide_directions`）。
[第3期の設計書](../plans/movegen-speedup-3.md)が起案した「方向ごとの走り駒集合」は、Stockfishの合成集合を方向単位へ一般化したものである。
同書は、逆引きの走り計算の90%が該当する走り駒を1枚も見つけないことを根拠に挙げるが、まだ起案の段階にある。

### 表の容量と測定済みの比較

表の容量は、Stockfishのマジック表が0x19000＋0x1480＝107,648要素、8バイトずつで約841 KiBである（[attacks.cpp 157行から158行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/attacks.cpp#L157-L158)）。
minaseの利き計算の表は、利き線表が36 KiB、固定利き表と遮蔽なしの到達範囲表が2色×28定義×192升×24バイトで各252 KiB、5×5の近傍表と獅子の跳び先表が各4.5 KiBであり、合計は約549 KiBになる。

minaseでも、マジックと小さな表の試作は済んでいる。
どちらも現行の算術方式（1方向あたり中央値6.21 ns）を上回らず、主な原因は関数のインライン展開が失われたことだった（[試作の測定](../measurements/magic-bitboard-prototype.md)、[magic bitboardの適用の調査](magic-bitboard-feasibility.md)）。
したがって、minaseが算術系統に属するのは192ビットの盤で表が大きくなるという消極的な理由だけによるのではなく、実測に基づく選択である。

## 局面の表現と派生構造

局面の骨格は3実装で共通する。
いずれも、全体の占有、色別の集合、および駒種別の集合と、升ごとの駒コードの配列を二重に持ち、駒の配置と除去のたびに両方を更新する（Stockfishの[position.h 220行から221行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/position.h#L220-L221)、やねうら王の[position.h 1132行から1135行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/position.h#L1132-L1135)、minaseの[position/mod.rs](../../crates/minase-core/src/position/mod.rs)と[placement.rs](../../crates/minase-core/src/position/placement.rs)）。
minaseは駒種別の集合を先後別に持ち、29種×2色の配列とする。
巻き戻しは、Stockfishの`StateInfo`の連鎖ではなく、着手ごとに返す`Undo`の値から駒を戻す方式である（[make_move.rs](../../crates/minase-core/src/position/make_move.rs)）。

最も大きく異なるのは、局面に付随する派生構造である。
Stockfishとやねうら王は、`StateInfo`に王手をかけている駒の集合`checkersBB`、ピンの関係を表す`blockersForKing`と`pinners`、および王手になる升の集合`checkSquares`を持つ（Stockfishの[position.h 60行から65行](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/position.h#L60-L65)、やねうら王の[position.h 131行から160行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/position.h#L131-L160)）。
さらに2升間の升の集合を引く`between_bb`と`line_bb`の表を持ち、擬似合法手の生成、`legal()`によるピンの判定、および王手回避の生成を組み立てる。

minaseには、これらの構造が1つもない。
RULES.md第8条第3項から第5項は、自分の王駒を相手の利きへ移す着手と王手を解消しない着手を合法と定めるので、ピンと王手による合法性の絞り込みを要しないからである。
代わりにminaseは、中将棋の規則が要求する次の構造を持つ。

- **仮想盤面**：着手の途中と着手後の占有を差分で表す`VirtualBoard`（[virtual_board.rs](../../crates/minase-core/src/movegen/virtual_board.rs)）。第13条第4項が、獅子を取った直後の仮想的な盤面で足を判定すると定めるために要る。
- **任意の占有を受け取る逆引き**：`attackers_to_by`（[control.rs](../../crates/minase-core/src/movegen/control.rs)）。静的交換評価と足の判定の両方が使う。
- **獅子のための表**：獅子の跳び先`lion_jumps`と、固定利きの逆引きに使う5×5の近傍`neighbourhoods`（[tables.rs](../../crates/minase-core/src/attacks/tables.rs)）。
- **成り権の保留集合**：成り権を保留している駒の升の集合`promotion_deferred`（[position/mod.rs](../../crates/minase-core/src/position/mod.rs)）。

Stockfishの`attackers_to`も占有を引数に取り、静的交換評価で遮蔽駒の背後の攻撃者を扱う点はminaseと共通する。
ただしminaseでは、同じ仕組みを獅子の捕獲制限という合法性の判定にも使う。

升ごとの利き数を差分更新する表は、既定の構成ではどの実装も持たない。
やねうら王の`LONG_EFFECT_LIBRARY`は、標準のNNUEの構成では無効であり、実験用の評価を選んだときだけ有効になる（[config.h 294行、400行、415行、719行](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/config.h#L415)）。
この点は[評価項目が依存する土台の調査](evaluation-infrastructure-survey.md)が詳しく整理している。

## minaseへの示唆

以上をまとめると、minaseのビットボードは、やねうら王と同じ算術系統の利き計算、Stockfishと同じ二重表現の局面、およびHaChuと同じ番兵つきの升番号の組合せである。
チェスと将棋のエンジンに特有の王手とピンの機構を外し、その代わりに獅子の規則が要求する仮想盤面と逆引きを持つ。

既存エンジンから取り入れる余地は、測定していない次の3案に残る。

1. 方向ごとの走り駒集合。Stockfishの合成駒集合を方向単位へ一般化し、逆引きで該当する走り駒のいない方向の計算を省く。[第3期の設計書](../plans/movegen-speedup-3.md)が起案している。
2. 占有の反転による逆方向の処理。やねうら王のQugiy方式と同じく、減る方向を増える方向へ変えて2回目の表引きをなくす。段階4で不採用になった`leading_zeros`の案とは別の方式であり、効果は測っていない。
3. 筋優先の語配置。縦にだけ走る駒種が多いことを利用する。影響がビットボードを使う全処理に及ぶので、試すとしても最後になる。

どの案も、採否は既存の手順（固定入力での速度比較の後、コミット同士の自己対局による逐次確率比検定）で決める。
第1案と第2案は同じ逆引きの処理に効くため、改善率を足し合わせて見積もることはできない。
