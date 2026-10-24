# 自己生成データで評価関数を育てたエンジンの教師生成

本書は[強豪エンジンの教師生成と自己参照の偏りへの対策](../teacher-generation-strong-engines.md)の付録である。

本書は、外部の強い教師に頼らずに評価関数を育てたチェスエンジンと、Fairy-Stockfishの変種用NNUEについて、教師データの生成手順を一次資料で確かめた結果である。
NNUEは、差分更新できる小さなニューラルネット評価関数を指す。
参照日は2026年10月25日である。
探索値と対局結果の混合比、Viridithasのbasilisk群・eleison・noumenaの比較、およびFairy-Stockfishの手引きが挙げる「少なくとも1億局面」の目安は、minaseの既存の調査 `docs/research/chess-evaluation-targets.md`、`docs/research/chess-shogi-teacher-mixing.md`、`docs/research/nnue-architecture-discussion.md` が扱っているので、本書では参照するだけにとどめる。

## 結論

乱数で初期化したネットから始めたことを一次資料で確認できたチェスエンジンは、Astra、Zevra 2、Sapling、Prune、Stormphraxの5つである。
PZChessBotもゼロ知識から学んだと書くが、乱数の重みから始めたとまでは書いていない。
このうち世代ごとの記録まで公開しているのはAstra、Zevra 2、Saplingの3つであり、Pruneは反復の規則を文書にしている。
記録の規模は、Astraが21世代で約120億局面、Zevra 2が2.6までに11世代と2.7でさらに4世代、Saplingが21世代である。
StormphraxとPZChessBotについては、READMEの記述だけを確認した。
Viridithas、Integral、Akimbo、PlentyChessも自前のデータだけで学ぶが、初代の教師は手作り評価関数（以下HCE、hand-crafted evaluation）、駒価値、または公開の駒位置表PeSTOを種にしており、厳密なゼロ知識ではない。
Fairy-Stockfishの手引きは、初代をHCEによる深さ2の自己対局から作り、2代目以降は現行最良のネットで生成するよう勧める。
つまり、外部の強い教師を使わない開発の大半は、手作りの弱い評価関数か乱数のネットを種にし、自己対局と学習の反復で強さを積み上げている。

チェスの自己対局の生成手順は、エンジンが違ってもほぼ同じ型に収束している。
開始局面は初期局面またはチェス960の配置から8〜9手（Zevraは12手）をランダムに指して作り、深さ10の探索や5万ノードの探索で検証して、評価値の絶対値が300〜1,000 cpを超える開始局面を捨てる。
cpはポーン1枚の100分の1を単位とする評価値である。
1手の探索量は多くが5,000〜30,000ノードであり、開始局面を作った後はランダムな手を指さない。
生成中の裁定は、評価値の絶対値2,000〜3,000 cpが4〜5手続けば勝ち、0〜5 cp以内が8〜12手続けば引き分けとする型が多い。
王手中の局面、最善手が捕獲の局面、序盤の16手前後までの局面は、生成時か学習時のどちらかで除く。
Fairy-Stockfishの生成器だけは、24手目までの5手を探索上位5手からの抽選で指すという、対局途中のランダム化を既定にしている。

自己参照による偏りの強化を、特定の駒の配置癖のような形で測った記録は、調べた資料の中に1つもなかった。
開発者が偏りや過学習に触れた記述は、Viridithas、Sapling、Fairy-Stockfishの3つの資料に限られる。
Viridithasの作者は、自分のネットで教師を付け直したときに「worried about overfitting」と記し、新しい構造のネットに自己対局させる理由を「to reveal any halfka-induced overfitting to help the next net」と書いた。
Saplingの生成器は、開始配置とその色反転を対にして指す理由を「cancels any colour or first-searched-move bias in the WDL labels」と注記している。
Fairy-Stockfishの手引きは、王手回数を入力に持たないネットだけで3checkの教師を作ってはならないと書く。
偏りそのものではないが、教師の誤りがどれほど効くかを示す記録として、Zevra 2は生成器の不具合で壊れていた教師の値を2.7で直し、それだけで約+170 Eloを得たと報告した。

偏りを抑える方向に働く仕組みで効果の実測が記録されているのは、対局結果（以下WDL、win/draw/lossの略）への重み、データの蓄積、開始局面の多様化の3つである。
Saplingは乱数のネットが生成した初代の教師を対局結果だけで学び、ネットが育つにつれて対局結果の重みを0.3まで下げた。
Astraは13世代目のネットを対局結果だけで微調整して+47.35 ± 7.76 Eloを得ており、学習器bulletの例も最後の段階を対局結果だけで学ぶ。
Viridithasは、最新ネットで作った小さな新データだけで学ぶと106.2 Elo負け、累積データへ混ぜると勝つことを繰り返し記録し、同じ量なら新しいデータの方が古いデータより強いことも示した。
Saplingはチェス960の4億局面を加えて、チェス960の短時間対局で+132 Eloを記録した。
ただし、これらは棋力全体の差であり、特定の偏りが消えたことを示す測定ではない。

Fairy-Stockfishは盤の大きさが最大12×10升、NNUEの駒種が15種までなので、12×12升で成駒を含めて29種の駒を持つ中将棋を改変なしでは扱えない。
公開されているFairy-Stockfishの変種NNUEの一覧で最も大きい盤は、12×8升のcourierと、12×10升の格子の外周を四隅以外塞いだomicronであり、中将棋や12×12升以上の盤でNNUEを学んだ公開例は見つからなかった。
したがって中将棋のNNUEの教師生成について直接の先例はなく、minaseの設計はチェスと小さな盤の変種の慣行から類推するしかない。

## 調べた版と資料

各リポジトリは、調査用の作業領域に浅く複製し、コードを実行せずに読んだ。
Fairy-Stockfishの関連リポジトリは、作業機にある複製が上流と異なるフォークであるため、上流の最新コミットを別に取得し、上流のURLと行番号で引用する。

| 対象 | リポジトリ | コミット | 日付 | 読んだもの |
|---|---|---|---|---|
| Astra | h1me01/Astra | `689ba7888e21635954ef3b9d6a6748087400a04d`（main）、`ca8b837c77cf20d61a3b59d3bb2c4819f909bf27`（selfgen） | 2026-06-22、2026-05-20 | README、selfgenブランチの生成コードとコミット文 |
| Zevra 2 | sovaz1997/Zevra2 | `cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021` | 2026-08-31 | README、生成コード、v2.6とv2.7のリリース説明、Hugging Faceのデータ説明 |
| Sapling | Timmoth/Sapling | `2b43285d97fa4a9e49ce1d08fd4155a1fa950724` | 2026-10-24 | ネットの履歴 `log.txt`、現行と2024年10月の生成コード |
| Viridithas | cosmobobak/viridithas | `b407e3eb0b8fc6e251d22664b696cfa870208a7a` | 2026-10-22 | README、生成コード、ネットの履歴 `notes/networkhistory.txt` |
| PlentyChess | Yoshie2000/PlentyChess | `8bf271f462b5ae01a3bb8be6cdb30de19e6bdbd4` | 2026-10-24 | README、生成コード、プルリクエスト#279の説明 |
| Integral | aronpetko/integral | `bed1786be517489cc8d924339c7ec58311ac3a0f` | 2026-10-23 | README、生成コード |
| Akimbo | jw1912/akimbo | `f7dd7677d6ab9a4a76e3769731084894a7f7ae09`（main）、`3bfea08c8c56c462a28cb1f3b76a442938e8085f`（v1.0.0） | 2025-02-07、2024-03-26 | README、v1.0.0の生成コード |
| Stormphrax | Ciekce/Stormphrax | `d1468d99e3d3733100d20d682ac627998cce94b2` | 2026-08-29 | READMEだけ |
| Prune | Arzam18/Prune | `493632de645c7fe17a635b7de42d132e1024bd38` | 2026-10-04 | README、`TRAINING.md` |
| PZChessBot | kevlu8/PZChessBot | `1fb0df2336fb71ba5e364176fd07e2fbb968177a` | 2026-10-23 | READMEだけ |
| Carp | dede1751/carp | `7d5065bc26dd16002e6581b25abb3decf62ed86c` | 2025-01-18 | README、生成コード |
| OpenBench | AndyGrant/OpenBench | `dd03aaff9e855e8b922f5d365ad5b74a670560f6`、wiki `52cb0ee156fa8af88a15cd4bf29dda1fda6b4380` | 2026-10-21、2026-08-31 | データ生成の作業種別とwiki |
| bullet | jw1912/bullet | `579b78b37a694cacd6e9ede4706386c15c34837c` | 2026-10-20 | `docs/3-data.md`、`examples/advanced` |
| variant-nnue-tools | fairy-stockfish/variant-nnue-tools | `445b53c419d03d9167a786140a4d4bbe12ee7426`（tools） | 2026-09-25 | 生成器のソースと `docs/` |
| variant-nnue-pytorch | fairy-stockfish/variant-nnue-pytorch | `b15df38a9aae8ab9b40b2378020b3099c7c5d179`、wiki `668804196697bb157ae3f066382e60a8c46f0215` | 2026-06-30、2026-02-19 | 学習器のソースとwiki |
| Fairy-Stockfish | fairy-stockfish/Fairy-Stockfish | `2e591089558a5afa72ab5a22192208e71848a30c`、wiki `36b9aba92661d3066b92242b2a88f7aa5aa5b278` | 2026-09-20、2026-09-24 | 盤の大きさの定義、wikiのNNUEの頁 |
| Fairy-Stockfishの公式サイト | fairy-stockfish/fairy-stockfish.github.io | `a07784b0631c6f9158e651747242a587ba152cf7` | 2026-09-19 | 公開ネットの一覧 `nnue.markdown` |

このほか、TalkChessのスレッド「Generating original training data」（2025年2月）にある、SaplingとZevraとLeorikの作者自身の投稿を、生のHTMLで文言を照合したうえで使った。
Viridithasの作者のブログ記事（2026年2月12日）も使った。

## 比較表

次の表は、各エンジンについて共通の11項目を並べたものである。
列の見出しは、共通の11項目（生成者、1手の探索量、開始局面の多様化、対局途中のランダム化、生成中の裁定、局面の選別、教師値、データ量、世代の回し方、偏りへの言及と対策、初代の作り方）の略称である。
「未」は未確認を表し、詳細と典拠は各節に書く。
表中のTBは終盤データベース、DFRCは白と黒が別々のチェス960の配置を持つ開始局面を指す。

| エンジン | 生成者 | 探索量 | 開始局面 | 途中の乱数 | 裁定 | 選別 | 教師値 | データ量 | 世代の回し方 | 偏りへの言及 | 初代 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Astra | 自分 | 未 | 初期局面から一様ランダム8手、または定跡から4〜5手。深さ10で絶対値800 cp超を捨てる | 未 | 未 | 8手目未満を学習時に除く例あり | 探索値と結果。比は未、v14は結果だけで微調整 | 累計約120億局面、1世代500万〜30億局面 | 併合データ。v16は旧世代のデータを再併合 | 記述なし | 乱数の重み |
| Zevra 2 | 自分（前世代の最強ネット） | 5,000ノード固定 | 初期局面から一様ランダム12手 | なし | 裁定なし、詰み・規則上の引き分けまで指す | 生成時に王手中、最善手が捕獲・成り、詰みの値を除く | 探索値と結果を記録。比は未 | 2.6で10億局面、2.7で約15億局面、累計約18億局面 | 毎世代、乱数から学び直す | ラベル破損の修正で約+170 Elo | 乱数の重みのネット（768→8） |
| Sapling | 自分 | 6,500ノード（2024年版）、現行は5,000〜30,000の対数一様 | 初期局面からランダム9手。2024年10月13日以降はチェス960の配置から9手、現行は8手 | なし | 2,500 cp以上が5手続けば勝ち。60手以降、50手規則の時計が20以上で0が4回続けば引き分け | 2024年版は生成時に静かな手と非王手だけ。net 21は8手目未満を除く | 探索値と結果。結果の重みを1.0から0.3へ下げた | 1世代1,000万〜25.5億局面 | 未 | 色反転の対局で色と探索順の偏りを打ち消す | 乱数の重み |
| Viridithas | 自分 | 25,000ソフトノード | DFRCの配置からランダム8〜9手。5万ノードで絶対値1,000 cp超を捨てる | なし | 2,500 cp以上が4手で勝ち、4 cp以下が12手で引き分け、TBで終局 | 学習時に16手目未満、戦術的な手、王手中、2万 cp以上を除く | 探索値と結果。比は既存調査を参照 | 小さな新データ単独は負け、累積に混ぜると勝ち | 新規学習と微調整の併用、古いデータの除外や新データの重み増しを比較 | 過学習の懸念、自己対局で過学習を露出させる意図 | 旧版の対局をHCEの低深度探索で付け直し |
| PlentyChess | 自分 | 未 | 初期局面からランダム8〜9手、駒種別の確率で選ぶ。定跡からは3〜4手。深さ10かつ100万ノードで絶対値1,000 cp以上を捨てる | なし | 未 | 未（最良の72億局面を使用） | 未（一部に自己蒸留） | 約105億局面を生成、累計150億局面以上 | 未 | 記述なし | PeSTOの駒位置表 |
| Integral | 自分 | 20,000ソフトノード | ランダム8〜9手。駒種と行き先の升で重み付け。多くはUHO_Lichess定跡から3〜4手。深さ10で絶対値300 cp以上を捨てる | なし | 2,500 cp以上が5手で勝ち、200手以降に2 cp以下が8手で引き分け、5駒TBで探索 | 未（全手を記録） | 未 | 数億局 | 初期のデータは捨てた | 記述なし | v4のHCEのデータ |
| Akimbo 1.0.0まで | 自分 | 5,000ソフトノード | 一様ランダム8〜9手 | なし | 3,000 cp超の1手で勝ち、引き分けは規則だけ | 生成時に最善手が捕獲と王手中を除く | 未 | 未 | 未 | 記述なし | 駒価値から始めたHCEの反復の最終データ |
| Fairy-Stockfish | 自分（現行最良ネット、初回はHCE） | 深さ3（既定）、手引きの例は深さ2、深さ4〜5を推奨 | 定跡ファイルは任意 | 24手目までの5手を上位5手から抽選（既定） | 3,000 cp以上が4手で投了（80%の局）、80手以降に0が8手で引き分け | 生成時に5手目未満、王手中、既出局面、最善手が捕獲を除く | 探索値。学習器の既定は探索値だけ | 1回1億局面（既定）、少なくとも1億局面を推奨 | 乱数からでも続きからでもよい | 3checkはネットだけで生成しない | HCEの深さ2、ネットの駒位置表部分は駒価値で初期化 |

## 乱数の重みから始めたエンジン

### Astra

Astraは、6.1.1までLeela Chess Zero（以下lc0）の対局で学んだネットを使っていたが、その後、乱数で初期化したネットから自己対局を繰り返して21本のネットを学んだ。
最後に既存の採用版を破ったネットが現行の重みであり、経緯はselfgenブランチに残っている。
学習には自作の学習器Soreiを使い、約120億局面をStockfishのbinpack形式で保存している。
[README 12〜17行](https://github.com/h1me01/Astra/blob/689ba7888e21635954ef3b9d6a6748087400a04d/README.md#L12-L17)

selfgenブランチの生成コードは、開始局面を作る部分だけを持つ。
定跡を与えない場合は初期局面から合法手を一様に8手指し、定跡を与えた場合はその局面から4手か5手を指す。
そのうえで深さ10の探索を行い、評価値の絶対値が800 cp以下の局面だけを出力する。
[gen_fens.cpp 25〜109行](https://github.com/h1me01/Astra/blob/ca8b837c77cf20d61a3b59d3bb2c4819f909bf27/src/datagen/gen_fens.cpp#L25-L109)
この出力形式 `info string genfens <局面>` は、後述するOpenBenchのデータ生成の取り決めである。
推論として、対局の進行、1手のノード数、勝敗の裁定はOpenBenchの作業設定で決まり、リポジトリには残っていない。

世代ごとの記録はコミット文にあり、各コミットにはSPRT（逐次確率比検定）による対局結果が付いている。
コミット文は比較相手を明記しておらず、初回だけに「vs random weights」とある。
以後の行は直前の採用ネットとの比較と読むのが自然だが、これは推論である。
次の表は主な世代を抜き出したものである。

| コミット | 日付 | コミット文の内容 | Elo差 | 持ち時間 |
|---|---|---|---|---|
| [7425b6d](https://github.com/h1me01/Astra/commit/7425b6d67af36ecb0bd5a74ad8c55143f4befd1c) | 2026-01-31 | 乱数の重みに対して、自己生成の約500万局面で学んだ重み | +1199.83（714局全勝） | 10秒＋1手0.1秒 |
| [6d407e3](https://github.com/h1me01/Astra/commit/6d407e33486096184ba8eb3bbb2995911c98fa81) | 2026-02-01 | 新しいデータ、重み、構造 | +542.76 ± 76.97 | 10秒＋1手0.1秒 |
| [8d60c41](https://github.com/h1me01/Astra/commit/8d60c41b66b0a3ad62cf865bb1bcad27b3586550) | 2026-02-01 | 新しいデータ、重み、構造 | +676.81 ± 119.46 | 10秒＋1手0.1秒 |
| [b81b4e3](https://github.com/h1me01/Astra/commit/b81b4e377aec989435ec8bd508470beb713fa5ef) | 2026-02-02 | v4、新しく生成した約1.25億局面 | +222.98 ± 23.07 | 10秒＋1手0.1秒 |
| [738c49e](https://github.com/h1me01/Astra/commit/738c49e694481fa7e27a234892bb2e9889a7afce) | 2026-02-03 | v5、新しく生成した約1.5億局面 | +32.97 ± 6.82 | 10秒＋1手0.1秒 |
| [5bbbb5f](https://github.com/h1me01/Astra/commit/5bbbb5f81504dbb432402f4fc13100a0d7438c9b) | 2026-02-05 | v6、新しい構造、新しく生成した約3.5億局面 | +33.05 ± 6.23 | 10秒＋1手0.1秒 |
| [6395c88](https://github.com/h1me01/Astra/commit/6395c886f33fa14522b312f976e665c85dae51d1) | 2026-03-12 | v7、新しい構造、新しく生成した約25億局面 | +94.62 ± 10.52 | 10秒＋1手0.1秒 |
| [ed3bbef](https://github.com/h1me01/Astra/commit/ed3bbefbdd910897cb65cb3e15ce40755fde8b3a) | 2026-04-03 | v8、新しい構造、新しく生成した約20億局面 | +9.43 ± 3.58 | 10秒＋1手0.1秒 |
| [11f8700](https://github.com/h1me01/Astra/commit/11f8700d7490425a7158f50ecd222a998e9d64ce) | 2026-04-18 | v11、新しく生成した約30億局面 | +23.54 ± 5.66 | 10秒＋1手0.1秒 |
| [e6ed63f](https://github.com/h1me01/Astra/commit/e6ed63feda97f0a0b76404d402fa553cb5ad93e4) | 2026-04-18 | v14、v13の重みを対局結果だけで微調整 | +47.35 ± 7.76 | 10秒＋1手0.1秒 |
| [e981c78](https://github.com/h1me01/Astra/commit/e981c78e0e6686eb226d2ff82362990cebe45b48) | 2026-04-22 | v16、v15のデータにv9のデータを併合 | +13.92 ± 4.36 | 10秒＋1手0.1秒 |
| [bb51756](https://github.com/h1me01/Astra/commit/bb517563c236bb4a45b2d2a1f6e0ecb5d9392c5a) | 2026-04-28 | v17、v16のデータに新しい20億局面を併合 | +7.29 ± 3.08 | 10秒＋1手0.1秒 |
| [7834a1b](https://github.com/h1me01/Astra/commit/7834a1bede67c226bfcab9a7d2df5da46223b8fe) | 2026-05-14 | v19、v18のデータに新しい約23億局面を追加 | +5.16 ± 3.06 | 8秒＋1手0.08秒 |
| [f8c2a63](https://github.com/h1me01/Astra/commit/f8c2a639538d25435dc5e57b3c8fba7a2e79c047) | 2026-05-19 | v20、新しい構造、v19のデータに新しい16億局面 | +9.11 ± 4.25 | 8秒＋1手0.08秒 |
| [ca8b837](https://github.com/h1me01/Astra/commit/ca8b837c77cf20d61a3b59d3bb2c4819f909bf27) | 2026-05-20 | v21、新しい構造、v20と同じデータ | +8.56 ± 3.93 | 60秒＋1手0.6秒 |

利得は最初の数世代で数百Eloに達し、v8以降は、v14の微調整を除いて1世代あたり5〜25 Elo程度に縮んだ。
データは世代ごとに捨てず、新しく生成した局面を前の世代のデータへ併合しており、v16では過去に生成したv9のデータを再び学習データに加えている。
v14の微調整の記録は、それまでのネットの上で対局結果だけを教師にする仕上げが大きく効いた例である。
v8とv11の間には、v10と同じデータを「linear wdl」で学び直したコミットもあるが、混合比の値はコミット文に書かれていない。
[cd4e229](https://github.com/h1me01/Astra/commit/cd4e2298b049960813a6742a583a943ac6203412)
各世代を乱数から学び直したか、前のネットから続けたかは、v14の微調整を除いて記載がない。

### Zevra 2

Zevra 2は、2.6で初めてNNUEを採用した。
2.6のリリース説明は、ネットをHCEの評価値を使わずにゼロから学んだと述べ、初代は乱数の重みのネットによる自己対局で学んだ768→8の小さなネットであり、11世代目は前のネットによる10億局面の自己対局で学んだとする。
2.6のネットを使う版は、HCE版に対して持ち時間60秒＋1手0.6秒の3,890局で2363勝1004敗523分、+126.7 ± 10.7 Eloだった。
[v2.6のリリース説明](https://github.com/sovaz1997/Zevra2/releases/tag/v2.6)

2.7では、各世代の最強ネットが自己対局の教師を作り、その教師で新しいネットを乱数から学び直し、勝ったネットが次の世代を生成するという反復をg0からg3まで回した。
[README 16〜21行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/README.md#L16-L21)、[v2.7のリリース説明](https://github.com/sovaz1997/Zevra2/releases/tag/v2.7)
2.7のネットはg3の時点で使えた約15億局面で学び、公開したデータは累計約18億局面である。
[Hugging Faceのデータ説明](https://huggingface.co/datasets/sovaz1997/zevra-selfplay)
READMEとリリース説明は、生成器の不具合で教師の値が壊れていたことを修正し、正しいデータで学び直しただけで約+170 Eloを得たとも書いている。
各世代の学習が累積データを使うのか、その世代の対局だけを使うのかは、「trained from scratch on them」という記述からは確定できない。

生成コードは、初期局面から合法手を一様に12手指し、以後は1手5,000ノードで探索して指す。
[dataset.c 9行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/src/dataset.c#L9)、[同81〜83行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/src/dataset.c#L81-L83)
勝ちや引き分けの裁定はなく、詰み、ステイルメイト、または規則上の引き分けまで指し続ける。
王手中の局面、最善手が捕獲、アンパッサン、成りの局面、詰みの評価値の局面、盤上に王以外の駒がない局面は保存しない。
[同87〜148行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/src/dataset.c#L87-L148)
エンジンは指し手をばらつかせるTemperatureの設定を持つが、生成時は0に固定している。
[同155行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/src/dataset.c#L155)

作者は2025年2月8日のTalkChessへの投稿で、幅32のネットで7〜8回反復した時点でHCE版の強さに追いつき、最初の10〜12手をランダムにして残りを保存し、数時間で約1億局面を生成できたと書いた。
[TalkChessのスレッド](https://www.talkchess.com/viewtopic.php?p=976198)
対局結果と探索値の混合比は、非公開の学習器で決めており確認できなかった。

### Sapling

Saplingのネットの履歴 `log.txt` は、乱数のバイト列をそのまま重みとした00番から21番までの学習条件を残している。
[log.txt](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt)
WDLの欄はbulletの対局結果側の重みであり、その向きは既存の調査 `docs/research/chess-evaluation-targets.md` が確かめている。

| ネット | 局面数 | 対局結果の重み | 備考 |
|---|---|---|---|
| 00 | なし | 1.0 | 乱数のバイト列 |
| 01 | 1,000万 | 1.0 | 隠れ層256 |
| 02〜04 | 1,500万〜3,000万 | 0.9 | |
| 05 | 5,500万 | 0.8 | |
| 06 | 1.1億 | 0.75 | |
| 07〜11 | 1.75億〜7.5億 | 0.7 | |
| 12〜16 | 10億〜15億 | 0.4 | 隠れ層を512、768、1024へ拡大 |
| 17〜19 | 15億〜17億 | 0.3 | 入力に王の位置のバケットを追加 |
| 20 | 20億（うちチェス960が4億） | 0.3 | |
| 21 | 25.5億（1,850万局、うちチェス960が10億） | 0.3 | 8手目未満を除く、学習器bulletの学習単位であるスーパーバッチを600回、延べ600億局面 |

初代を対局結果だけで学び、ネットが育つにつれて探索値の比重を上げるスケジュールは、乱数のネットの探索値には情報がほとんどないことへの対処と読める。
これは推論であり、作者自身は理由を書いていない。
各ネットを乱数から学び直したか、前のネットから続けたかは記載がない。
21番の項は2回書かれており、後の「Corrected entry」を採った。

生成コードは2回大きく変わっている。
2024年10月8日の版は、初期局面から合法手を一様に9手指し、以後は1手6,500ノードで探索し、最善手が静かな手で王手中でない局面だけを生成時に保存した。
[DataGenerator.cs（2024年10月8日）155〜200行](https://github.com/Timmoth/Sapling/blob/ede65a4606bffeed9ad3a2c1f3755a679e7b69b0/src/Sapling.Engine/DataGen/DataGenerator.cs#L155-L200)
勝ちは評価値の絶対値2,500 cp以上が5手続いた時点で裁定し、引き分けは60手以降、50手規則の時計が20以上で、評価値0が4回続いた時点で裁定する。
[同112〜116行](https://github.com/Timmoth/Sapling/blob/ede65a4606bffeed9ad3a2c1f3755a679e7b69b0/src/Sapling.Engine/DataGen/DataGenerator.cs#L112-L116)
2024年10月13日のコミットは開始配置をチェス960の960通りに変え、「Added 400m FRC datapoints -> +132 ELO STC FRC」と記した。
FRCはチェス960の別名であり、STCは短い持ち時間の対局を指す。
[ce8f0f9](https://github.com/Timmoth/Sapling/commit/ce8f0f9c909edca82a541bbf4dbcc6cc69726fb5)

2026年10月15日以降の現行コードは、さらに2つの仕組みを加えた。
1つは1局ごとに探索ノード数を5,000から30,000の間で対数一様に抽選する仕組みであり、注釈は「so the net sees a spread of search strengths instead of a single fixed depth」と理由を書く。
[DataGenerator.cs 57〜65行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/DataGen/DataGenerator.cs#L57-L65)、[同199〜203行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/DataGen/DataGenerator.cs#L199-L203)
もう1つは、同じ開始配置とその色を反転した配置を対にして2局指す仕組みであり、注釈は「cancels any colour or first-searched-move bias in the WDL labels」と理由を書く。
[同93〜99行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/DataGen/DataGenerator.cs#L93-L99)、[同233〜247行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/DataGen/DataGenerator.cs#L233-L247)
ただしコードでは、開始配置の後の8手のランダムな手は1局ごとに独立に選ばれるので、対の2局がランダムな手の後まで鏡像になるわけではない。
[同277〜292行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/DataGen/DataGenerator.cs#L277-L292)
この2つの仕組みは21番以降のネットにしか効いておらず、01〜20番は2026年10月15日より前の生成器で作られた。
2024年10月8日より前の生成器は本調査では読んでいない。

作者は2025年2月7日のTalkChessへの投稿で、HCEを使わず乱数の重みから自己対局で学んだと述べている。
同じ投稿によれば、最初の5〜10回の反復はそれぞれ数時間で大きな利得があり、次の10回は1回に数日から数週間かかり、データが良くなればデータを使い回せるようになった。
助言としては、ランダムな7〜8手から始めること、大きな評価値が数手続いたら勝ちと裁定すること、5,000ノードのような固定の上限を置くことを挙げ、1台で毎秒約4,000局面を生成したと書いた。
[TalkChessのスレッド](https://www.talkchess.com/viewtopic.php?p=976198)

### Prune、Stormphrax、PZChessBot

Pruneは、乱数のネットからデータを生成し、自己生成のデータをbulletで学ぶと書いている。
[README 10〜13行](https://github.com/Arzam18/Prune/blob/493632de645c7fe17a635b7de42d132e1024bd38/README.md#L10-L13)
`TRAINING.md` は反復の規則を次のように定める。
乱数のネットから始め、初期の反復では約10万局を生成し、開始局面はUHO定跡から取り、viriformat形式で保存し、対局結果の重み0.75で学び、viriformatの既定の除外を使う。
新しいネットが前のネットより強ければ、新しいネットで次の生成を行い、そうでなければ生成局数を増やす。
[TRAINING.md](https://github.com/Arzam18/Prune/blob/493632de645c7fe17a635b7de42d132e1024bd38/TRAINING.md)
推論として、この規則は、勝てない世代では生成器を更新せずにデータ量を増やすので、劣化したネットに次の教師を作らせない仕組みとして働く。

Stormphraxは、NNUEを「trained from zero knowledge starting with random weights」と述べ、自己生成のデータだけを使い、第三者の生成したデータは決して使わないと書く。
[README 14〜16行](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/README.md#L14-L16)
現行のネットはbulletで学び、ごく初期のネットはMarlinflowで学んだ。
[同99行](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/README.md#L99)
生成コードは読んでおらず、項目2から10は未確認である。

PZChessBotは「Trained from zero-knowledge using self-play games」と書き、学習器にbulletを挙げるが、初代のネットを乱数の重みにしたかどうかは書いていない。
[README 91行](https://github.com/kevlu8/PZChessBot/blob/1fb0df2336fb71ba5e364176fd07e2fbb968177a/README.md#L91)
それ以上の手順は確認していない。

## 手作りの評価関数などを種にしたエンジン

### Viridithas

Viridithasの評価関数は、PeSTOの駒位置表から始まり、Lichessの対局を集めたLichess EliteでTexel法により調整し直し、さらに自己対局で調整を続けたHCEを経ている。
最初のNNUEは、2.5.0から2.7.0までの版の対局を、開発中の2.7.0の低深度探索で付け直したデータで学んだ。
13世代目以降の多くのネットは、Lichess Eliteの局面をViridithasが付け直したデータも含み、7.0.0から8.0.0の間に定跡なしで局面の多様性を確保する生成器が書かれた。
作者は、他のエンジンの出力で学んだネットは1つもないと明言している。
[README 51〜65行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/README.md#L51-L65)

現行の生成器は、既定で1手25,000ソフトノード（目安として止める探索量）、上限はその8倍で探索する。
[datagen.rs 121〜122行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L121-L122)、[同470〜474行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L470-L474)
開始局面は既定でDFRCの配置を選ぶ。
DFRCは、白と黒が別々のチェス960の配置を持つ変種であり、配置は960×960通りから選ばれる。
そこから8手か9手をランダムに指すが、静的交換評価（SEE）で1,000 cpを超える損をしない手を最大8回まで探し、見つからなければ任意の合法手を指す。
[同53〜59行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L53-L59)、[同162〜226行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L162-L226)
その局面を5万ソフトノードで探索し、評価値の絶対値が1,000 cpを超える開始局面は捨てる。
[同517〜527行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L517-L527)
置換表は手番の色ごとに4 MBずつ分け、両者が互いの読みを共有しないようにしている。
[同433〜443行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L433-L443)
勝ちは評価値の絶対値2,500 cp以上が4手続いたとき、引き分けは4 cp以下が12手続いたときに裁定し、詰みや終盤データベース（以下TB）の値が出たときと、TBの局面に入ったときにも終局させる。
[同553〜608行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L553-L608)
生成器は全手を保存し、学習時の既定の除外は16手目未満、駒4個未満、評価値の絶対値2万以上、戦術的な手、王手中の局面である。
除外には、対局結果と評価値の食い違いが閾値を超える局面を除く項目もあるが、既定では無効である。
[dataformat.rs 20〜112行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen/dataformat.rs#L20-L112)

ネットの履歴は、データの作り方を変えた結果を対局で比べた記録を多く含む。
まず、自分のネットで教師を付け直す実験は結果が分かれた。
viri11のデータをviri11で付け直したviri12は+50 Eloだったが、作者は「worried about overfitting」と書いた。
viri17のデータ全体をviri17で付け直したviri18は+23.9 ± 7.2だった。
一方、viri22のデータをviri22で付け直したviri23は、「Hopefully will be less overfitted」と書いたうえで−23.5 ± 9.9となり、viri30で付け直したviri31は−3.0 ± 6.6だった。
[networkhistory.txt 21〜57行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L21-L57)、[同83〜84行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L83-L84)

次に、最新ネットの新しいデータだけで学ぶと弱くなり、累積データに混ぜると強くなるという結果が3回出ている。
viri18の自己対局32万局だけで学んだviri20は−106.2 ± 21.2で、同じ対局を大きな累積データに混ぜたviri21は+7.6 ± 6.5だった。
8,000万局面だけのviri38は−87.6 ± 19.1で、それに既存のデータを足したviri39は+24.8 ± 9.6であり、作者は主な問題がデータ量の減少だったと書いた。
4.5億局面だけのviri52は−16.40 ± 9.40で、既存と合わせた13.5億局面のviri53は+6.95 ± 4.99だった。
[同45〜51行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L45-L51)、[同108〜117行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L108-L117)、[同154〜164行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L154-L164)
Lichess Eliteの局面を除いた自己対局だけのデータで学んだviri29は約−60 Eloであり、作者はLichess Eliteのデータに重要な学習内容があるか、96万局では足りないかのどちらかだと書いた。
[同75〜79行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L75-L79)

一方、同じ量で比べると、新しいデータは古いデータより強い。
少量の古いデータだけで学んだsol-oldと新しいデータだけで学んだsol-newの比較では、新しい方が1手25,000ノードで+13.55 ± 2.27、40秒＋1手0.4秒で+20.83 ± 3.07、120秒＋1手1.2秒で+19.59 ± 2.89だった。
[同1336〜1362行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L1336-L1362)
古いデータを除いて新しいデータの重みを増した微調整のfalkeは、1手25,000ノードで+4.54 ± 2.96、長い持ち時間（以下LTC）で+13.86 ± 5.07だった。
[同891〜908行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L891-L908)
ただし、2024年5月1日より前のデータを除いて25,000ノードのデータを4倍に重み付けしたspigotは−18.62 ± 6.63で負けており、古いデータの除外が常に得になるわけではない。
[同1031〜1039行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L1031-L1039)

自己対局を偏りの検出に使う意図を明記した記録もある。
zhudunは、新しい入力構造のネットexcaliburで1.34億局面を生成して加えたネットであり、作者は生成に当時の採用ネットgemstoneではなく新しい構造のexcaliburを使った理由を「i want to get selfplay to reveal any halfka-induced overfitting to help the next net」と書いた。
結果は短い持ち時間で−0.49 ± 2.23、LTCで+7.67 ± 4.38だった。
[同244〜256行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L244-L256)
また、実戦には遅すぎるが1手あたりの判断が強い大きなネットcolossusが生成したデータを4倍に重み付けしたperseveranceは、1手25,000ノードで+7.24 ± 3.94、LTCで+8.44 ± 3.90だった。
[同774〜790行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L774-L790)

探索値を別の評価で置き換える試みは、いずれも負けた。
TD法（λ=0.05）で付け直したデータで微調整したheliosは、1手25,000ノードで−3.55 ± 3.31、LTCで−4.05 ± 3.09だった。
[同658〜675行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L658-L675)
データの探索値を当時の採用ネットinimicalの静的評価で置き換えたequilibriumは−2.72 ± 2.78と−3.12 ± 2.73、より大きな教師ネットpreceptorの静的評価で置き換えたvoltarineは−2.44 ± 2.88と−3.19 ± 2.85だった。
[同1702〜1745行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/notes/networkhistory.txt#L1702-L1745)
生成器には、ネットの静的評価でデータの評価値を付け直す機能も残っている。
[datagen.rs 1080〜1081行](https://github.com/cosmobobak/viridithas/blob/b407e3eb0b8fc6e251d22664b696cfa870208a7a/src/datagen.rs#L1080-L1081)

作者は2026年2月12日のブログ記事で、lc0の学習を論じる文脈において、固定された教師からの蒸留には天井があり、それを超えるには新しいネットの判断を探索で増幅して古い教師より良いデータを作り、再び蒸留する必要があり、それがまさに自己対局の反復だと述べた。
原文は「Distillation from a fixed oracle has a ceiling」「To surpass that ceiling, you must search-amplify the new network, generating better data than the old oracle could, and distill again」である。
[AlphaZero, RL, and SPSA](https://asteri.sm/files/2026-02-12-az-rl-and-spsa.html)
推論として、この主張は探索が評価関数の誤りを正すことを前提にしており、探索でも正せない種類の誤りには触れていない。

### PlentyChess

PlentyChessは、2024年11月5日のプルリクエスト#279で、lc0のデータで学んだネットを自己生成のデータだけで学んだネットに置き換えた。
説明によれば、PeSTOの評価から始めて約3か月で約105億局面を生成し、そのうち最良の72億局面で学んだ。
できたネット単体は以前のlc0のネットより約30 Elo弱かったが、他の改良と合わせた版は短い持ち時間で+8.72 ± 4.14、40秒＋1手0.4秒で+3.30 ± 2.28、DFRCで+28.95 ± 8.33だった。
[プルリクエスト#279](https://github.com/Yoshie2000/PlentyChess/pull/279)
現行のREADMEは、150億局面以上の自己生成局面で学び、学習の一部に自己蒸留を使うと書く。
[README 5行](https://github.com/Yoshie2000/PlentyChess/blob/8bf271f462b5ae01a3bb8be6cdb30de19e6bdbd4/README.md#L5)
「最良の72億局面」の選び方と自己蒸留の中身は書かれていない。

生成コードはOpenBenchの開始局面生成の取り決めを実装している。
初期局面から8手か9手、定跡を与えた場合は3手か4手のランダムな手を指す。
ランダムな手は、まず駒種をポーン35%、ナイト15%、ビショップ15%、クイーン15%、キング15%、ルーク5%の確率で選び、その駒の合法手から一様に選ぶ。
最後の局面を深さ10かつ100万ノードで探索し、評価値の絶対値が1,000 cp以上なら捨てる。
[datagen.cpp 19〜65行](https://github.com/Yoshie2000/PlentyChess/blob/8bf271f462b5ae01a3bb8be6cdb30de19e6bdbd4/src/datagen.cpp#L19-L65)、[同113〜134行](https://github.com/Yoshie2000/PlentyChess/blob/8bf271f462b5ae01a3bb8be6cdb30de19e6bdbd4/src/datagen.cpp#L113-L134)
推論として、対局はOpenBenchのデータ生成の作業で指されており、1手のノード数と裁定は作業設定に依存してリポジトリには残らない。

### Integral

IntegralのREADMEは、データを数億局の自己対局から作り、多くの対局は定跡UHO_Lichess_4852_v1の局面から3〜4手をランダムに指して始め、大半のデータで5駒のTBを生成時の探索に使うと書く。
この定跡の名前はLichessの対局に由来することを示すが、作り方は本調査では確かめていない。
初代のネットは強いHCEを持つ版4のデータで学んだが、初期のデータはすべて捨て、現在の大半のデータは1手20,000ソフトノードで生成している。
[README 23〜28行](https://github.com/aronpetko/integral/blob/bed1786be517489cc8d924339c7ec58311ac3a0f/README.md#L23-L28)

生成コードの既定のランダムな手数は8〜9手である。
[data_gen.h 22行](https://github.com/aronpetko/integral/blob/bed1786be517489cc8d924339c7ec58311ac3a0f/src/data_gen/data_gen.h#L22)
定跡を与えない場合は、駒種をポーン35、ナイト25、ビショップ25、ルーク5、クイーン5、キング5の比で選び、行き先の升に中央寄りや前方ほど大きい重みを付けて手を選ぶ。
定跡を与えた場合は合法手から一様に選ぶ。
[data_gen.cc 18〜133行](https://github.com/aronpetko/integral/blob/bed1786be517489cc8d924339c7ec58311ac3a0f/src/data_gen/data_gen.cc#L18-L133)
開始局面は深さ10かつ100万ノードで検証し、評価値の絶対値が300 cp以上なら捨てる。
勝ちは評価値が2,500 cp以上の手が5手続いたとき、引き分けは対局開始から200手以上進んだ後に絶対値2 cp以下が8手続いたときに裁定し、反復と駒不足でも引き分けとする。
[同217〜293行](https://github.com/aronpetko/integral/blob/bed1786be517489cc8d924339c7ec58311ac3a0f/src/data_gen/data_gen.cc#L217-L293)
この引き分けの条件が参照する`half_moves` は、50手規則の時計ではなく対局全体の手数であることを、盤の状態の更新で確かめた。
[board.cc 288〜289行](https://github.com/aronpetko/integral/blob/bed1786be517489cc8d924339c7ec58311ac3a0f/src/chess/board.cc#L288-L289)
生成器は全手をbinpack形式で記録しており、学習時の除外と混合比はリポジトリでは確認できなかった。

### Akimbo、Carp

Akimboの現行のREADMEは、1.0.0までのデータはすべて自己生成であり、HCEを持っていた時期に駒価値から始めてデータの生成と調整を反復し、最後のHCEのデータで初代のネットを学んだと書く。
同じREADMEは、計算に数百時間かかって退屈になったので現在はlc0のデータを使うが、それで正式な版は出さないと続ける。
[README 17〜20行](https://github.com/jw1912/akimbo/blob/f7dd7677d6ab9a4a76e3769731084894a7f7ae09/README.md#L17-L20)
1.0.0の生成器は、1手5,000ソフトノード、上限100万ノードと10秒で探索し、初期局面か定跡の局面から一様ランダムに8手か9手を指して始める。
評価値の絶対値が3,000 cpを超えた最初の手で勝ちと裁定し、引き分けは規則によるものだけである。
最善手が捕獲でなく王手中でもない局面だけを生成時に保存する。
[datagen.rs 12〜17行](https://github.com/jw1912/akimbo/blob/3bfea08c8c56c462a28cb1f3b76a442938e8085f/src/datagen.rs#L12-L17)、[同192〜282行](https://github.com/jw1912/akimbo/blob/3bfea08c8c56c462a28cb1f3b76a442938e8085f/src/datagen.rs#L192-L282)

Carpは自己対局のデータだけで学ぶと書き、生成器の注釈はコードの大半をSvartとViridithasから取ったと記す。
[README 57〜63行](https://github.com/dede1751/carp/blob/7d5065bc26dd16002e6581b25abb3decf62ed86c/readme.MD#L57-L63)、[datagen.rs 1〜2行](https://github.com/dede1751/carp/blob/7d5065bc26dd16002e6581b25abb3decf62ed86c/tools/src/datagen.rs#L1-L2)
既定は深さ8の探索で、一様ランダムに8手か9手を指し、深さ10で絶対値1,000 cp以上の開始局面を捨てる。
勝ちは2,000 cp以上が4手、引き分けは5 cp以下が12手で裁定し、王手中、TBの詰みの値、最善手が静かな手でない局面、16手目以前の局面を生成時に除く。
[同83〜232行](https://github.com/dede1751/carp/blob/7d5065bc26dd16002e6581b25abb3decf62ed86c/tools/src/datagen.rs#L83-L232)
初代のネットの作り方は確認できなかった。

### 対象から外したエンジン

Obsidianはlc0のデータで学んだネットを使うとREADMEに書いている。
[README 19行](https://github.com/gab8192/Obsidian/blob/720614a61ed6201037516d555f0bd2f7e5aeae3c/README.md#L19)
Alexandriaも、lc0の公開した学習データで学んだネットを使うと書いている。
[README 29〜30行](https://github.com/PGG106/Alexandria/blob/54d4c206d76146fc88721b74f7ec65e077ab5dcb/README.md#L29-L30)
Berserkは学習器の名前を挙げるが、データの出所を書いていないので対象から外した。
[README 62〜72行](https://github.com/jhonnold/berserk/blob/32628515050b83805bab4afa1026dd2bcaa93f55/README.md#L62-L72)
Akimboの1.0.0より後の開発版も、lc0のデータを使うので対象から外した。

## 生成と学習の共通基盤

### OpenBenchのデータ生成

チェスの多くのエンジンは、対局試験の基盤OpenBenchの「DATAGEN」作業でデータを生成している。
OpenBenchのwikiによれば、エンジンは `genfens N seed S book <定跡>` という命令に応じて開始局面をN個出力し、OpenBenchがその局面から対局を指させて棋譜を回収する。
固定ノードや固定深さの自己対局では同じ開始局面の色を入れ替えても同じ対局になるので、色を入れ替えた対局は既定で指さない。
同じwikiは、異なる対局が生じる例として「adversarial play, or playing against a different branch of the same engine」を挙げており、別の版を相手にした生成も想定している。
[Data Generation Workloads](https://github.com/AndyGrant/OpenBench/wiki/Data-Generation-Workloads)
勝ちと引き分けの裁定、およびTBの使用は、作業ごとの設定として対局管理プログラムへ渡される。
[worker.py 430〜476行](https://github.com/AndyGrant/OpenBench/blob/dd03aaff9e855e8b922f5d365ad5b74a670560f6/Client/worker.py#L430-L476)
既定値は各OpenBenchの設置ごとのエンジン設定にあり、本調査では確認できなかった。

### bulletの文書と例

bulletの文書は、自前でデータを生成する開発者の間で最もよく使われているとしてviriformatのbinpackを勧め、binpackは対局を丸ごと保存するので学習時に除外の関数が必要だと書く。
[docs/3-data.md 14〜20行](https://github.com/jw1912/bullet/blob/579b78b37a694cacd6e9ede4706386c15c34837c/docs/3-data.md#L14-L20)
上級の例は、16手目未満、駒4個未満、戦術的な手、王手中、キャスリング、評価値の絶対値1万以上を除き、評価値と対局結果の食い違いが2,500を超える局面も除く。
さらに、局面を15%の確率で読み飛ばし、盤上の駒数の分布が目標の分布に近づくように採否を確率で決める。
[filter.rs 9〜77行](https://github.com/jw1912/bullet/blob/579b78b37a694cacd6e9ede4706386c15c34837c/examples/advanced/filter.rs#L9-L77)
学習は3段階で、対局結果の重みを0.2で一定、0.2から0.5へ線形に増加、最後に小さな学習率で1.0とする。
[main.rs 193〜220行](https://github.com/jw1912/bullet/blob/579b78b37a694cacd6e9ede4706386c15c34837c/examples/advanced/main.rs#L193-L220)
これは学習器の配布例であり、特定のエンジンの採用設定を示すものではない。

## Fairy-Stockfishの変種用NNUE

### 手引きが勧める手順

Fairy-Stockfishは既定ではHCEで評価し、変種ごとのNNUEは任意で読み込む。
NNUEは、Stockfish公式の学習器nnue-pytorchを変種向けに一般化したvariant-nnue-pytorchで学び、教師はFairy-Stockfishから派生したvariant-nnue-toolsで生成する。
[Fairy-Stockfish wiki「NNUE」](https://github.com/fairy-stockfish/Fairy-Stockfish/wiki/NNUE)

学習器のwikiの教師生成の頁は、まず `Use NNUE` を `false` にしてHCEで生成する例を示す。
例の命令は `generate_training_data depth 2 count 10000000 random_multi_pv 4 random_multi_pv_diff 100 random_move_count 8 random_move_max_ply 20 write_min_ply 5 eval_limit 10000 set_recommended_uci_options data_format bin` であり、1回で1,000万局面を深さ2で作る。
既存のネットで生成する場合は `Use NNUE` を `pure` にして評価関数ファイルを指定し、その変種の現行最良のネットが一覧にあれば、それを生成に使うことを勧める。
同じ頁は、局面数と深さが結果の強さを決める主な要因であり、深いほど良いが時間がかかり、深さ4〜5でかなり良い結果になると書く。
分岐の少ない変種では `random_multi_pv_diff` を大きくして局面の多様性を増やすよう勧め、定跡ファイルで開始局面を与えることもできるとする。
[Training data generation](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Training-data-generation)
FAQは、生成が既出の局面を除くので、小さな変種や強制手の多い変種では新しい局面が尽きて生成が止まることがあり、そのときは `random_multi_pv_diff` を上げて再開するよう書く。
[FAQ](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/FAQ)

学習は、既存のネットを変換して `--resume-from-model` で続きから学ぶことも、乱数から学ぶこともでき、手引きはどちらかを推奨してはいない。
1エポックは2,000万局面であり、2億局面なら10エポック程度を目安にする。
[NNUE training](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/NNUE-training)
学習したネットは、variantfishtestなどで前の最良のネットと対局させて強さを確かめ、良ければ共有のフォルダへ登録し、一覧を更新する。
[Step-by-Step Guide](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Step-by-Step-Guide)
世代をまたいで古いデータを混ぜるかどうかについて、手引きは何も書いていない。

偏りに関わる注意としては、FAQの1項目がある。
3checkのような王手回数で勝つ変種では、王手回数をNNUEの入力に持たず、NNUEの評価の上に追加の項を足しているので、ネットだけの評価は不正確であり、教師の生成に `pure` を使ってはならないと書く。
[FAQ](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/FAQ)
推論として、これは、ネットが表現できない要素を持つ変種では、ネットだけに教師を作らせると誤りがそのまま教師に入ることへの警告と読める。

### 生成器の既定値とその意味

生成器 `generate_training_data` の既定値と動作を、上流の最新コミットのソースで確かめた。
文書と食い違う箇所があるので、以下はソースの行を典拠にする。

1手の探索は既定で深さ3であり、`nodes` を指定すると深さとノード数のどちらか先に達した方で止まる。
[training_data_generator.cpp 45〜50行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L45-L50)、[同320行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L320)

ランダムな手は、`random_move_min_ply` 1から `random_move_max_ply` 24までの手数のうち、`random_move_count` 5個の手数を重複なく一様に選び、その手数で探索の最善手の代わりに指す。
[同59〜61行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L59-L61)、[同536〜570行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L536-L570)
手数は両者の手を通して数えるので、24手目までの両者の手のうち5手がランダムになる。
既定の `random_multi_pv` 5では、その局面で上位5手を求める探索を行い、最善手との評価値の差が `random_multi_pv_diff` の100以内の候補から一様に1手を選ぶ。この差は探索の内部の評価値（`rm.score`）で比べるので、単位はcpではなくエンジン内部の単位である。
`random_multi_pv` を0にすると、合法手から一様に選ぶ。
多候補探索の深さは、生成時の探索の深さより浅くならない。
[同76〜78行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L76-L78)、[同115行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L115)、[同573〜662行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L573-L662)
したがって既定のランダム化は、序盤に限った「ほぼ同等の手からの抽選」であり、チェスのエンジンの多くが開始局面の作成で行う一様なランダム手とは性格が異なる。

`eval_limit` の既定は3,000であり、上限は2手詰みの値に制限される。
[同53行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L53)、[同109行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L109)
文書は、これを超える評価値の局面は書き出さず対局を打ち切ると説明するが、ソースの動作は異なる。
対局ごとに10分の8の確率で投了ありとし、投了ありの対局では探索値の絶対値が `eval_limit` 以上の手が4手続いた時点で終局させ、投了なしの対局では既知の勝ちの値に達するまで指し続ける。
[同291行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L291)、[同331〜344行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L331-L344)
書き出しの条件には評価値の上限がないので、投了ありの対局でも上限を超えた局面が終局までの最大3手分は記録され、投了なしの対局ではさらに多く記録される。
引き分けは、80手以降に探索値0が8手続いた場合と、双方の駒不足の場合に裁定し、`write_max_ply` 400手に達した対局も引き分けとして扱う。
数取りの規則を持つ変種では、2026年7月21日の変更で、これらの裁定に数取りの結果を使うようになった。
[同449〜531行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L449-L531)、[b8d5488](https://github.com/fairy-stockfish/variant-nnue-tools/commit/b8d5488d2bc95a8e9c31a0eeff8a864f7dbbccb6)

局面を書き出す条件は、5手目以降であること、既出の局面でないこと、王手中でないこと、静止探索と静的評価の差が `eval_diff_limit` 以下であること、最善手が捕獲でないことである。
[同363〜369行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L363-L369)
既出の判定は、生成全体で共有する6,700万項目余りの表に局面の鍵を記録し、同じ鍵が残っていれば除く近似である。
表の大きさは `64 * 1024 * 1024` 項目である。
[同120行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L120)、[同420〜437行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L420-L437)
教師値はその深さの探索値を手番側から見た値であり、対局結果は対局の終了後に各局面へ付ける。
[同382行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L382)、[同692行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L692)

これらの既定値は、2024年9月15日のコミットで「reasonable values」に改められた。
このとき `eval_diff_limit` は500から64000、`random_multi_pv` は0から5、`random_multi_pv_diff` は32000から100、`write_min_ply` は16から5、`filter_captures` は無効から有効、`count` は80億から1億に変わった。
[9c5c2be](https://github.com/fairy-stockfish/variant-nnue-tools/commit/9c5c2beb7813a3ae3e78f1b7d16b9554a5c9b2ee)
`eval_diff_limit` が64000になったので、静止探索と静的評価の差による静かな局面の選別は事実上無効になり、代わりに最善手が捕獲の局面を除く選別が既定になった。
学習器のwikiの「eval_diff_limit（default: 500）」という記述は、この変更より前のものである。
生成器の文書 `docs/generate_training_data.md` は `filter_captures`、`filter_checks`、`filter_promotions`、`eval_diff_limit` を載せていない。
また、文書は深さの範囲を `mindepth` と `maxdepth` と書くがソースが受け付けるのは `min_depth` と `max_depth` であり、文書の `adjudicate_draws_by_insufficient_mating_material` もソースでは `adjudicate_draws_by_insufficient_material` である。
[同780〜782行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L780-L782)、[同822行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator.cpp#L822)、[generate_training_data.md](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/docs/generate_training_data.md)

別の生成器 `generate_training_data_nonpv` は、1手5,000〜15,000ノードの自己対局で探索した局面のうち1%を保存し、それを深さ3で付け直す。
対局の手順上の局面ではなく探索木の中の局面を集める方式であり、既定では王手中や最善手が捕獲の局面の除外は無効である。
[training_data_generator_nonpv.cpp 43〜72行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/tools/training_data_generator_nonpv.cpp#L43-L72)、[generate_training_data_nonpv.md](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/docs/generate_training_data_nonpv.md)

### 学習器の初期化と既定値

学習器は、NNUEの駒位置表に相当する出力を、変種の設定ファイルに書かれた駒価値で初期化する。
各駒の価値を盤上のすべての升に同じ値で置き、600分の1に縮めて初期値とする。
[model.py 165〜174行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/b15df38a9aae8ab9b40b2378020b3099c7c5d179/model.py#L165-L174)、[halfka_v2.py 41〜56行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/b15df38a9aae8ab9b40b2378020b3099c7c5d179/halfka_v2.py#L41-L56)
設定ファイルの駒価値は、生成器の `trainer_config` 命令がFairy-Stockfishの中盤の駒価値から書き出す。
[uci.cpp 433〜439行](https://github.com/fairy-stockfish/variant-nnue-tools/blob/445b53c419d03d9167a786140a4d4bbe12ee7426/src/uci.cpp#L433-L439)
したがって初代のネットは、HCEで作った教師と、HCEの駒価値で初期化した駒位置表の2つを種にする。

学習器の `--lambda` の既定は1.0であり、探索値だけを教師にする。
`--random-fen-skipping` の既定は3で、各局面を平均して4つに1つだけ使う。
[train.py 29〜38行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/b15df38a9aae8ab9b40b2378020b3099c7c5d179/train.py#L29-L38)
読み込み時の「smart fen skipping」は既定で有効と表示されるが、変種版のデータ読み込みでは選別の関数が常に偽を返すので、実際には何も除かない。
局面の選別は生成器の側で行う設計になっている。
[training_data_loader.cpp 588〜613行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/b15df38a9aae8ab9b40b2378020b3099c7c5d179/training_data_loader.cpp#L588-L613)

### 中将棋と大きな盤の公開例

Fairy-Stockfishの盤の大きさは、大きな盤用の版でも筋がAからLの12、段が1から10の10までである。
[types.h 546〜564行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/types.h#L546-L564)
学習器の技術解説も、学習データの形式を設計する前提として、Fairy-Stockfishの盤を最大120升、すなわち12×10升と説明している。
[Technical details](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Technical-details)
FAQは、NNUEで学べる変種の駒種を15種まで、盤上の駒を40〜60個までとし、それを超える駒数には大きなデータ形式でのビルドが必要だとする。
[FAQ](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/FAQ)
中将棋は12×12升の盤で、初期配置の21種と成駒を合わせて29種の駒を使い、双方で92枚の駒を持つので、盤の大きさと駒種の両方でこの範囲を超える。

公式サイトの公開ネットの一覧には、中将棋も12×12升以上の盤の変種も載っていない。
[nnue.markdown](https://github.com/fairy-stockfish/fairy-stockfish.github.io/blob/a07784b0631c6f9158e651747242a587ba152cf7/nnue.markdown)
一覧のうち筋の数が最も多いのは、12×8升のcourierと、12×10升の格子を使うomicronである。
omicronの開始局面は外周を四隅以外の壁で塞いでおり、盤上で使える升は10×8升と四隅の4升であり、12×10升の全体を使うわけではない。
[variant.cpp 1508〜1525行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/variant.cpp#L1508-L1525)、[同1604〜1615行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/variant.cpp#L1604-L1615)
10×10升の全体を使う変種ではgrandとshakoのネットがあり、将棋系では9×9升の将棋とsho shogiが最大である。
[同1536〜1560行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/variant.cpp#L1536-L1560)、[同1669〜1690行](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/2e591089558a5afa72ab5a22192208e71848a30c/src/variant.cpp#L1669-L1690)
GitHubのリポジトリ検索でも、2026年10月25日の時点で「chushogi」「chu shogi nnue」「chushogi nnue」に当たるNNUEの公開例はなく、見つかったのは合法手生成器、乱数で指すエンジン、駒の印刷用データ、およびminase自身だった。
lishogiが中将棋の解析や対局にどのエンジンを使っているかは確認できなかった。

## minaseへの含意（推論）

この節はすべて推論である。

第1に、自己対局の対局結果は、双方が同じ癖を持つ限り、その癖を罰しない。
minaseでは、自己対局の教師で学んだ評価関数が、麒麟を自陣の右端へ運ぶ癖を持つようになった（[分析の記録](../../measurements/adhoc/kirin-right-edge-bias.md)）。
麒麟を右端へ運ぶ癖を先手も後手も持てば、相手はその癖を咎める手を指さないので、癖が悪手であっても勝敗に表れにくく、λを下げて対局結果を混ぜても教師はその癖を正しにくい。
minaseが世代1で教師に対局結果を25%混ぜていても（λ=0.75）癖が生じたことは、この見方と矛盾しない。
チェスの先例で、自己対局の外から局面や相手を持ち込む仕組みは3つある。
ViridithasはLichess Eliteの局面を自分で付け直して混ぜており、それを抜いたデータで学んだネットは約60 Elo弱かったが、作者はデータ量の不足という別の説明も挙げている。
Integralは、名前からLichessの対局に由来すると読める定跡の局面から自己対局を始める。
OpenBenchは同じエンジンの別の版を相手にした生成を想定している。
minaseでは、lishogiの約2万3千局の人間の棋譜から途中局面を取り出して開始局面にする方法と、癖を持たない版、たとえば世代0の評価関数を持つ版を相手にして生成する方法が、これらに相当する。

第2に、乱数のネットから始めた先例は、初期は対局結果の重みを大きくし、仕上げにも対局結果だけで微調整する。
Saplingは初代を対局結果だけで学び、Astraのv14とbulletの例は最後の段階を対局結果だけで学ぶ。
minaseの現行は探索値だけを教師にして（λ=1.0）学んでおり、仕上げの段階だけ対局結果の重みを上げる比較は、先例に沿った費用の少ない実験になる。
ただし第1の理由により、対局結果の重みは自己対局で共有された癖を正す手段にはならないので、癖への効果は別に測る必要がある。

第3に、評価値と対局結果の食い違いが大きい局面を除く選別がViridithasとbulletの例にある。
麒麟が右端にある局面の教師について探索値と最終結果の食い違いを集計すれば、その局面で評価値が勝敗の予測を誤っているかは少ない費用で確かめられる。
ただし、食い違いがなくても、不利な配置を正しく低く評価したうえで負けている場合があるので、癖の有害性はこの集計では判断できず、別の比較が要る。

第4に、乱数から始めたAstra、Zevra 2、Saplingは、前の世代の評価関数で新しいネットを初期化していない。
Zevra 2は毎世代、乱数から学び直す。
minaseはNNUEの線形経路を前の世代の駒位置表（PST）で初期化しているので、PSTがすでに癖を持っていれば、初期値が癖を次の世代へ運ぶ経路が1つ増える。
minaseは、非線形部分だけでも右端に約+110 cpの選好を学んだという観測から、癖を次の世代へ伝える主な経路は教師データだと判断しているが、線形経路を世代0のPSTか駒価値だけで初期化した比較を行えば、初期値の寄与も切り分けられる。

第5に、生成の規模の目安として、Zevra 2は乱数から始めてHCE版の強さに達するまで7〜8回の反復を要し、乱数から始めた3つのエンジンは最終的に11〜21世代と10億〜120億局面を積んでいる。
1世代の新規局面は、Astraのv4からv6で1.25億〜3.5億、Saplingの5番と6番で5,500万〜1.1億局面であり、minaseの1世代約8,300万局面はこの範囲にある。
チェスの1手5,000〜30,000ノードに対し、minaseの20,000ノードは数値としては同じ範囲だが、中将棋は合法手がはるかに多いので、同じノード数で読める深さはずっと浅いと考えられる。
データの混ぜ方については、Viridithasの記録が、最新ネットの小さな新データだけで学ぶと負けること、同じ量なら新しいデータが古いデータより強いことの両方を示している。
`docs/research/nnue-architecture-discussion.md` が初期設定とした抽出比率、すなわち新規50%、直前の世代25%、それ以前25%は、この2つの結果のどちらとも矛盾しない。

第6に、Fairy-Stockfishの既定のランダム化は、24手目までの5手を、最善手との差がエンジン内部の単位で100以内の上位5手から抽選する。
minaseは対局途中のランダムな手を使っていないが、この方式は手の質を大きく落とさずに序盤の分岐を増やすので、開始局面を作る8〜16手のランダム化に加える候補になる。
麒麟の配置は序盤の数十手で決まると考えられるので、抽選の範囲をチェスの24手より長く取る必要があるかもしれない。

最後に、特定の駒の配置癖を世代ごとに追跡した先例は見つからなかった。
minaseが記録している「右端2筋にいた手数の割合」は先例にない監視指標であり、世代の採否に棋力の測定と並べて使う価値がある。

## 未確認の事項

- Stormphraxの生成コードと手順は読んでおらず、項目2から10は未確認である。
- PZChessBotは、乱数から学んだというREADMEの記述以外を確認していない。
- Astraの1手のノード数、勝敗の裁定、対局結果の重みの値、各コミット文のElo差の比較相手、および各世代を乱数から学んだか続きから学んだかは確認できなかった。
- Zevra 2の対局結果と探索値の混合比、および各世代の学習が累積データを使うのかその世代のデータだけを使うのかは確認できなかった。
- Saplingの各ネットを乱数から学んだか前のネットから続けたかは確認できなかった。
- PlentyChessの「最良の72億局面」の選び方、自己蒸留の中身、1手のノード数、裁定は確認できなかった。
- Integralの学習時の除外と混合比は確認できなかった。
- Akimboの1.0.0までの混合比と、HCEの調整を何回反復したかは確認できなかった。
- Carpの初代のネットの作り方は確認できなかった。
- OpenBenchの各設置でデータ生成に使う裁定の既定値は確認できなかった。
- Fairy-Stockfishの手引きが、世代をまたいで古いデータを混ぜることを想定しているかは確認できなかった。
- lishogiが中将棋に使うエンジンと、非公開の中将棋用NNUEの有無は確認できなかった。
- Viridithasの作者のブログ記事は、lc0の文脈で書かれた一般論であり、NNUEの自己対局で同じ主張を実測した記録は見つからなかった。
