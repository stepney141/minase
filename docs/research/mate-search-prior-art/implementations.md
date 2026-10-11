# df-pn 詰将棋ソルバの既存実装調査

本書は、中将棋エンジン minase に詰み探索を設計するための参考資料として、公開されている df-pn 系の詰将棋ソルバをソースコードの水準で調べた結果をまとめる。
**df-pn**（depth-first proof-number search）は、各節点に証明数 **pn**（詰みを証明するために解決すべき葉の数の見積り）と反証数 **dn**（不詰を証明するために解決すべき葉の数の見積り）を持たせ、しきい値つきの深さ優先探索で最も見込みのある子を選び続けるアルゴリズムである。
以下では、置換表を **TT**、置換表の不要エントリ回収を **GC**、岸本の Threshold Controlling Algorithm を **TCA**、同一局面への到達経路の違いによって結果が変わる問題を **GHI**（graph history interaction）、探索が返す最善の手順（詰将棋では詰み手順）を **PV**（principal variation）、合流のある探索グラフを **DAG**（directed acyclic graph）、将棋エンジンと GUI の通信規約を **USI**（Universal Shogi Interface）と略す。

調査日は 2026 年 10 月 11 日であり、各リポジトリは `git clone --depth 1` で取得した。
引用中のパスは、接頭辞 `R/` を各リポジトリの複製の根と読み替える（例: `R/YaneuraOu/source/mate/mate_dfpn.hpp` は YaneuraOu リポジトリの `source/mate/mate_dfpn.hpp`）。
行番号は、下表のコミットにおけるものである。

| 実装 | リポジトリ | コミット | 主な調査ファイル |
| --- | --- | --- | --- |
| やねうら王 `Mate::Dfpn` と tanuki- 詰将棋エンジン | yaneurao/YaneuraOu | c1b80eaa09fe13d5f12b1599d1ae4d53c224de30 | source/mate/、source/engine/yaneuraou-mate-engine/、source/engine/tanuki-mate-engine/ |
| KomoringHeights | komori-n/KomoringHeights | f1627f48eb5ea47f60eddf525d3e006a73a6e5cb | source/engine/user-engine/ |
| shtsume | hkijin/shtsume | c48a8fa9233df0ec9c0b3b382378d6d9a52974c9 | source/ |
| tsumeshogi-solver | sugyan/tsumeshogi-solver | e3838eb79178b9c858a94adcb3bbb1cc03bfa2b9 | dfpn/、solver/ |
| oslmate（OSL の df-pn 移植） | TadaoYamaoka/oslmate | ed13b2de55e08d9bbafb772ec9b616300d1f2d12 | osl/osl/std/osl/checkmate/、oslmate/oslmate.cpp |
| cshogi（OSL 移植と dlshogi 系 df-pn） | TadaoYamaoka/cshogi | 73f4a0da5f7ee03d121ac551a346254eb7a43608 | src/osl_dfpn.cpp、src/dfpn.cpp |

HaChu（Debian パッケージ収録版、コミット 822d512）と Fairy-Stockfish（コミット 2e59108）は、minase のリポジトリと並べて置いた複製（`../hachu-debian`、`../Fairy-Stockfish`）を調べた。

OSL 系の調査はブログ記事 3 本（2026 年 4 月 4 日、6 月 16 日、6 月 17 日）も参照した。
本書の設計判断に関わる引用（しきい値更新、TT のレイアウト、GC、千日手、出力）は、該当行を再読して確認した。

## 1. やねうら王の `Mate::Dfpn`（source/mate/mate_dfpn.hpp）

やねうら王には、ライブラリとしての詰み探索 `Mate::Dfpn` と、別系統の tanuki- 詰将棋エンジンの 2 つがある。
tanuki- 詰将棋エンジンは `mate_dfpn.hpp` を使わない。
config.h の TANUKI_MATE_ENGINE 節は `USE_MATE_1PLY` だけを定義し（`R/YaneuraOu/source/config.h:467-476`）、YANEURAOU_MATE_ENGINE 節が `USE_MATE_DFPN` を定義する（同 481-491）。

### 1.1 アルゴリズムの変種

`mate_dfpn.hpp` は、TT で合流を扱う df-pn ではなく、メモリ上に明示的な木を持つ最良優先探索である。
作者自身がファイル冒頭でこれを書いている。

```
R/YaneuraOu/source/mate/mate_dfpn.hpp:8-18
   8	古来からあるhash tableを用いた実装や、df-pn論文にあるような実装をやめて、
   9	df(disproof number)とpn(proof number)を方策として持つ最良優先探索として実装する。
  10	※　pn-searchと呼ばれるアルゴリズムに近い。
  12	また、合流を処理しないことにする。
```

子へ降りる条件は df-pn のしきい値と同じ形で、「2 番目に良い子」の値を超えたら親へ戻る。
ループ条件は次のとおりで、残り節点数が pn を下回ったら打ち切る枝刈り（作者が「やねうらお考案」と記す）も含む。

```
R/YaneuraOu/source/mate/mate_dfpn.hpp:703-716
 703	 while (
 704	    node->pn <= second_pn
 705	 && node->dn <= second_dn
 706	 && node->pn
 707	 && node->dn
 708	 && !out_of_memory
 710	 && (!nodes_limit ||
 711	   ( MoveOrdering && (nodes_limit - nodes_searched > (std::max(current_root->pn, node->pn) >>16) )) ||
 712	   (!MoveOrdering && (nodes_limit - nodes_searched > (std::max(current_root->pn, node->pn)))))
 716	 && !stop
```

子へ渡すしきい値は `select_the_best_child` が計算する。
OR 節点では pn 最小の子を選び、`pn2` を「他の子の pn の最小値と親の second_pn の小さい方」、`dn2` を「second_dn から他の子の dn（無限大を除く）の和を引いた値」とする。
これは df-pn の `thpn_child = min(thpn, pn_2 + 1)` と `thdn_child = thdn - dn(n) + dn(n_1)` に相当する。
標準形の `+1` は、703-705 行のループ条件を `<=` にすることで吸収されており（`pn > second_pn` で戻るのは `pn >= second_pn + 1` で戻るのと同じ）、しきい値の意味は変わらない。

```
R/YaneuraOu/source/mate/mate_dfpn.hpp:789-817
 792	for (u32 i = 1; i < child_num; ++i)
 793		if (children[i].pn < children[selected_index].pn)
 794			selected_index = i;
 798	for (u32 i = 0; i < child_num; ++i)
 799		if (i != selected_index)
 801			if (children[i].pn < pn2)
 802				pn2 = children[i].pn;
 812			if (children[i].dn < NodeType::DNPN_MATE)
 813				dn2 -= children[i].dn;
 817	dn2 = std::min(dn2, second_dn);
```

AND 節点は 821-835 行で対称である。
無限大は型の最大値 `DNPN_INF` で表し、`DNPN_INF - 2000` 以上を詰み域とし、N 手詰めを `DNPN_INF - N` に符号化する（66-67 行、140-156 行）。
集計では、無限大が絡まない限り単純な和をとり、飽和処理はない（884 行、892 行）。
MoveOrdering 版では、葉の初期値を駒割から `pn = -value + 30000`、`dn = value + 30000` と決める（110-111 行）。

### 1.2 TT に相当する構造

`Mate::Dfpn` の節点は、ハッシュ表ではなく線形に切り出す固定配列 `NodeManager` に置かれる。
GC はない（308 行「これは先頭からリニアに使っていく。GCは考えない。」）。

```
R/YaneuraOu/source/mate/mate_dfpn.hpp:211-228（DFPN32 版の Node、16 B）
 211	NodeCountType pn, dn;          // u32
 215	u32 lastMove  : 23;
 219	u32 repeated  : 1;
 226	u32 child_num :  8;
```

DFPN64 版は `Node* children`、u64 の pn と dn、`Move lastMove`、`u8 child_num`、`bool repeated` を持つ 32 B である（60-99 行）。
容量は MB 指定なら `size_mb*1024*1024/sizeof(Node)` 個（436-443 行）、節点数指定なら `alloc_by_nodes_limit`（447-453 行）で決まり、尽きると `out_of_memory` が立って `Move::none()`（不明）を返す（517-519 行）。

結論だけを保存する補助キャッシュ `MateHashEntry`（16 B）もある。
盤面キー下位 48 bit、手番、詰みか不詰か、手数 11 bit、着手、手番側の持ち駒 32 bit、ロック用の atomic からなる（`R/YaneuraOu/source/mate/mate.h:64-128`）。
置換方針は常時上書きであり、既存エントリの持ち駒が優越する場合だけ書き込まない（`R/YaneuraOu/source/mate/mate.cpp:250-282`）。
ロックは比較交換（CAS）命令によるスピンロックである（同 296-308）。
保存は、千日手が絡まずに証明が完了した節点に限る（mate_dfpn.hpp:753-761）。
ただしこのキャッシュを使うのは `source/testcmd/mate_test_cmd.cpp` だけであり、2 つの詰将棋エンジンはどちらも使わない。

### 1.3 節点展開

攻め方の手は王手に限る。
王手を受けていなければ `CHECKS_ALL`、受けていれば `EVASIONS_ALL` を生成して `gives_check` で絞り、受け方は `EVASIONS_ALL` を生成し、どちらも `legal` で絞る（`R/YaneuraOu/source/mate/mate_move_picker.h:49-70`）。
OR 節点で王手を受けていなければ、展開前に `Mate::mate_1ply` を呼び、1 手詰めがあれば子を 1 つだけ作る（mate_dfpn.hpp:1146-1167）。
子がない場合は、OR 節点なら不詰、AND 節点なら詰みとする（1191-1197 行）。
1 回の反復では 1 節点を展開するだけで親へ戻る（724-731 行）。

### 1.4 千日手と GHI

経路上の千日手は `Mate::mate_repetition` が `pos.is_repetition` で判定し、探索開始局面以降で該当しなければ 16 手まで遡る（mate.h:45、mate.cpp:139-161）。
OR 節点では DRAW、LOSE、INFERIOR を不詰、WIN と SUPERIOR を詰みとみなす（mate.cpp:143-151）。
木が合流しないため、GHI は木の値そのものには影響しない。
ただし結論をキャッシュへ保存するときに経路依存の値を混ぜないよう、`repeated` フラグを親へ伝播させる（900-947 行）。
TCA に相当する仕組みはない。

### 1.5 終了と結果の取り出し

公開 API は `mate_dfpn(pos, nodes_limit)`、`get_pv()`、`get_current_pv()`、`get_unproof_pv()`、`get_mate_ply()` である（mate_dfpn.hpp:481-547）。
返り値は、詰みなら最善手、不詰なら `Move::null()`、不明なら `Move::none()` である。
PV は証明木を `pick_the_best` で辿り、OR 節点で dn 最大（最短の詰み）、AND 節点で dn 最小（最長の抵抗）の子を選ぶ（558-652 行）。
これは探索済みの木の中での最短にすぎず、API のコメントも「最短手順である保証はない」と記す（mate.h:264）。
余詰の検査はない。

やねうら王詰将棋エンジンの `go mate` は、別スレッドでソルバを走らせ、主スレッドが PV 出力と時間監視を行う（`R/YaneuraOu/source/engine/yaneuraou-mate-engine/yaneuraou-mate-search.cpp:60-98`）。
出力は `checkmate timeout`、`checkmate none`（不明。USI 標準の `notimplemented` ではない）、`checkmate nomate`、および手順列である（同 100-125）。
`go mate N` の N はミリ秒の時間として解釈する（`R/YaneuraOu/source/usi.cpp:609-620`）。
コードを読む限り、時間切れでもソルバへ停止を伝えずに `thread.join()` で待つため、`checkmate timeout` はソルバが自然に終わった後に出る（実行では確認していない）。

### 1.6 マルチスレッド

探索は単一スレッドである（mate_dfpn.hpp:29-35、336-337）。
ふかうら王（dlshogi-engine）はソルバのインスタンスをスレッドごとに持つ。

### 1.7 通常探索との接続

通常探索 yaneuraou-search.cpp が呼ぶのは `Mate::mate_1ply` だけである。
search では `!rootNode && !ttHit && !excludedMove && !inCheck` のときに呼び、詰みなら `mate_in(ss->ply + 1)` を TT へ `BOUND_EXACT`、深さ `depth + 6` で書く（`R/YaneuraOu/source/engine/yaneuraou-engine/yaneuraou-search.cpp:2896-2924`）。
qsearch でも TT 非ヒット時に同じ処理を行う（4766-4782 行）。
`MateSearchPly` と `MateHashMb` はこのコミットに存在せず、`PvInterval`（82 行）は PV の出力間隔にすぎない。

df-pn を対局探索に組み込んでいるのは、ふかうら王である。
葉では `LeafDfpnNodesLimit`（既定 40 節点）で `mate_dfpn` を呼び、詰みなら勝ちを設定し、不詰なら以後の詰み探索を省く（`R/YaneuraOu/source/engine/dlshogi-engine/UctSearch.cpp:700-716`、`SearchOptions.cpp:177-185`）。
PV 上の局面は専用スレッド（`PV_Mate_Search_Threads`、既定 1 本、`PV_Mate_Search_Nodes` 既定 500,000 節点）が `Node48bitOrdering` 型のソルバで調べる（`PvMateSearch.cpp:22-30`）。
なお `MateDfpnSolver` は `dfpn_stop` を `impl` へ転送しておらず（mate.h:278、316-370）、`PvMateSearch.cpp:371` の停止要求は探索を止めないと読める（実行では確認していない）。

## 2. tanuki- 詰将棋エンジン（source/engine/tanuki-mate-engine/tanuki-mate-search.cpp）

tanuki- 詰将棋エンジンは、岸本の TCA つき df-pn を擬似コードに沿って実装している（`R/YaneuraOu/source/engine/tanuki-mate-engine/tanuki-mate-search.cpp:11-13`）。
無限大は `kInfinitePnDn = 100000000`（65 行）で、和は `kInfinitePnDn - 1` で飽和させる（633 行、652 行）。
しきい値更新は教科書どおりである。

```
R/YaneuraOu/source/engine/tanuki-mate-engine/tanuki-mate-search.cpp:711-712（OR）、734-735（AND）
 711	thpn_child = std::min(thpn, second_best_pn + 1);
 712	thdn_child = std::min(thdn - entry.dn + best_dn, kInfinitePnDn);
 734	thpn_child = std::min(thpn - entry.pn + best_pn, kInfinitePnDn);
 735	thdn_child = std::min(thdn, second_best_dn + 1);
```

TT は Stockfish 式で、20 B のエントリを 3 個束ねた 64 B のクラスタからなる。

```
R/YaneuraOu/source/engine/tanuki-mate-engine/tanuki-mate-search.cpp:128-163
 128	uint32_t hash_high;
 131	uint32_t pn; // 初期値 : 1
 132	uint32_t dn; // 初期値 : 1
 135	uint32_t num_searched;
 139	uint16_t minimum_distance; // 初期値 : kInfiniteDepth
 142	uint16_t generation;
 157	static_assert(sizeof(TTEntry) == 20, "");
 162	static constexpr int kNumEntries = 3;
```

キーは持ち駒を含む局面キーで（224 行）、`hash_high` の最下位ビットに root_color を埋める（182 行）。
置換は、世代が古いエントリを優先し、なければ `num_searched` が最少のエントリを潰す（199-207 行）。
証明駒と反証駒は持たず、優越関係の利用は TODO である（15-16 行、189-192 行）。
GC はなく（112 行「SmallTreeGCは実装せず」）、世代は `go mate` ごとに進む（264-266 行）。

千日手は `n.is_repetition` で判定し、DRAW を `pn = ∞, dn = 0` とする（555-559 行）。
TCA の `inc_flag` は、子の `minimum_distance` が親より小さく未解決の場合（古い子の参照）に立てる（499-509 行、601-611 行）。
葉では OR 節点で `mate_1ply` を呼ぶ（488-493 行）。
PV は `MorePreciseMatePv`（既定 true）なら OR 節点で最短、AND 節点で最長の手を選び直す再探索で作る（867-959 行）。
出力は `checkmate timeout`、`checkmate nomate`、手順列の 3 種である（403-419 行）。
時間による打ち切りは `#if 0` で無効であり（450-464 行）、`Worker::nodes` を増やす箇所がないため節点数制限も機能しないと読める（実行では確認していない）。
参考文献として長井の df-pn 論文と博士論文、Weak PN、岸本の AAAI-10、Kishimoto らの 2012 年サーベイを挙げる（19-55 行）。

## 3. KomoringHeights（source/engine/user-engine/）

KomoringHeights は、やねうら王の USER_ENGINE 枝として作られた詰将棋専用エンジンであり、README.md は GPL v3 と記す（LICENSE ファイルは複製に含まれない）。
minase は MIT なので、本書は設計の参考にとどめ、コードは転記しない。

### 3.1 アルゴリズムの変種

アルゴリズムは **df-pn+**（葉の初期値を発見的に与える df-pn）であり、pn と dn の単位を 2 として小数相当の値を扱う。

```
R/KomoringHeights/source/engine/user-engine/typedefs.hpp:194-198
 194	using PnDn = std::uint64_t;
 196	inline constexpr PnDn kInfinitePnDn = std::numeric_limits<PnDn>::max() / 2 - 1;
 198	inline constexpr PnDn kPnDnUnit = 2;
```

子へのしきい値は φ と δ の形で計算する。
φ は手番側から見た「自分の勝ちの証明数」、δ はその逆であり、OR 節点では φ = pn、δ = dn、AND 節点ではその逆になる。

```
R/KomoringHeights/source/engine/user-engine/local_expansion.hpp:379-388
 379	const auto thphi = Phi(thpn, thdn, or_node_);
 380	const auto thdelta = Delta(thpn, thdn, or_node_);
 381	const auto child_thphi = std::min(thphi, GetSecondPhi() + 1);
 382	const auto child_thdelta = NewThdeltaForBestMove(thdelta);
R/KomoringHeights/source/engine/user-engine/local_expansion.hpp:511-526
 512	PnDn delta_except_best = sum_delta_except_best_;
 513	if (mp_.size() > idx_.size()) {
 514	  delta_except_best += std::max<std::size_t>((mp_.size() - idx_.size()) / 8, 1);
 517	if (sum_mask_[idx_[excluded_moves_]]) {
 518	  delta_except_best = SaturatedAdd(delta_except_best, max_delta_except_best_);
 522	if (thdelta >= delta_except_best) {
 523	  return ClampPnDn(thdelta - delta_except_best);
 526	return 0;
```

δ の集計は和と最大値の混合である。
`sum_mask_` のビットが立つ子は和で、立たない子は最大値で計上し、子の δ が `kForceSumPnDn = kInfinitePnDn / 1024` 以上になるとその子は最大値側へ移る（26 行、177 行、301 行）。
後回しにした子（遅延展開中の合駒や不成）があれば、その数の 1/8（下限 1）を罰則として加える（485-488 行）。
飽和処理は `ClampPnDn`（typedefs.hpp:206-208）と符号なしの `SaturatedAdd`（同 79-86）が担う。

根では反復深化を行う。
初期しきい値はスレッド番号に等しく、以後は前回の pn と dn に (1.7 + 0.3 × スレッド番号) を掛けて 1 を足した値まで広げ、無限大に達したら「overflow detected」を出して打ち切る。

```
R/KomoringHeights/source/engine/user-engine/komoring_heights.cpp:16-19
  16	const auto thpn = static_cast<PnDn>(static_cast<double>(pn) * (1.7 + 0.3 * tl_thread_id)) + 1;
  17	const auto thdn = static_cast<PnDn>(static_cast<double>(dn) * (1.7 + 0.3 * tl_thread_id)) + 1;
  19	return std::make_pair(ClampPnDn(curr_thpn, thpn, kInfinitePnDn), ClampPnDn(curr_thdn, thdn, kInfinitePnDn));
```

### 3.2 TT

TT は通常テーブルと千日手テーブルに分かれ、`USI_Hash`（MB）の 75% を前者、25% を後者に割り当てる（transposition_table.hpp:17、69-75）。
通常エントリは 64 B で、各フィールドは次のとおりである。

```
R/KomoringHeights/source/engine/user-engine/ttentry.hpp:580-597
 580	std::atomic<Hand> hand_{kNullHand};   // 4 B。空きエントリの印にも使う
 581	SearchAmount amount_;                 // uint32。探索量
 582	Key board_key_;                       // uint64。盤面ハッシュ
 584	MateLen proven_len_;                  // 4 B。詰み手数
 585	MateLen disproven_len_;               // 4 B。不詰手数
 587	PnDn pn_;                             // uint64
 588	PnDn dn_;                             // uint64
 590	mutable SharedExclusiveLock<std::int8_t> lock_;   // 1 B
 591	RepetitionState repetition_state_;                // uint8
 593	mutable std::atomic<std::int16_t> min_depth_;     // 最小訪問深さ
 595	Hand parent_hand_;                    // 二重カウント対策用の親
 596	Key parent_board_key_;
 597	BitSet64 sum_mask_{};                 // δ を和で計算する子の集合
```

世代フィールドはない。
配置は開番地法で、盤面キー下位 32 bit から開始位置を決め、一致か空きに当たるまで線形に走査する（regular_table.hpp:186-189、ttquery.hpp:264-275）。
書き込み時の追い出しはなく、エントリは GC でだけ消える（regular_table.hpp:130-131）。

証明駒と反証駒には専用フィールドがなく、詰みや不詰の結果を「`hand_` を証明駒または反証駒に一致させたエントリ」へ書く（ttquery.hpp:285-293）。
読み出しでは、完全一致、劣等局面（`hand_is_equal_or_superior(entry_hand, hand)`）、優等局面の順に参照する（ttentry.hpp:383-397）。
詰みと不詰は「`proven_len_` 手以下で詰み」「`disproven_len_` 手以下では詰まない」という区間で保持し、確定エントリには探索量へ `kFinalAmountBonus{1000}` を加えて消えにくくする（ttentry.hpp:19、107-118、333、346）。

GC は使用率 500 ‰ 以上で発動する。
点検は GC 担当スレッド（最後のスレッド）が「容量 × 0.5」節点ごと、さらに 4096 回に 1 回だけ行う（search_monitor.hpp:19-30、komoring_heights.cpp:393-395、thread_initialization.hpp:21）。
削除のしきい値は、空でないエントリを間隔 334 で 20,000 個標本化し、探索量の中央値（`kGcRemovalRatio = 0.5`）を `nth_element` で求めて決める。

```
R/KomoringHeights/source/engine/user-engine/regular_table.hpp:248-262
 248	const std::int64_t gc_removal_pivot =
 249	    std::max<std::int64_t>(static_cast<std::int64_t>(detail::kGcSamplingEntries * gc_removal_ratio), 1);
 251	std::nth_element(amounts.begin(), pivot_itr, amounts.end());
 253	const bool should_cut = (max_amount > std::numeric_limits<SearchAmount>::max() / 8);
 259	  if (!entry.IsNull() && entry.Amount() <= amount_threshold) {
 260	    entry.SetNull();
 261	  } else if (!entry.IsNull() && should_cut) {
 262	    entry.CutAmount();
```

探索量がしきい値以下のエントリをすべて消し、最大探索量が uint32 上限の 1/8 を超えていれば残りの探索量を半減し、最後に線形走査の連続性を保つためコンパクションを行う（267 行）。
各局面の探索量は、未解決なら「最善子の探索量 + 子の数 − 1」として子から集計する（local_expansion.hpp:647）。

千日手テーブルは、経路ハッシュ 8 B、深さ、不詰手数、世代 uint16 の 16 B エントリからなり、世代ごとの GC を独自に持つ（repetition_table.hpp:36-42、166-172、201-237）。
探索開始時に全消去する（transposition_table.hpp:91）。

### 3.3 節点展開

攻め方は `CHECKS_ALL`、王手を受けているときは `EVASIONS_ALL` を `gives_check` で絞り、受け方は `EVASIONS_ALL` を使い、最後に `legal` で絞る（move_picker.hpp:47-61）。
子の並べ方は、φ、δ、詰み手数、千日手開始深さ、探索量の順で決まらなければ `MoveBriefEvaluation`（歩、角、飛の不成に 1000 点、駒価値を減点、玉からの距離 × 10）に従う（search_result.hpp:258-300、initial_estimation.hpp:194-216）。

葉の初期値は df-pn+ の表で、OR 局面は到達升への双方の利き数と取る駒の種類、AND 局面は取る手か玉を動かす手かと利きの多寡で決める（initial_estimation.hpp:21-38、87-151）。
`DelayedMoveList` は、同じ升への合駒と成不成の組を連結し、先の手の結果が確定するまで後の手を展開しない（delayed_move_list.hpp:145-173、206-262）。
AND 局面で初めて訪れる子には、子局面を進めて 0 手不詰と 1 手詰めを判定する（local_expansion.hpp:39-43、196-200）。
1 局面の手数上限は `kMaxCheckMovesPerNode = 110`（typedefs.hpp:171）であり、`sum_mask_` は `BitSet64` なので 65 番目以降の子は常に最大値で計上される。

### 3.4 千日手、GHI、TCA、二重カウント

現在の経路上の局面、またはその劣等局面に戻る子は、展開時に「千日手」（pn = ∞、dn = 0、開始深さつき）とする。
経路の照合は Node が持つ `VisitHistory`（(board_key, hand) から深さへの開番地表、32768 エントリ）で行う。

```
R/KomoringHeights/source/engine/user-engine/local_expansion.hpp:160-161
 160	if (const auto depth_opt = n.IsRepetitionOrInferiorAfter(move.move)) {
 161	  result = SearchResult::MakeRepetition(hand_after, len, 1, *depth_opt);
R/KomoringHeights/source/engine/user-engine/visit_history.hpp:109
 109	if (entry.board_key == board_key && hand_is_equal_or_superior(entry.hand, hand)) {
```

親へ返すとき、千日手の開始深さが現在より浅ければ千日手として伝え、そうでなければ解消したとみなして通常の不詰に戻す（local_expansion.hpp:577-583、611-620）。

GHI には、千日手の結果を TT エントリに `kPossibleRepetition` の印を付けて pn と dn を 1 に戻し、経路ハッシュをキーに千日手テーブルへ登録する方式で対処する（ttquery.hpp:137-143、302-308）。
経路ハッシュは深さごとに別の乱数表を引く Zobrist 型であり、同じ手順を前後させた経路とは衝突しない（path_keys.hpp:179-187、233-248）。
refs.md はこれを独自方式と記す。

TCA は、元論文の真偽値の旗を非負整数 `inc_flag` へ広げたものである。
TT が浅い探索の結果を返したとき（`min_depth < depth16`、ttentry.hpp:503-505）に増やし、新しい子を初めて展開したときに減らし、しきい値は pn + 1、dn + 1 まで広げる。

```
R/KomoringHeights/source/engine/user-engine/komoring_heights.cpp:383-390
 383	// Threshold Controlling Algorithm(TCA).
 385	if (local_expansion.DoesHaveOldChild()) {
 386	  inc_flag++;
 389	if (inc_flag > 0) {
 390	  ExtendSearchThreshold(curr_result, thpn, thdn);
```

二重カウント対策は Kishimoto (2010) の方式である。
未解決の結果を書くときに親の (board_key, hand) を記録し、最善子から親を最大で深さ回まで遡って現在の経路上の局面に合流すれば二重カウントとみなす。
途中で pn または dn が `kAncestorSearchThreshold`（3 単位）を超えて増えた場合は候補から外す（double_count_elimination.hpp:112-137）。
合流が見つかれば、分岐元で先頭の子と合流経路側の子の `sum_mask` ビットを落として δ を最大値計上へ切り替え、`sum_mask` は TT に保存して次回に引き継ぐ（local_expansion.hpp:396-411、ttquery.hpp:146）。

### 3.5 終了と結果の取り出し

最初の詰みを見つけた後、手数上限 `len` を 2 ずつ減らして再探索し（余詰探索）、見つからなくなったら `len + 2` に戻して `GetMatePath` で手順を作り直す（komoring_heights.cpp:203-243）。
余詰探索の深さは `PostSearchLevel`（None、UpperBound、MinLength）で選ぶ（engine_option.hpp:168-172）。
手順の復元では 1 手詰めを確かめ、各子の `FinalRange`（最長不詰手数と最短詰み手数）から最善手を選び、足りなければしきい値を無限大にして再探索する（komoring_heights.cpp:456-546）。

出力は `checkmate timeout`、`checkmate nomate`、手順列の 3 種で（user-search.cpp:42-48）、`notimplemented` は出さない。
`go mate N` の N はミリ秒であり（`R/KomoringHeights/source/usi.cpp:660-669`）、節点数は `NodesLimit`（0 は無制限）で制限する。
不詰の場合は受け方の応手を 1 手 PV に付ける（docs/ReleaseNote.md:11-12）。

### 3.6 マルチスレッド

全スレッドが同じ根から探索し、仕事の分担はない。
多様性は、根のしきい値の倍率、TT 読み出し時の周期的な +1、df-pn+ の定数と駒価値への乱数で与える（komoring_heights.cpp:16-17、ttquery.hpp:31-39、initial_estimation.hpp:48-77）。
ロックは、通常テーブルがエントリごとの 1 B の共有排他スピンロック、千日手テーブルが表全体で 1 つの `SharedExclusiveLock<std::int32_t>` である（ttentry.hpp:590、repetition_table.hpp:239）。
GC は他スレッドの探索と並行してエントリ単位のロックで動く（regular_table.hpp:258）。

### 3.7 通常探索との接続

接続はない。
`go` でも同じソルバを実行して `bestmove` を返すだけである（user-search.cpp:118-128）。
独自オプションは `NodesLimit`、`PvInterval`、`RootIsAndNodeIfChecked`、`ScoreCalculation`、`PostSearchLevel` の 5 つである。

参考文献（docs/refs.md:26-34）は、脊尾 (1999) の優越関係、長井と今井 (2002) の df-pn、Nagai (2002) の df-pn+、Kishimoto と Müller (2004) の GHI、金子ら (2010) の新規節点での固定深さ探索、Kishimoto (2010) の TCA と二重カウント、Song ら (2016) の deep df-pn である。

## 4. shtsume（source/）

shtsume は C 言語の詰将棋専用 USI エンジンである（版は `R/shtsume/source/shtsume.h:23` の "v1.2.7e"）。
source/ の base.c 系と nbase.c 系は旧版と新版ではなく層構造で、Makefile はディレクトリ内の全 .c をリンクする（`R/shtsume/Makefile:22`）。

### 4.1 アルゴリズムの変種

反復深化のない単一呼び出しの df-pn であり、集計式に特徴がある。
AND 節点の pn と OR 節点の dn に総和ではなく「最大値 + 未解決の兄弟の数」を使い、これは Weak Proof Number と同じ形である。

```
R/shtsume/source/shtsume.c:117-131（proof_number。disproof_number は 133-151 で対称）
 117	uint16_t wpn = list->tdata.pn, pn = 0;
 124	while(list){
 125	    if(!list->tdata.pn) break;
 126	    pn++;
 127	    wpn = MAX(wpn, list->tdata.pn);
 131	return MIN(INFINATE-1, wpn+pn);
```

子へのしきい値は、OR 節点では次のとおりである。
未探索の最良の子には pn しきい値 1 を与えて 1 段だけ展開させる。

```
R/shtsume/source/shtsume.c:472-486
 472	if(list->next) {
 473	    c_threshold.pn =
 474	    MIN(th_tdata->pn,
 475	        MIN(INFINATE, list->search?(list->next)->tdata.pn+1:1));
 476	    if(th_tdata->dn == INFINATE-1)
 477	        c_threshold.dn = INFINATE-1;
 478	    else
 479	        c_threshold.dn =
 480	        S_COUNT(sdata)? th_tdata->dn - dcnt:th_tdata->dn;
 486	c_threshold.sh = th_tdata->sh-1;
R/shtsume/source/shtsume.c:779-782（AND）
 780	c_threshold.dn = MIN(th_tdata->dn, (list->next)->tdata.dn+1);
 781	c_threshold.pn = th_tdata->pn - pcnt;
```

しきい値には手数 `sh` の上限も含まれる。
無限大は uint16 の欄に `INFINATE = SHRT_MAX`（32767）を入れて表し、飽和は `MIN(INFINATE-1, …)` だけである（shtsume.h:61-62、112-114）。
根の pn しきい値は `PROOF_MAX - 1 = 255` に固定されている（shtsume.c:188）。

### 4.2 TT

TT は、盤面キーで引く `zfolder`、攻め方の持ち駒で引く `mcard`、深さ別の `tlist` という 3 段の連結リストである。

```
R/shtsume/source/shtsume.h:213-247
 215	tlist_t             *next;
 216	tdata_t             tdata;            // pn, dn, sh の各 uint16
 217	short                  dp;            // 初期局面からの深さ
 227	tlist_t            *tlist;            /* 深さ別情報格納    */
 228	mkey_t               mkey;            /* 詰方の持ち駒     */
 229	unsigned int         hinc: 1;         // 詰め上がりで駒余り
 230	unsigned int        nouse: 6;         // 詰時余り駒数
 231	unsigned int      current: 1;         // pvで使用中
 232	unsigned int      protect: 1;         // 保護フラグ
 233	unsigned int          cpn:16;         // current時の証明数しきい値
 244	zkey_t                zkey;
 246	mcard_t             *mcard;
```

持ち駒 `mkey_t` は玉 11 bit、飛 2、角 2、金 3、銀 3、桂 3、香 3、歩 5 bit のビット欄で、`MKEY_COMPARE` が完全一致、優越、劣等、無関係を判定する（`R/shtsume/source/shogi.h:464-484`）。
盤面ハッシュは持ち駒を含まない 64 bit の Zobrist キーである（base.c:104、169）。
世代、置換方針、バケットはなく、索引は `zkey & mask`（shtsume.h:355）で引く。
詰みと不詰は `dp = ALL_DEPTH` で記録して mkey に証明駒または反証駒を入れ、登録時に自分が優越または劣等する既存値を削除する（ntbase.c:1445-1449、1496-1507、1615-1618、1655-1666）。
未解決の値だけは手数 `dp` ごとに別の tlist として持ち、根しきい値付近の pn は保存しない（ntbase.c:1218-1219）。

GC は要素プールが尽きたときに起こる（ntbase.c:404-407、1222）。
全表を走査し、`protect` も `current` も立っていない mcard の tlist を、詰みなら玉方手番の深さ、不詰なら偶数深さまたは `gc_level` 未満、未解決なら pn または dn が `gc_level` 未満か玉方手番、という条件で消す（ntbase.c:203-244）。
削除数が要素数の 20% を超えるまで `gc_level` を 1 ずつ上げて繰り返す（258-265 行、367-375 行）。
容量は `USI_Hash`（MB、既定 256）から `g_usi_hash * 16384 - 1` 要素に換算し（nusi.c:134、shtsume.h:350）、3 つのプールをそれぞれ確保する。

### 4.3 節点展開

攻め方の王手は `generate_check`（ncheck.c:60）が生成し、打ち歩詰めは `fu_tsume_check` で除く（ncheck.c:94-99）。
攻め方が王手を受けていれば `evasion_check`（ncheck.c:951-983）が逆王手だけを返す。
受け方の手は `generate_evasion`（nevasion.c:86-262）が玉の移動、王手駒を取る手、合駒の順に作り、合駒は 1 つの直列リストにまとめて先頭が詰んだ時点で次を展開する（shtsume.c:574-581、843-853）。
無駄合いは TT と詰み判定を使って除く（ntbase.c:739 以降）。
葉の初期値は (1, 1) で、OR 節点の未探索の子には玉からの距離を pn の下限として与える（nbase.c:15、shtsume.c:373-378）。
mate1ply は呼ばず、詰みは `generate_evasion` が NULL を返したときに確定する（shtsume.c:558-563）。

### 4.4 千日手と GHI

現在の経路上の局面には `tbase_set_current` が印を付け、祖先が設定した pn しきい値を `cpn` に保存する。
同じ盤面と持ち駒に再到達すると、pn = ∞ ではなく、その `cpn` を pn として返す。

```
R/shtsume/source/ntbase.c:632-634
 632	mcard->current = 1;
 633	mcard->cpn = mvlist->tdata.pn;
R/shtsume/source/ntbase.c:1157-1162
 1157	if(mcard->current){
 1158	    mvlist->tdata.pn = MAX(1,mcard->cpn);
 1159	    mvlist->tdata.dn = 1;
```

AND 節点では、この pn の膨張を攻め方の不詰に変換する（shtsume.c:664-674「単純ループ局面は不詰とする」、`pn >= PRE_PROOF_MAX-1 && dn == 1` のとき `dn = 0, pn = INFINATE`）。
この結果は経路に依存するが、通常の不詰として TT に登録される（shtsume.c:769）。
GHI の緩和としては、pn が `PRE_PROOF_MAX`（240）を超えた子を先頭の子の手リストへ連結する処理と、先頭の子の dn が `PRE_DISPROOF_MAX`（1024）を超えたら 2 番目の子を先頭に縮退させる処理がある（shtsume.c:677-730）。
経路ハッシュや visit history の専用構造はない。

### 4.5 終了と結果の取り出し

詰みが出ると、証明木を固定する `make_tree` を `n_make_tree` 回、余詰探索 `bns_plus` を `search_level` 回行う（shtsume.c:196-230）。
`bns_plus` は根の pn しきい値を 1 ずつ上げて詰み手以外を再探索し、駒余りの詰みなら打ち切る。
GC で失われた詰みの値は、make_tree、bns_plus、PV 作成の各段で再探索して復元する（shtsume.c:911-949、nsearchinf.c:267-274）。
PV は比較関数で決まり、攻め方は即詰め、駒余り、短い手数を優先し、玉方は長い手数を優先する（nbase.c:337-347、405-409）。

出力は `checkmate <手順>` と `checkmate nomate` の 2 種である（nusi.c:343-392）。
時間切れ、`stop`、根の pn 上限 255 への到達は、いずれも pn ≠ 0 かつ dn ≠ 0 のまま終わって「Perpetual check!」を info 出力し、`checkmate nomate` になる（nusi.c:385-392）。
時間は CPU 時間 `clock()` で測る（nsearchinf.c:47-52）。
節点数の制限はなく、手数上限は `TSUME_MAX_DEPTH = 2000` である。
探索は単一スレッドで、USI 受信スレッドが素の `bool g_stop_received` を書き換える（usi.c:35、420-428）。
通常探索との接続はなく、`go` には `bestmove resign` を返す（nusi.c:413-426）。

## 5. tsumeshogi-solver（Rust）

tsumeshogi-solver は、長井と今井の論文「df-pn アルゴリズムの詰将棋を解くプログラムへの応用」の擬似コードを Rust のトレイトとして写した小さな実装である（`R/tsumeshogi-solver/dfpn/src/search.rs:4-5`）。

### 5.1 API の設計

探索は 2 つのトレイトに依存する。

```
R/tsumeshogi-solver/dfpn/src/traits.rs:5-19
   5	pub trait Position {
   6	    type M: Copy + Into<Move>;
   8	    fn hash_key(&self) -> u64;
   9	    fn generate_legal_moves(&mut self, node: Node) -> Vec<(Self::M, u64)>;
  10	    fn do_move(&mut self, m: Self::M);
  11	    fn undo_move(&mut self, m: Self::M);
  14	pub trait Table: Default {
  15	    type U: Unsigned + PrimInt + SaturatingAdd;
  17	    fn look_up_hash(&self, key: &u64) -> (Self::U, Self::U);
  18	    fn put_in_hash(&mut self, key: u64, value: (Self::U, Self::U));
```

`generate_legal_moves` は、各手とその手を指した後のハッシュ値の組を返す。
探索本体 `Search<P, T>` はこの 2 つを合成したトレイトで、`mid` と `dfpn_search` を既定実装として持つ（search.rs:6-133）。
`solver` クレートはこれを `SearchOrCancel`（`cancel()` を反復ごとに確かめる版、`solver/src/dfpn_extended/search_or_cancel.rs:5-83`）で拡張し、`CancelableSearcher` が時間制限を実装する（cancelable_searcher.rs:69-82）。
局面は yasai クレートの `Position` を包み、OR 節点では `legal_moves()` を `is_check_move` で絞り、AND 節点では全合法手を返す（`solver/src/implementations/yasai_position.rs:20-30`）。

### 5.2 アルゴリズムとしきい値

無限大は型の最大値、根のしきい値は最大値 − 1 である。

```
R/tsumeshogi-solver/dfpn/src/search.rs:21-32
  23	let (pn, dn) = self.mid(hash, T::U::max_value() - T::U::one(), T::U::max_value() - T::U::one(), Node::Or);
  29	if pn != T::U::max_value() && dn != T::U::max_value() {
  30	    self.mid(hash, T::U::max_value(), T::U::max_value(), Node::Or);
R/tsumeshogi-solver/dfpn/src/search.rs:53-54, 64-83
  53	// 3. ハッシュによるサイクル回避
  54	self.put_in_hash(hash, (delta, phi));
  64	if phi <= md || delta <= sp {
  72	let phi_n_c = if phi_c == T::U::max_value() - T::U::one() {
  73	    T::U::max_value()
  74	} else if delta >= T::U::max_value() - T::U::one() {
  75	    T::U::max_value() - T::U::one()
  76	} else {
  77	    delta + phi_c - sp
  78	};
  79	let delta_n_c = if delta_c == T::U::max_value() - T::U::one() {
  80	    T::U::max_value()
  81	} else {
  82	    phi.min(delta_2.saturating_add(&T::U::one()))
  83	};
```

子の φ しきい値は `thδ − Σφ + φ_c`、δ しきい値は `min(thφ, δ_2 + 1)` であり、論文どおりの素の df-pn である。
φ の和は `saturating_add` で飽和させ（129 行）、和が最大値 − 1 以上なら δ の最小値を 0 とする（59-61 行）。
展開前に「自分のしきい値を (δ, φ) として TT に書く」54 行の処理が、論文の「ハッシュによるサイクル回避」であり、経路上の局面に戻った子は親のしきい値以上の値を返して選ばれなくなる。
千日手の専用処理はなく、テストに GHI の問題を含めて確認している（`solver/src/lib.rs:67-86`）。

### 5.3 TT

TT は (pn, dn) だけを持ち、証明駒、世代、GC はない（search.rs:15 のコメント「本当は優越関係が使える」）。
`HashMapTable` は上限のない `HashMap<u64, (U, U)>` で未登録なら (1, 1) を返し（hashmap_table.rs:24-33）、`VecTable` は 2^bits 個の直接写像でキーを検証せず常に上書きする（vec_table.rs:36-41）。
したがって `VecTable` はハッシュ衝突をそのまま受け入れる。

### 5.4 結果の取り出し

PV は、証明後に TT を使って全詰み手順を深さ優先で列挙し、評点の最大の手順を選ぶ。

```
R/tsumeshogi-solver/solver/src/solve.rs:48-59
  48	let (node, mate_pd) = if moves.len() & 1 == 0 { (Node::Or, (max, 0)) } else { (Node::And, (0, max)) };
  53	let mate_moves = searcher.generate_legal_moves(node).into_iter()
  56	    .filter(|(_, h)| !hashes.contains(h) && searcher.look_up_hash(h) == mate_pd)
  58	if node == Node::And && mate_moves.is_empty() {
  59	    solutions.push(searcher.pos.calculate_result_and_score(moves));
```

評点は `len * 100 − 攻め方の残り持ち駒数` で（yasai_position.rs:101）、末尾の「合駒、同」の組を削り、玉方が打った合駒が最後に攻め方の持ち駒に入る手順を 0 点にする（同 45-99）。
長い手順ほど高得点なので、選ばれるのは最長抵抗に対する手順である。
出力は CLI だけで USI はなく、`-t` で秒単位の時間制限を与える（`src/main.rs:88-90`）。
マルチスレッドと通常探索との接続はない。

## 6. OSL（GPS 将棋）の df-pn の移植（oslmate と cshogi/src/osl_dfpn.cpp）

oslmate は、GPS 将棋の OpenShogiLib（OSL）にある df-pn を Windows 向け USI 詰みエンジンとして単体で切り出したものであり、cshogi の `osl_dfpn.cpp`（約 10,300 行）はそれを Apery 系の盤面実装へ移植したものである。
Yamaoka 氏のブログによれば、後者は PV と節点数が oslmate と一致するまで約 1 か月半かけて合わせ込み、やねうら王の 3 手から 11 手詰め各約 100 万局面を全問正解した（2026 年 6 月 16 日の記事）。
cshogi 版は OSL の乱数表を `osl_hash.txt` から読み込んでハッシュ値まで一致させている（`R/cshogi/src/osl_dfpn.cpp:183-194`）。
oslmate のビルドは並列化マクロ `OSL_SMP` を定義しないため、以下では並列化のコードを無効として扱う。

### 6.1 アルゴリズムの変種

基本は df-pn であり、そこに df-pn+ 風の手のコスト（`proof_cost`）と、子へのしきい値に加える frontier 平均値（`proof_average`、`disproof_average`）を加算する。
乗算形の 1+ε trick ではない。
pn と dn は 1 つの 64 bit 値 `pdp` に詰め、無限大は特殊値で表す。

```
R/oslmate/osl/osl/core/osl/checkmate/proofDisproof.h:23-41
  23	PROOF_MAX = (0xffffffffu / 16),
  24	DISPROOF_MAX = (0xffffffffu / 16),
  26	NO_ESCAPE_DISPROOF  = (DISPROOF_MAX - 1),
  27	CHECK_MATE_DISPROOF = (DISPROOF_MAX - 2),
  29	NO_CHECK_MATE_PROOF   = (PROOF_MAX - 1),
  30	PAWN_CHECK_MATE_PROOF = (PROOF_MAX - 2),
  31	LOOP_DETECTION_PROOF  = (PROOF_MAX - 3),
  32	ATTACK_BACK_PROOF  = (PROOF_MAX - 4),
```

根のしきい値は `ROOT_PROOF_TOL = ROOT_DISPROOF_TOL = 65536ul*1024` である（`R/oslmate/osl/osl/std/osl/checkmate/dfpn.cc:46-48`）。
OR 節点 `attack()` の打ち切りと、子へのしきい値は次のとおりである。

```
R/oslmate/osl/osl/std/osl/checkmate/dfpn.cc:2012-2014, 2022-2025, 2059-2068
 2012	#ifdef KISHIMOTO_WIDEN_THRESHOLD
 2013	if (loop == 0 && sum_disproof >= node.threshold.disproof() && sum_disproof > IgnoreUpwardDisproofThreshold)
 2014	  node.threshold = ProofDisproof(node.threshold.proof(), sum_disproof+1);
 2022	if (min_proof >= node.threshold.proof()
 2023	    || sum_disproof >= node.threshold.disproof()
 2024	    || next_i >= node.children.size()
 2025	    || node_count + min_proof >= node_count_limit) {
 2059	unsigned int disproof_c = node.threshold.disproof()
 2060	  - (sum_disproof - node.children[next_i].disproof());
 2062	if (disproof_c > node.threshold.disproof())
 2063	  disproof_c = node.children[next_i].disproof()
 2064	    + (node.threshold.disproof() - sum_disproof);
 2066	next.threshold = ProofDisproof(std::min(min_proof2+proof_average, (size_t)node.threshold.proof())
 2067	                               - node.proof_cost[next_i],
 2068	                               disproof_c);
```

AND 節点 `defense()` は対称である。

```
R/oslmate/osl/osl/std/osl/checkmate/dfpn.cc:2606-2615
 2606	unsigned int proof_c = node.threshold.proof()
 2607	  - (sum_proof - node.children[next_i].proof());
 2613	next.threshold = ProofDisproof(proof_c,
 2614	                               std::min(min_disproof2+disproof_average,
 2615	                                        (size_t)node.threshold.disproof()));
```

2013-2014 行は、dn の和が既にしきい値を超えていても 1 回目のループではしきい値を広げる処理で、コメントが「Kishimoto」の名を記す。
dn の和には補正がある。
DAG で合流する子と、遠くから打つ香、飛、角の王手は種類ごとの最大値だけを和に加え（1951-2004 行）、AND 節点では利きのない升への合駒を `SacrificeBlockCount` として和から外す（2485-2488 行）。
Kakinoki の false branch に当たる場合は和を最大値に置き換え（2513-2534 行）、pdp が過大な上向きの子は `IGNORE_MONSTER_CHILD` によって最後まで選ばない（2460-2467 行）。
cshogi 版は同じ式を関数に切り出し、和を uint64 で計算して `saturate_sum` で飽和させる点だけが異なる（`R/cshogi/src/osl_dfpn.cpp:2731-2733`、2746-2770）。

### 6.2 TT と GC

TT は固定長の表ではなく、`std::unordered_map<BoardKey, std::forward_list<DfpnRecord>>` である（dfpn.cc:631、898、dfpn.h:89）。
`BoardKey` は 96 bit の盤面ハッシュで持ち駒を含まず、同じ盤面のレコードを持ち駒ごとにリストで並べる（`R/oslmate/osl/osl/core/osl/hashKey.h:28-32`、54-65）。
レコードの構造は次のとおりで、世代と深さの欄はない。

```
R/oslmate/osl/osl/std/osl/checkmate/dfpnRecord.h:17-39, 60
  17	ProofDisproof proof_disproof;
  19	uint64_t solved;                     // 解決済みの子の bitset
  22	uint64_t dag_moves;                  // 合流を引き起こす指手一覧
  24	Move best_move;
  25	PieceStand proof_pieces;
  26	mutable unsigned int node_count;
  27	unsigned int tried_oracle;
  29	Move last_move;
  31	PieceStand proof_pieces_candidate;
  32	unsigned int min_pdp;
  33	uint32_t working_threads;
  34	Square last_to;
  36	int8_t proof_pieces_set;
  37	char need_full_width, false_branch;
  39	bool dag_terminal;
  60	CArray<PieceStand,2> stands;
```

同じ持ち駒のレコードがあれば上書きし、なければ `push_front` で追加する（dfpn.cc:646-680）。
probe は完全一致を探す途中で、優越関係にある証明済みまたは反証済みのレコードがあればそれを採り、未解決なら `INITIAL_DOMINANCE` で初期値の下限を取る（800-835 行。上限は pn 35、dn 110）。
`solved` は 64 bit なので、65 手目以降の子で `1ull << i` が未定義動作になり、cshogi 版は x64 の挙動に合わせて `1ull << (index & 63)` と明記している（osl_dfpn.cpp:2784-2789）。

GC は `smallTreeGC` と `runGC` が行う。
削除の基準は「未解決で、かつ `node_count`（その節点以下の累積探索節点数）がしきい値未満」であり、証明済みと反証済みは消さない。

```
R/oslmate/osl/osl/std/osl/checkmate/dfpn.cc:764-769, 1233-1258
  764	if (! q->proof_disproof.isFinal()
  768	    && q->node_count < threshold) {
  769	  erase_after(p);
 1233	if (! (before >= growth_limit || (growth_limit - before) < growth_limit/8))
 1234	  return false;
 1237	const size_t removed = smallTreeGC(gc_threshold);
 1248	gc_threshold += 15;
 1249	static double memory_limit = 0.75;
 1250	if (memory > memory_limit) {
 1251	  growth_limit -= growth_limit/8;
 1252	  gc_threshold += 15 + gc_threshold/4;
 1255	if (removed < before*2/3)
 1256	  gc_threshold += 15 + gc_threshold/2;
 1257	if ((removed < before*3/5 && memory > 0.75) || removed < before/2)
 1258	  throw Dfpn::DepthLimitReached();
```

しきい値の初期値は 10 で（909 行）、GC のたびに 15 ずつ増え、消せた量が少なければさらに増える。
消せた量が半分未満なら例外 `DepthLimitReached` で探索全体を中断する。
発動は `attack()` の入口で、木の最大深さが `EnableGCDepth = 512` を超えたときに試す（88 行、1747-1765 行）。
経路表 `DfpnPathTable` にも、`visiting` でないレコードを `node_count` で消す別の GC がある（253-257 行、313 行）。
容量は `USI_Hash`（既定 64 MB）から `hash_bytes / (sizeof(HashKey) + sizeof(DfpnRecord) + 2 * sizeof(void*))` 個の growth_limit に換算し、表は `go mate` のたびに作り直す（`R/oslmate/oslmate/oslmate.cpp:392-395`、256）。
cshogi 版はレコードに `remaining_depth`（uint16）と `exact` を加え（osl_dfpn.cpp:1818-1838）、GC は同じ基準と係数を使う（2473-2526 行）。

### 6.3 節点展開

攻め方の手は、王手を受けていなければ相手玉の升を目標に `AddEffectWithEffect<...>::generate<P,true>` で王手を生成し、受けていれば玉の逃げ手のうち王手になるものだけを採る（dfpn.cc:1591-1608）。
受け方は、まず合駒の各升で最も安い駒だけを打つ `generateCheapKingEscape` で絞り（`R/oslmate/osl/osl/core/osl/move_generator/escape_.tcc:31-42`）、詰みが出たら `need_full_width` を立てて全幅で読み直す（dfpn.cc:2149-2174、2569-2576）。
葉の初期値は `LibertyEstimator::attackH` が王の逃げ道の数から pn を、1 または 2 を dn として与える（`R/oslmate/osl/osl/std/osl/checkmate/libertyEstimator.h:23-25`、69-99）。
受け方の各子では `FixedDepthSolverExt::hasEscapeByMove` で 1 手詰めを、根と条件つきの節点では `hasCheckmateMove<P>(2, ...)` で 3 手詰めを調べる（2278-2290 行、1717-1745 行）。

### 6.4 千日手、GHI、DAG

千日手は経路ハッシュ `PathEncoding` で検出する。
各手の符号は奇数なので、和の偶奇が手番を表す（`R/oslmate/osl/osl/std/osl/pathEncoding.h:28-35`）。
ループの判定は、経路表の `visiting`（現在の経路上か）と `twin_list`（ループと判定した経路の一覧）で行う（dfpn.cc:404-412）。

```
R/oslmate/osl/osl/std/osl/checkmate/dfpn.cc:1943-1947, 2452-2453
 1943	if (node.isLoop(i)) {
 1944	  node.children[i].proof_disproof = ProofDisproof::LoopDetection();
 1946	  proof = ProofDisproof::LOOP_DETECTION_PROOF;
 1947	  disproof = 0;
 2452	if (node.isLoop(i)) {
 2453	  node.setLoopDetection();
```

OR 節点ではループする子をその経路に限り攻め方の不詰とし、AND 節点では子にループがあれば親を (1, 1) に戻して経路を twin に登録する。
LoopDetection の結果は TT へ格納しない（1090 行の assert）。
持ち駒が劣る状態で同じ盤面へ戻る場合も `BadAttackLoop` として扱う（201-227 行）。
TCA はなく、`distance`（根からの最短距離）で上向きの子を判定し、`IGNORE_MONSTER_CHILD` と `KISHIMOTO_WIDEN_THRESHOLD` で無限ループを抑える。
DAG の二重カウントは、長井の方式 `NAGAI_DAG_TEST` で `findDagSource` が祖先を照合し、`dag_moves` にビットを立てて和から外す（dfpnRecord.h:9、dfpn.cc:1623-1661）。
cshogi 版は cshogi の `RepetitionType` を使わず（osl_dfpn.cpp:6223-6226）、`draw_ply` を深さの上限にするだけである。

### 6.5 終了と結果の取り出し

PV は `ProofTreeDepthDfpn::retrievePV` が TT の `best_move` を辿って再構成し、OR 節点では 1 手詰めと 3 手詰めを最優先し、AND 節点では最も長く逃れる受けを選ぶ（`R/oslmate/osl/osl/std/osl/checkmate/proofTreeDepthDfpn.cc:123-129`、154、198-208）。
最短手順は保証しない。
ドライバは節点数の上限を 256 から始め、4096 までは 4 倍、262144 までは 2 倍、その後は +262144 ずつ広げて再探索する（oslmate.cpp:398-405、267-311）。
停止と時間切れは段階の間でだけ確かめる。
出力は `checkmate <手順>`、`checkmate nomate`、`checkmate timeout` の 3 種で（oslmate.cpp:272、287、293）、LoopDetection は dn = 0 なので `nomate` に流れる。
オプションは `USI_Hash`、`InitialNodeLimit`（既定 256）、`DfpnMaxDepth`（既定 1600）である（80-83 行）。
`go` で `mate` 以外を受けると `bestmove resign` を返し（129-132 行）、通常探索との接続はない。
対称型マルチプロセッサ（SMP）向けの並列化コード（`working_threads`、`restartThreads`）は残っているが無効であり、探索は単一スレッドである。

### 6.6 cshogi の dlshogi 系 df-pn（src/dfpn.cpp）

cshogi にはもう 1 つ、dlshogi 由来の素の df-pn がある。
無限大は `kInfinitePnDn = 100000000`（`R/cshogi/src/dfpn.cpp:163`）で、しきい値は tanuki と同じ式である。

```
R/cshogi/src/dfpn.cpp:631-632, 751-752
 631	thpn_child = std::min(thpn, second_best_pn + 1);
 632	thdn_child = std::min(thdn - entry.dn + best_dn, kInfinitePnDn);
 751	thpn_child = std::min(thpn - entry.pn + best_pn, kInfinitePnDn);
 752	thdn_child = std::min(thdn, second_best_dn + 1);
```

TT は `unordered_map<Key, vector<unique_ptr<TTEntry>>>` で上限も GC もなく、`NewSearch()` で全消去する（159-161 行）。
エントリは `Hand hand`（常に先手の持ち駒）、`int pn`、`int dn`、`uint16_t depth`、`uint32_t num_searched` である（`R/cshogi/src/dfpn.h:9-13`）。
照合は持ち駒と `depth`（手数）の一致で行うため、同じ局面でも手数が違えば別エントリになる。
千日手は `n.isDraw(16)` で判定し、連続王手も通常の千日手も攻め方の負け（pn = ∞）として `num_searched = REPEAT` の印を付け、優越関係での再利用を禁じる（11 行、219 行、266 行、306 行）。
新しい節点では OR 節点で 3 手詰め、AND 節点で 2 手読みと `mateMoveIn1Ply` を試す（248-439 行）。
停止は `while (searchedNode < maxSearchNode && !stop)`（506 行）で、PV は OR 節点で証明済みの最初の手、AND 節点で最長の受けを選ぶ（784-858 行）。
Python からは `cshogi.DfPn` がこの実装、`cshogi.osl.DfPn` が OSL 移植である（`R/cshogi/cshogi/_cshogi.pyx:1730-1925`）。

## 7. HaChu と Fairy-Stockfish

どちらにも df-pn や証明数探索はない。中将棋を直接扱う詰みソルバとしては、本書の調査後に神無次郎氏の fmcs.exe を別に調べた（[fmcs の調査](../fm-fmcs-survey.md)）。
HaChu の `tsume` は対話的オプションであり（`hachu.c:165`、3010-3024）、通常の alpha-beta 探索の中で「詰まされる側が王手を受けていなければ勝ち（INF）を返す」フィルタとして働く。

```
hachu.c:2046-2064
 2046	// in-check test and TSUME filter
 2048	k = p[king=royal[stm]].pos;
 2050	  if((k = p[king + 2].pos) == ABSENT && (!tsume || tsume & stm+1))
 2051	    return -INF;   // lose when no King (in tsume only for side to be mated)
 2052	} else if(p[king + 2].pos != ABSENT) {
 2053	  if(tsume && tsume & stm+1) {
 2054	    retDep = 60; return INF; // we win when not in check
 2056	  k = ABSENT; // two kings is no king...
```

man page はこれを「checkmate problems where the winning side is only allowed to play checking moves」と説明する（hachu.pod:46-47）。
2048-2056 行は、王駒 2 枚（`royal[stm]` と `p[king+2]`）を扱う点で中将棋固有であり、「two kings is no king」として 2 枚あるときは王手判定を省く。
Fairy-Stockfish の `TsumeMode`（`src/ucioption.cpp:209`）も alpha-beta 側の裁定にすぎず、「王を持つ側が王手を受けていなければ `mate_in(ply)`」とする（`src/position.cpp:2949-2953`）。
`go mate N` は通常探索の mate distance 停止である（`src/search.cpp:510-512`）。

## 8. 比較表

| 項目 | やねうら王 `Mate::Dfpn` | tanuki- 詰将棋 | KomoringHeights | shtsume | tsumeshogi-solver | OSL（oslmate、cshogi.osl） | cshogi `dfpn.cpp` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 言語、ライセンス | C++、GPL v3 | C++、GPL v3 | C++、GPL v3（README の記載） | C、MIT | Rust、GPL v3 | C++、OSL 部分は BSD 系（oslmate/osl/LICENSE） | C++、GPL v3 |
| 変種 | 木を明示的に持つ pn 探索（合流なし）、MoveOrdering 版は駒割の初期値 | TCA つき df-pn | df-pn+、TCA、二重カウント除去、和と最大値の混合 δ | 単一呼び出し df-pn、集計は最大値 + 兄弟数（Weak PN 型） | 素の df-pn（長井と今井の擬似コード） | df-pn に手コストと frontier 平均を加算、Kishimoto のしきい値拡張、長井の DAG 対策、Kakinoki の false branch | 素の df-pn |
| 無限大と飽和 | 型の最大値、`INF − N` で手数、有限和は飽和なし | 10^8、和を `INF − 1` で飽和 | `u64::max/2 − 1`、`ClampPnDn` と `SaturatedAdd` | uint16 に 32767、`MIN(INF−1, …)` | 型の最大値、`saturating_add` | `0xffffffff/16`、特殊値で詰みの種類を区別、飽和なし（cshogi 版は `saturate_sum`） | 10^8、`std::min` のみ |
| TT 構造 | 固定配列を線形切り出し（16 B または 32 B）。別に 16 B の結論キャッシュ | 20 B × 3 の 64 B クラスタ、世代、`num_searched` 最少を置換 | 64 B エントリの開番地法、追い出しなし。16 B の千日手テーブルを別に持つ | zfolder、mcard、tlist の 3 段連結リスト、未解決は深さ別 | トレイト。HashMap または直接写像の Vec | `unordered_map<盤面キー, forward_list<Record>>`、持ち駒ごとにレコード | `unordered_map<Key, vector<Entry>>`、持ち駒と手数で照合 |
| 証明駒と反証駒 | 結論キャッシュに持ち駒 32 bit、優越判定 | なし（TODO） | `hand_` を証明駒に一致させて書く、読み出しで優劣参照 | mcard の mkey に格納、優劣で既存値を削除 | なし | `proof_pieces`、`DisproofPieces`、`stands`、優越 probe、proof oracle | `hand`、優越参照、REPEAT で再利用禁止 |
| GC | なし（満杯で不明を返す） | なし | 使用率 50% で発動、探索量の中央値以下を削除、確定値に +1000 のボーナス | プール枯渇で発動、`gc_level` を上げながら 20% 超まで削除、`protect` で証明木を保護 | なし | 深さ 512 超で `node_count < しきい値` の未解決レコードを削除、しきい値は 10 から +15 ずつ、半分未満なら例外で中断 | なし |
| 王手生成と受け方 | `CHECKS_ALL`、`EVASIONS_ALL`、`legal` | 同左 | 同左、合駒と不成の遅延展開 | 独自 `generate_check`、合駒の直列化、無駄合い除去 | yasai の `legal_moves` を `is_check_move` で絞る | 利きベースの王手生成、安価な合駒だけを先に生成し後で全幅 | `CheckAll`、`Evasion` |
| 葉の初期値 | (1,1) または駒割、OR で mate_1ply | (1,1)、OR で mate_1ply | df-pn+ 表、AND 側で 2 手詰め判定 | (1,1)、OR は玉との距離 | (1,1) | 逃げ道の数（LibertyEstimator）、1 手詰めと 3 手詰め | (1,1)、3 手詰めと 2 手読み |
| 千日手、GHI | `is_repetition`、不詰、`repeated` フラグで保存を抑制 | DRAW を pn = ∞、`minimum_distance` で古い子 | VisitHistory で経路上と劣等局面を千日手、経路ハッシュの千日手テーブル | `current` 印と祖先の pn しきい値を返す、AND で不詰へ変換 | 展開前にしきい値を TT へ書く（論文の方式） | PathEncoding と twin_list、LoopDetection は TT に書かない | `isDraw(16)`、REPEAT 印 |
| PV と余詰 | 証明木を辿る、最短保証なし、余詰なし | 再探索で最短と最長を選ぶ | 手数上限を 2 ずつ減らす余詰探索、FinalRange | make_tree、bns_plus（pn しきい値 +1）、比較関数で最短と最長 | 全手順を列挙し最長抵抗を選ぶ、無駄合い評点 | `retrievePV`、1 手 3 手詰め優先、最短保証なし | 証明済みの最初の手と最長の受け |
| USI 出力 | `timeout`、`none`、`nomate`、手順 | `timeout`、`nomate`、手順 | `timeout`、`nomate`、手順 | `nomate`、手順（時間切れも `nomate`） | CLI のみ | `timeout`、`nomate`、手順 | Python API |
| 並列 | なし | なし | 全スレッド同根、多様化、エントリ単位ロック | なし | なし | SMP コードは無効 | なし |
| 対局探索との接続 | ふかうら王が葉（40 節点）と PV 専用スレッドで使用。通常探索は mate_1ply のみ | なし | なし | なし | なし | なし | なし |

## 9. 中将棋への適用上の注意

中将棋には持ち駒がなく、王駒が王将と太子の 2 枚になり得て、獅子が 1 手で 2 段階移動する（RULES.md 第 4 条第 5 項、第 20 条、第 12 条）。
この 3 点から、上の実装群のどこが移植できないかを整理する。

### 9.1 持ち駒があるからこそ存在する構造

以下は、すべて持ち駒の存在に依存しており、中将棋では丸ごと不要になる。

- **証明駒と反証駒**：OSL の `proof_pieces`、`proof_pieces_candidate`、`ProofPieces`、`DisproofPieces`、`addMonopolizedPieces`（dfpnRecord.h:25-36、proofPieces.h:14-44、disproofPieces.h:14-43、proofPiecesUtil.h:23-37）、KomoringHeights の `hands.hpp` 全体（`HandSet`、`BeforeHand`、`AfterHand`、`RemoveIfHandGivesOtherChecks`、`AddIfHandGivesOtherEvasions`）と結果集計時の合成（local_expansion.hpp:557-639）、shtsume の `proof_koma*` と `disproof_koma*`（nbase.c:450-770）、cshogi `dfpn.cpp` の証明駒の和と反証駒の積（523-628、644-743）。
- **持ち駒の優越判定**：OSL の probe と proof oracle（dfpn.cc:800-835、858-876）、KomoringHeights の `hand_is_equal_or_superior` による劣等と優等の参照（ttentry.hpp:378-402）、VisitHistory の劣等局面判定（visit_history.hpp:109）、shtsume の `MKEY_COMPARE` と SUPER_TSUMI、INFER_FUDUMI の優先順位（shogi.h:477-484、ntbase.c:1104-1190）、やねうら王の結論キャッシュの `hand` と `hand_is_equal_or_superior`（mate.h:120-121、mate_dfpn.hpp:1019-1049）、やねうら王の千日手判定の `REPETITION_SUPERIOR` と `REPETITION_INFERIOR`（mate.cpp:145-149）。
- **TT のキーに持ち駒を含める、または持ち駒で層を分ける設計**：OSL の `BoardKey` と `stands` の 2 層（hashKey.h:28-65、dfpnRecord.h:60）、KomoringHeights の `BoardKeyHandPair` と `hand_` を空きエントリの印に使う設計（board_key_hand_pair.hpp:17-32、ttentry.hpp:79-83）、shtsume の mcard 層（shtsume.h:223-235）、cshogi `dfpn.cpp` の `TTEntry.hand`。中将棋では (盤面キー, 持ち駒) が盤面キーだけに縮退し、優劣を使う読み出しは完全一致に戻る。ただし KomoringHeights 型の設計では、空きエントリの印を別のフィールドに移す必要がある。
- **打ち歩詰めの検査**：OSL の `has_pawn_checkmate`、`PAWN_CHECK_MATE_PROOF`、`isPawnDropFoul`、`PawnCheckmateMoves`（dfpn.cc:1811-1819、1852-1859、proofDisproof.h:30、92-96）、shtsume の `fu_tsume_check`（base.c:360-366、呼び出しは ncheck.c:97、835、nevasion.c:2269）、やねうら王 mate1ply の `can_pawn_drop`（mate1ply_without_effect.cpp:545-549）、KomoringHeights の二歩判定（hands.hpp:147、212-216）。
- **合駒の生成と無駄合いの判定**：OSL の `generateCheapKingEscape`、`need_full_width`、`blocking_verify`、`blockingSimulation`、`SacrificeBlockCount`（escape_.tcc:31-42、dfpn.cc:2149-2174、2485-2488、3061-3109）、KomoringHeights の `DelayedMoveList` の合駒連結（delayed_move_list.hpp:206-262）と駒打ちの勝ち手に対する反証駒補正（local_expansion.hpp:562-575）、shtsume の `evasion_drop`、合駒の直列リスト、`invalid_drops`、`cross_point_check`、`is_dest_effect_valid`（nevasion.c:118-259、2245-2278、ntbase.c:739-1008）、tsumeshogi-solver の無駄合駒評点（yasai_position.rs:45-99）。中将棋にも移動合（走り駒の利きを遮る着手）はあるが、打ちによる合駒がないので、これらの「打つ合駒」専用の処理は対象がない。
- **駒打ちに依存する発見的手法**：OSL の遠距離からの打ち王手の dn 補正（dfpn.cc:1966-1983）と `hasPieceOnStand<GOLD>`（1722 行）、KomoringHeights の初期値表の駒打ち項（initial_estimation.hpp:104、138、147）、やねうら王 MovePicker の駒打ち分岐（mate_move_picker.h:166、178-179）、shtsume の駒余り欄 `hinc`、`nouse` と比較関数の駒余り優先（shtsume.h:229-230、nbase.c:341-359）。
- **経路ハッシュの駒打ち項**：KomoringHeights の `g_dropped_pr` と `g_stolen_pr`（path_keys.hpp:185-187、236-238）。

### 9.2 持ち駒なしでも引き継げる技法

次の要素は持ち駒に依存せず、中将棋でもそのまま使える。

- **しきい値更新の式**：tanuki、cshogi `dfpn.cpp`、tsumeshogi-solver の教科書的な式、KomoringHeights の φ と δ による定式化、OSL の frontier 平均の加算。
- **無限大の表現と飽和**：KomoringHeights の `u64::max/2 − 1` と `ClampPnDn`、やねうら王の `INF − N` による手数の符号化。
- **TT の配置と GC**：KomoringHeights の開番地法と探索量中央値による GC、OSL の `node_count` による smallTreeGC、shtsume の `protect` による証明木の保護。ただし、どれも「どのエントリを残すか」の基準に持ち駒を使っていないので、盤面キーだけの TT に移せる。
- **千日手と GHI**：KomoringHeights の経路上の局面を即不詰とする扱いと経路ハッシュの千日手テーブル、OSL の `PathEncoding` と `twin_list`、tsumeshogi-solver の「展開前にしきい値を TT へ書く」方式。ただし、KomoringHeights と OSL の劣等局面による千日手判定は、持ち駒がなければ盤面の完全一致だけになる。
- **TCA と二重カウント対策**：KomoringHeights の `inc_flag` と親ポインタによる分岐元探索、OSL の `findDagSource`。
- **根の反復深化と節点予算**：KomoringHeights の (1.7 + 0.3 × スレッド) 倍、OSL ドライバの節点上限の段階的拡張、ふかうら王の葉での 40 節点予算と PV 専用スレッド。
- **PV の再構成と余詰**：KomoringHeights の手数上限を 2 ずつ減らす余詰探索と `FinalRange`、shtsume の `bns_plus`、tsumeshogi-solver の全手順列挙。
- **遅延展開**：KomoringHeights の成不成の組の連結は、打ちではなく移動の属性なので引き継げる。

### 9.3 王が 1 枚であることを前提にした箇所

いずれの実装も、王の位置を 1 升で表し、「王手」と「王手回避」の二分法で OR 節点と AND 節点の手を生成する。
中将棋では王将と太子の 2 枚が盤上にあり得る（RULES.md 第 20 条）。
以下は、その前提が直接埋め込まれている箇所である。

- **王の位置を 1 升で返す関数**：やねうら王の `Position::square<KING>`（position.h:505-510）と mate1ply の各参照（mate1ply_without_effect.cpp:422-725）、KomoringHeights の `Node::KingSquare()`（node.hpp:85）、`DoesHaveMatePossibility` の `king_square`、`check_squares`、`blockers_for_king`（typedefs.hpp:314-336）、OSL の `Tree::king(P)` と `kingPiece(alt(P)).square()`（dfpn.cc:516、1607）、shtsume の `char ou[2]` と `SELF_OU`、`ENEMY_OU`（shogi.h:544、676-681）、cshogi `osl_dfpn.cpp` の `kingSquare`（37 箇所）。
- **王手生成の目標升**：OSL の `AddEffectWithEffect::generate(state, kingPiece(alt(P)).square(), ...)`（dfpn.cc:1606-1607）、shtsume の相手玉 1 枚への利き表（ncheck.c:86、451、826、887）。やねうら王と KomoringHeights が使う `CHECKS_ALL` と `in_check` は、`checkersBB = attackers_to(square<KING>(sideToMove))`（`R/YaneuraOu/source/position.cpp:307`、329）のように 1 升の王への利きで王手を定義する。
- **回避手生成**：やねうら王と KomoringHeights の `EVASIONS_ALL`、OSL の `generateKingEscape`（dfpn.cc:1597、2130）、shtsume の `evasion_bb` と `generate_evasion`（bitboard.c:241-275、nevasion.c:94-105）。
- **「回避手がなければ詰み」という終端判定**：やねうら王（mate_dfpn.hpp:1194）、tanuki（576-580 行）、shtsume（shtsume.c:558-563）、OSL の `NoEscape`。
- **王手の検出**：OSL の `inCheck` と `inUnblockableCheck`（dfpn.cc:444）、shtsume の `oute_check` が `SELF_OU` だけを見る点（base.c:1202-1249）、KomoringHeights の `checkers().pop_count() != 1` と `between_bb`（hands.hpp:198-206）。
- **根が OR か AND かの判定**：KomoringHeights は `king_square(...) == SQ_NB` と `in_check` で決める（user-search.cpp:20-26）。
- **ピンと開き王手**：shtsume の `create_pin`、`set_discpin`、`set_tpin`（base.c:369、1251、ntbase.c:1290-1404）。
- **王を取る手の扱い**：OSL の `OracleAdjust` は王を取る手を無効として扱う（oracleAdjust.h:22-23）。中将棋では、王将を取っても太子が残れば対局は続く（RULES.md 第 20 条第 3 項）ので、王駒の捕獲は終局ではなく通常の捕獲になり得る。

これらの箇所を直すだけでは移植にならない。
そもそも「攻め方は王手、受け方は王手回避」という枠組みが本将棋の規則に依拠している。
中将棋では王手の放置が合法であり（RULES.md 第 8 条第 3 項から第 5 項）、終局は最後の王駒の捕獲または詰みで決まる（第 21 条）。
したがって AND 節点の手は回避手ではなく全合法手にする必要がある。
終端判定も 2 種類になる。
第 1 に、OR 節点で相手の最後の王駒を取る着手は pn = 0 の終端であり（第 21 条第 1 項）、AND 節点で「合法手がなければ負け」（第 23 条）は副次的な pn = 0 の終端である。
第 2 に、受け方が攻め方の最後の王駒を先に取る着手は dn = 0 の終端になる（第 21 条第 3 項 b および第 4 項）。
本将棋のソルバは、王手を受けた攻め方の手を逆王手になる回避手に限ることで後者を排除しているが、中将棋では攻め方が王手を放置できるので、この終端を明示的に持つ必要がある。
一方、OR 節点を「王手」に限定する意味は、相手の最後の王駒を次に取れる状態に限るという意味に変わる。
相手が王駒を 2 枚持つ局面では、1 枚への攻撃は勝ちにつながらないので、OR 節点の候補手をどう絞るか（両方を同時に取る獅子の着手、片方の捕獲後に残る 1 枚への詰みなど）は本将棋の実装から持ち込めず、設計で決める必要がある。
HaChu は alpha-beta 側の対応として、王駒が 2 枚あるときは王手判定を省く「two kings is no king」という扱いをしている（hachu.c:2052-2056）。

### 9.4 獅子の 2 段階移動と手の表現

獅子の 2 段階移動（RULES.md 第 12 条）は、着手の表現と経路ハッシュに影響する。

- **着手の符号化**：やねうら王の `Node::lastMove` は 23 bit（mate_dfpn.hpp:215、「Moveは21bitであることが保証されている」）、shtsume の `move_t` は 2 B で 7 bit の升と成りビット（shogi.h:755-774）である。中将棋の 2 段階移動は from、中間升、to の 3 升を要する。組合せの数（移動元 144、中間升なしを含めて 145、移動先 144、成り 2 の積 6,013,440）は 2^23 未満なので 23 bit に収まらないわけではないが、既存の符号化はそのまま使えない。
- **経路ハッシュ**：OSL の `PathEncoding` は 1 手を from と to の 1 区間として足し込む（pathEncoding.h:28-35）。2 段階移動を 1 手として符号化するには、中間升を含めた拡張が要る。KomoringHeights の経路ハッシュも from、to、駒種の組で引く（path_keys.hpp:233-248）。
- **子の数の上限**：やねうら王の `MaxCheckMoves = 100`（mate_move_picker.h:21-24）と `u8 child_num`、KomoringHeights の `kMaxCheckMovesPerNode = 110` と `BitSet64 sum_mask_`、OSL の `solved` 64 bit と `CheckOrEscapeMaxUniqMoves = 150`（`R/oslmate/osl/osl/core/osl/container.h:299`）は、本将棋の王手数の上限を念頭に決めたとみられるが、やねうら王の `mate_move_picker.h:21` は「91通り？」というコメントであり、OSL の `container.h:299` は `Move::MaxUniqMoves/4` と定義しているので、91 の厳密な上限性と各定数の由来は未確認である。中将棋の 12 × 12 盤で 46 枚の駒と獅子の 2 段階移動を含む合法手数、およびその中の「王手」数の上限は未確認であり、設計時に実測して決める必要がある。
- **1 手詰め判定**：やねうら王の mate1ply と OSL の `FixedDepthSolverExt`、KomoringHeights の `CheckMate1Ply` は、王 1 枚への利きと駒打ちを前提にした表引きであり、そのまま移植できない。中将棋では、葉で mate1ply を呼ぶ代わりに (1, 1) の初期値だけで始めるのが現実的な出発点になる。

### 9.5 設計への含意

以上から、中将棋向けの詰み探索は、本将棋の実装群から「しきい値更新、無限大と飽和、TT の配置と GC、経路ハッシュによる千日手処理、TCA、根の反復深化、PV 再構成」を引き継ぎ、「証明駒と反証駒、優越判定、打ち歩詰め、合駒、1 手詰め表、王手と回避の二分法による手生成」を捨てることになる。
ライセンスの面では、shtsume（MIT）と OSL 部分（BSD 系）以外は GPL v3 なので、minase（MIT）へはコードを転記せず設計だけを参考にする。
tsumeshogi-solver も GPL v3 であり、そのトレイト設計（局面、TT、探索の 3 層分離）は、minase-core の合法手生成器を局面トレイトの実装として差し込む形の設計上の参考にとどめる。

## 参考

- 長井歩、今井浩「df-pn アルゴリズムの詰将棋を解くプログラムへの応用」情報処理学会論文誌 43(6)、2002 年（tsumeshogi-solver と KomoringHeights が典拠に挙げる）。
- Kishimoto, A. "Dealing with infinite loops, underestimation, and overestimation of depth-first proof-number search." AAAI 2010（TCA と二重カウント。tanuki と KomoringHeights が典拠に挙げる）。
- TadaoYamaoka の開発日記、2026 年 4 月 4 日「GPS将棋(OpenShogiLib)のdf-pn(詰将棋)をUSI mateエンジンとして移植」、6 月 16 日「cshogiにGPS将棋のdf-pnを移植」、6 月 17 日「cshogiにGPS将棋のdf-pnを移植（その３ ソース解説）」。
- KomoringHeights `source/engine/user-engine/docs/refs.md`（上記のほか脊尾 1999、Nagai 2002、Kishimoto と Müller 2004、金子ら 2010、Song ら 2016 を挙げる）。
