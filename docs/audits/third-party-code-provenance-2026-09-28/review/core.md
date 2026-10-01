# 盤面と規則の中核（src/core/ ほか）の対照レビュー記録

対象は minase の基準コミット a05478a の src/core/、src/rng.rs、src/eval/handcrafted.rs、削除済みの旧モジュール、および docs/measurements/magic-bitboard-prototype/ である。minase のリポジトリには LICENSE ファイルがなく、Cargo.toml と README.md にもライセンス表記がない（a05478a 時点）。

## 判定A: magic bitboard 試作の定数生成器（docs/measurements/magic-bitboard-prototype/diagonal.patch 内の tools/generate_diagonal_magics.rs）

- minase 側: a05478a:docs/measurements/magic-bitboard-prototype/diagonal.patch 836行から943行（新規ファイル tools/generate_diagonal_magics.rs、103行）。同パッチ 5行から117行の src/core/attacks/diagonal.rs（DiagonalMagic と表構築）。導入は 5a8a802（2026-09-21 "Record magic bitboard feasibility experiments"）。パッチは src へ統合されておらず（a05478a の src と tools に DIAGONAL_MAGICS は存在しない、全ブランチでも -S 検索で0件）、測定記録の添付物としてだけ存在する。
- 対照側: Stockfish sf_17:src/bitboard.cpp 140行から217行（init_magics）、sf_17:src/misc.h 158行から184行（PRNG::rand64 と sparse_rand）。ライセンスは GPL-3.0-or-later（manifest.tsv）。
- 同時期の設計資料: docs/research/magic-bitboard-primary-sources.md 12行から18行が「Stockfish 17の表生成」として bitboard.cpp#L145-L217 を名指しで参照している。
- 実装固有の一致:
  1. 乱数器が xorshift64*（右12、左25、右27のシフトと乗数 0x2545F4914F6CDD1D = 2685821657736338717）で、Stockfish の PRNG::rand64 と定数まで同一である。magic の探索には任意の乱数器で足り、minase 本体は別の xorshift64（13/7/17）と splitmix64 を使っているため、ここだけ Stockfish と同じ乱数器を選ぶ必然はない。
  2. 候補を `random & random & random` の3回論理積で疎にする。Stockfish の sparse_rand と同じである。この手法は Romstad の公開コードに由来し広く流布しているが、1との組合せで Stockfish の構成と一致する。
  3. 試行ごとに表を消去せず、`epoch[index] == attempt` で試行番号を記録して未使用を判定する。Stockfish の `epoch[idx] < cnt` による "little speed-up trick to avoid resetting m.attacks[]" と同じ仕掛けで、配列名 epoch も同じである。chessprogramming wiki の一般的な探索例は試行ごとに used 配列を初期化するため、別の書き方が自然にあり得た。
  4. 部分集合ごとに occupancy と参照値（Stockfish は reference、minase は reach）の2配列を前計算し、照合しながら表を構築する構成。
  5. 表引きの構造体が mask・magic（factor）・表内オフセット・shift を持ち、添字を offset + (occ*magic >> shift) とする "fancy" 方式。これは方式そのものに近い。
- 反証（異なる選択）: 盤端を除く mask の作り方が中将棋の単方向の利き線ごと（4方向×144升）であり、3語の占有を論理和で1語へ畳み込む（Stockfish にない独自の処理）。部分集合列挙は Carry-Rippler ではなくビット添字の走査である。Stockfish の popcount((magic*mask)>>56) < 6 による候補の足切りがない。種は升の段ごとの最適化済み表ではなく単一の固定値 0x52f1804bb57c183d である。参照値は利きの Bitboard ではなく到達距離の整数である。コードは Rust で書き直されており、逐語的な対応はない。
- 出典表示: 生成器とパッチのコメント（`^+//` 行）は "Offline generator…" などの使い方の説明だけで、Stockfish への言及はない。出典は別文書 docs/research/magic-bitboard-primary-sources.md にだけ現れる。handcrafted.rs が HaChu をコード中で明記しているのと対照的である。
- 露出の範囲: 出荷されるライブラリとバイナリには含まれない。一方、パッチは a05478a のリポジトリ木と履歴に追跡されたファイルとして含まれるため、リポジトリを配布すれば一緒に配布される。露出はリポジトリ配布の水準であり、ゼロではない。
- 判定: 1（翻案を支持する証拠あり）。乱数器の定数、疎な候補、epoch による試行番号の仕掛けという3つの実装固有の選択が Stockfish 17 の init_magics と一致し、同時期の資料が当該行を参照しているため、独立実装では説明しにくい。ただし、一致は手順と数個の定数という小さな範囲にとどまり、逐語的な複製ではない。

## 判定B: 評価関数 v0 の駒価値（src/eval/handcrafted.rs）

- minase 側: a05478a:src/eval/handcrafted.rs 12行から49行（PIECE_VALUES）。導入は 93b4bd4（2026-08-11「探索部フェーズ2の評価関数v0…」）。コメントで出典を「H. G. Muller, HaChu（https://github.com/ddugovic/hachu）の variant.c にある chuPieces[]」と明記している。
- 対照側: debian_hachu@822d512:hachu.c 191行（LVAL 1000）と 194行から224行（chuPieces[]）。ddugovic 版 /home/stepney141/board-games/hachu-github@aa2c67f:variant.c 12行から43行も同じ値である。ライセンスは両ファイルとも冒頭に "released in the public domain"（manifest.tsv: LicenseRef-public-domain）。
- 実装固有の一致: 王駒2種を除く27駒種の値が、HaChu の値を2.5倍して0.5を切り上げた値と全て一致する（例: 醉象 201→503、盲虎 152→380、金将 151→378、麒麟 154→385、鳳凰 153→383、獅子 LVAL 1000→2500）。201、152、151、154、153 のような端数は HaChu が同値の駒を区別するために与えた調整値であり、独立に同じ組が出ることはない。
- 反証: 王将・玉将と太子は HaChu の 280/270 を採らず 2600 とし、コメントで理由を述べている。PST（前進・王駒・中央性）は HaChu の評価と異なる minase 独自の式である。
- 導入時の設計書: 93b4bd4:docs/plans/search.md 27行が「HaChu（variant.cのchuPieces[]、歩兵=40基準）の駒価値を2.5倍して歩兵=100へ換算した駒割」と明記し、130行が「駒価値の推定値まで無効になるわけではない」と採用理由を述べる。a05478a:src/eval/handcrafted.rs 189行から191行のテストも、この凍結表を規範値として参照する。
- 判定: 1（複製を支持する証拠あり。ただし出典明記済みで、対照はパブリックドメイン）。数値表の転記であり、ライセンス上の制約は生じない。

## 判定C: 盤表現・升・方向（src/core/board/）

- minase 側: bitboard.rs 1行から274行、square.rs 1行から131行、direction.rs 1行から101行。初出は 4b5f390（2026-07-30、初回コミット）。
- 対照側: Stockfish@17a6c8f types.h/bitboard.h、YaneuraOu@c1b80ea source/bitboard.h 30行から55行、Fairy-Stockfish@2e59108 types.h 113行から222行、shogiops@e295794 src/square-set.ts、scalashogi@9a1c2c3、HaChu hachu.c 68行から80行。
- 一致: 升集合を固定長整数で表し、lsb/msb/pop_lsb/popcount と集合演算を持つ。これはビットボードの一般的な語彙であり、必須の一致である。
- 反証: minase は `[u64; 3]`、`rank << 4 | file` の16升幅・番兵ビット（各段の筋12から15）、段0を先手側とする座標を使う。shogiops は 16×16 を 32ビット8語で表し右上を升0とする。YaneuraOu は81升の筋優先縦型、Fairy-Stockfish は `__int128` の段優先、HaChu は 32×16 の配列と EDGE 番兵である。方向の列挙順（East, West, North, South, NE, NW, SE, SW）と `FILE_MASKS`、`VALID_WORD = 0x0fff_0fff_0fff_0fff` のいずれも対照に同名・同値がない。docs/research/bitboard-comparison.md（作業ツリーの未追跡ファイル）も同じ差を結論としている。
- 判定: 2。

## 判定D: 利き計算（src/core/attacks/）

- minase 側: sliding.rs 1行から24行、tables.rs 1行から196行、fixed.rs 1行から442行。走りの利きは方向別の利き線表と遮蔽駒の lsb/msb による classical approach で、増加方向は 505b54d（2026-09-16、段階4）で「3語を192ビット整数として1を引く」方式へ変えた。
- 対照側: Stockfish の magic/PEXT、YaneuraOu の反転と減算、shogiops の hyperbola quintessence（src/attacks.ts 49行から70行）、HaChu の方向表。
- 一致: 利き線表と最初の遮蔽駒で切る方式、`b ^ (b - 1)` による最下位ビットまでの抽出は、chessprogramming wiki に載る公知の方式であり必須の一致に当たる。
- 反証: minase は駒種を「動きプロファイル」（固定利きの相対変位と走り方向の組、特殊移動 None/Lion/LionLike）で表し、色×プロファイル×升の固定利き表、5×5近傍、獅子の2升跳び表を持つ。この分割と名前（MovementProfile、RelativeDelta、SlideSpec、reach、neighbourhoods、lion_jumps）は対照のどれにもない。shogiops は駒ごとの関数（kirinAttacks など）、HaChu は8方向の距離コード配列（chuPieces の {X,1,J,…}）で表す。テスト用 LCG の定数（2862933555777941757 と 3037000493、初版は Knuth MMIX の 6364136223846793005/1442695040888963407）と種 0x5a4f425249535401（"ZOBRIST\x01"）は公知または独自である。
- 判定: 2。

## 判定E: 駒定義・局面・make/unmake・zobrist・乱数（src/core/piece/、src/core/position/、src/core/mv.rs、src/rng.rs）

- minase 側: piece/kind.rs、code.rs（下位5ビット=駒種+1、0x20 成り、0x40 後手、0xff 番兵）、position/mod.rs（mailbox + by_color + by_kind）、make_move.rs（Undo トークン）、zobrist.rs 1行から148行、rng.rs 1行から76行。
- 対照側: Stockfish position.h/cpp と misc.h PRNG、YaneuraOu の Zobrist（seed 20151225 の xorshift64*）、rshogi@871a5b8 crates/rshogi-core/src/position/zobrist.rs 19行から81行（YaneuraOu 準拠の PRNG）、HaChu hachu.c 692行・1058行・1121行（pieceKey×squareKey の積）、scalashogi Role 一覧。
- 一致: mailbox と色別・駒種別ビットボードの併用、put_piece/remove_piece という操作名、zobrist の増分更新、手番キー（先手番0）は多くのエンジンに共通する必須の一致である。splitmix64 の3定数と xorshift64 の 13/7/17 は Vigna と Marsaglia が公表した標準定数である。
- 反証: zobrist の乱数器は Stockfish・YaneuraOu・rshogi が使う xorshift64* ではなく素の xorshift64 で、種は ASCII の "MINASEZ1"（0x4d494e4153455a31）である。キー配置は升×（色×駒種×成否）の平坦な配列に続けて手番、先獅子、麒麟成り、成り権保留、先獅子升の順に引く独自のものである。駒種の列挙順と英名（GoBetween、FerociousLeopard など Wikipedia 系の名称）は scalashogi（Queen、Elephant、成駒を別 Role とする）や HaChu と異なる。削除済みテストの種も ASCII タグ（"ORACLE"、"UNMAKE"、"PLAYOUT"）で統一された minase 独自の癖である。
- 判定: 2。

## 判定F: 合法手生成・獅子系・獅子捕獲規則（src/core/movegen/）と削除済み src/movegen/{lion,legal,normal,oracle,verification}.rs

- minase 側: a05478a:src/core/movegen/lion.rs 1行から60行、lion_like.rs 1行から75行、lion_capture.rs 1行から163行、virtual_board.rs、generate.rs、control.rs、search_captures.rs。削除済みは 161e8a3^:src/movegen/lion.rs（141行、LocalOccupancy）、161e8a3^:src/movegen/{legal,normal}.rs、937e5b9^:src/movegen/oracle.rs、3e4a12d^:src/movegen/verification.rs。初回コミット 4b5f390 の PLAN.md 277行から303行は、shogiops を規則解釈の照合先と perft 参照値の出典として挙げる。
- 対照側: shogiops@e295794 src/position/rules/chushogi.ts（143行から160行の獅子捕獲制限、211行から270行の secondLionStepDests と removeLions、lastLionCapture）、scalashogi@9a1c2c3 variant/Chushogi.scala、HaChu hachu.c。
- 一致: 獅子の2段階移動、居喰い、じっと、2升跳び、隣接獅子の無条件捕獲、足の判定、先獅子の升の記録は RULES.md 第11条から第16条の規則そのもので、必須の一致である。
- 反証: minase は着手を {from, mid, to, promote} の1値で表し、獅子の生成順は「1升移動（通常経路）→経由升で取る2段階→2升跳び→じっと」である。仮想盤面 VirtualBoard で段階ごとの占有を差分更新する。shogiops は升ごとの到達先集合 dests と第2段階の関数 secondLionStepDests を分け、lastLionCapture に升だけを持つ。minase の LionTrigger は升と麒麟成りフラグを持ち、L0〜L4 の規則切替を持つ。付け喰い（is_tsukegui）と第16条第8項の歩兵・仲人の足の扱いは shogiops にない。HaChu は駒リストと距離コードで生成し、構造がまったく異なる。
- 旧モジュール: 937e5b9^:src/movegen/oracle.rs 1行から2行は「列挙ロジックはコミット c316a44 の movegen/lion.rs から移植した」とあり、移植元は minase 自身である。3e4a12d^:src/movegen/verification.rs 12行の `sfen_conversion_uses_shogiops_coordinates_and_piece_codes` は、SFEN の座標と駒記号を shogiops と揃える試験であり（c316a44「SFENの駒記号をscalashogiの完全な対応表に揃える」）、表記仕様の一致として必須の一致に当たる。
- 試験データの逆照合: a05478a の src/core 配下の全テストと、削除済みの verification.rs・oracle.rs から SFEN 形の文字列20件を抽出し、shogiops@e295794 の test/・src/ と scalashogi@9a1c2c3 の src/test/ を検索した。一致は0件である。逆向きに、shogiops と scalashogi の試験にある12段の SFEN 334件を a05478a の src/ と tests/ で検索すると、一致は中将棋の初期局面1件だけであり、これは規則が定める初期配置である。minase の試験局面は主に座標指定のビルダーで組み立てられている。
- 判定: 2。

## 判定G: 対局裁定・反復（src/core/game/）と削除済み src/mate.rs、src/perft.rs

- minase 側: game/adjudication/bare_king.rs 1行から103行、repetition/r1.rs 1行から370行、abcc4a8^:src/mate.rs、6a90ab0^:src/perft.rs。
- 対照側: scalashogi@9a1c2c3（MIT）variant/Chushogi.scala 351行から400行（bareKing、isInsufficientMaterial）、History.scala 16行から56行（isRepetition、perpetualCheckAttacker、consecutiveAttacks）。
- 一致: E3 の条件(a)(b)(c)、R1 の「可逆手12手」と攻撃側の判定は、RULES.md 第31条・第32条が scalashogi の挙動を規則として採り入れたものであり、r1.rs 12行と263行も出典を明記している。互換性のための仕様の一致であり、必須の一致に当たる。
- 反証: minase は局面ごとの出現回数と初出手数を HashMap に持ち、攻撃連続数を配列で更新する。scalashogi はハッシュ列を後ろから走査する。bare_king.rs はビットボードと補助関数への分割で書かれ、scalashogi の collect 式と対応しない。
- 判定: 2。

## 判定H: 直前局面生成（src/core/predecessor/）、規則コード（src/core/rules/）、成り（src/core/promotion.rs）

- minase 側: predecessor/mod.rs ほか、rules/code.rs・set.rs・parse.rs、promotion.rs。
- 対照: 中将棋の直前局面生成を持つ実装はコーパスにない。規則コードは RULES.md 第29条から第33条の minase 独自の体系である。
- 判定: 2（対照に相当する実装がなく、実装固有の一致は見当たらない）。

## 読んだ対照と読めなかった対照

- Fairy-Stockfish@2e59108: 大盤版は `unsigned __int128`（src/types.h 113行から115行）で最大120升であり、src/ に獅子と2段階移動の生成はない（lion の出現は src/variants.ini のコメントだけ）。中将棋の生成の出典にはなり得ない。表の分割は docs/research/magic-bitboard-feasibility.md が参照しているが、minase の本体は分割表を採用していない。
- python-shogi@a814cbf: 中将棋の実装を持たない（chu・lion の出現は CSA.py の "CHUDAN" だけ）。比較対象がないため判定2の根拠にも反証にもならない。
- YaneuraOu@c1b80ea: 中将棋の実装を持たない。zobrist の乱数器は source/position.cpp 85行の `PRNG rng(20151225)`（xorshift64*）であり、minase の xorshift64 と種 "MINASEZ1" とは異なる。
- apery_rust@8e64bc4、rshogi@871a5b8、shakmaty@6a96078: いずれも中将棋と獅子の実装を持たない。minase の特徴的な識別子（sliding_control、king_steps、lion_jumps、captured_squares、pieces_of_kind、dense_index、SquareIter、FILE_MASKS、VALID_WORD、ZOBRIST_SEED）を検索し、shakmaty の EN_PASSANT_FILE_MASKS 以外に一致はなかった。rshogi の zobrist は YaneuraOu 準拠（seed 20151225、xorshift64*）である。
- 未確認（判定4）の対照はない。

## まとめ

| モジュール | 判定 | 理由 |
|---|---|---|
| src/core/board/（bitboard、square、direction） | 2 | 144升・16升幅・3語の配置は対照のどれとも異なり、共通点はビットボードの一般語彙だけである。 |
| src/core/attacks/ | 2 | 利き線表と `b ^ (b-1)` は公知の方式であり、動きプロファイルの分割は独自である。 |
| src/core/piece/、mv.rs | 2 | 駒コードのビット配置と駒種の列挙順は対照と一致しない。 |
| src/core/position/（make/unmake、zobrist） | 2 | mailbox と色・駒種別ビットボードは一般的であり、キー生成の乱数器と種（"MINASEZ1"）は独自である。 |
| src/rng.rs | 2 | splitmix64 と xorshift64 は公表された標準定数であり、Stockfish 系の xorshift64* ではない。 |
| src/core/movegen/（獅子系を含む） | 2 | 一致は RULES.md の規則だけで、着手の表現と生成順は shogiops・HaChu と異なる。 |
| src/core/game/（裁定、反復） | 2 | scalashogi（MIT）の挙動を規則として明記して採り入れており、式の書き方は対応しない。 |
| src/core/predecessor/、rules/、promotion.rs | 2 | 対照に相当する実装がない。 |
| 削除済み src/movegen/{lion,legal,normal,oracle,verification}.rs、src/core/position.rs、src/mate.rs、src/perft.rs | 2 | 現行モジュールの前身であり、同じ理由による。検証用の種も ASCII タグの独自形式である。 |
| src/eval/handcrafted.rs | 1（出典明記、パブリックドメイン） | 駒価値27種が HaChu chuPieces[] の2.5倍換算と一致し、コメントに出典がある。 |
| docs/measurements/magic-bitboard-prototype/diagonal.patch の生成器 | 1（試作パッチ、未統合） | xorshift64* の定数、3回論理積の疎な候補、epoch による試行番号の仕掛けが Stockfish 17 の init_magics と一致する。 |
| 同 horizontal.patch、compare.py、reproduce.py | 2 | 段の内側10升を添字とする横利き表は公知の方式であり、比較スクリプトは minase 独自の測定手順である。 |
| 同 probe.patch、trace.patch、inline.patch、force-inline.patch | 2 | 利き問い合わせの記録と再生、および `#[inline]` 属性の付け替えだけで、minase 内部の計測用の変更である。 |
