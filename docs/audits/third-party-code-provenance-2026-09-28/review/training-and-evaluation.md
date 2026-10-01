# 学習と評価関数の対照レビュー記録

担当範囲は、minase 基準コミット a05478a の tools/train/、src/training/、src/eval/（handcrafted.rs を除く）、src/bin/selfplay_gen/、src/datagen/、および削除済みの学習系ファイルである。
削除済みファイルは、`git log --all` で内容を持つ最新の版を探し、`<work-dir>/r5src/` へ書き出して読んだ（NNUE系は nnue-gen1 ブランチの版が最新）。
リポジトリはすべて読むだけで、変更していない。

## 対照の入手

- variant-nnue-tools はコーパスになかったため、`git clone --filter=blob:none https://github.com/fairy-stockfish/variant-nnue-tools` で `<corpus-dir>/fairy-stockfish_variant-nnue-tools` へ取得した（HEAD 445b53c、2026-09-25、Copying.txt は GPL-3.0、Stockfish 派生）。
- YaneuraOu の learner.cpp は、削除コミット 0a6dd2cb の親から r5src/yo_learner.cpp へ書き出して読んだ。
- 日本語コメントの一致は、担当範囲の全ファイル（現存51本と削除版）から和文の連なりを抜き出し、YaneuraOu（HEAD と learner.cpp@0a6dd2cb^）、DeepLearningShogi、tatara、elmo_for_learn、apery、apery_rust、rshogi、Gikou、YaneuraOu-ScriptCollection、cpp_animal_shogi の1,520ファイルと照合した（スクリプトは review/scripts/jp_ngram.py、review/scripts/jp_ngram10.py）。
  14字窓では一致0件だった。
  10字窓の一致は「一致しなければならない。」「差分更新が完全再計算と一致することを」「視点のアキュムレータ」のような定型句の断片だけで、文単位の一致はなかった。

## 判定1　NNUE（削除済み）

- minase 側: e0b8314:src/eval/nnue.rs:1-497（本体）、d67dc92:tools/train/nnue_net.py:1-317、82aa0e6:tools/train/train_nnue.py:1-590。導入は 05362b1（2026-08-26）、ツリーからの削除は 9dd535f（2026-08-26）で、その後 nnue-gen1 ブランチで改修された。
- 対照: official-stockfish/nnue-pytorch@cd6192a:model.py（2021年1月版、L1=256、L2=32、L3=32）、同 HEAD aa2fff2:model/quantize.py:60-120、同 docs/nnue.md:981, 2171-2235（GPL-3.0-only）。Stockfish の nnue/ と YaneuraOu の eval/nnue は構成の比較にとどめた（GPL-3.0）。
- 設計書: 導入時点の docs/plans/evaluation.md は nnue-pytorch wiki、Stockfish wiki、Fairy-Stockfish「About NNUE」、やねうら王 wiki「評価関数の学習」、Leorik のリリースを参考資料に挙げる。

一致点と、その性質は次のとおりである。

1. 層の構成 256×2→32→32→1 と、各層の [0, 1] での clipped ReLU は、Stockfish 12 と nnue-pytorch 初期版の構成と同じである。ただし、この構成は公開された代表的な構成であり、設計書も第1層の拡幅を次の候補として挙げている。弱い一致にとどまる。
2. 隠れ層の重みの訓練時の上限 ±1.98（train_nnue.py:37、設計書の量子化表）は、nnue-pytorch の docs/nnue.md:981 が説明する 127/64=1.984 に由来する。起案稿（c4081f5、2026-08-25）は Stockfish と同じ i8 重み・尺度64・`scale = round(K × 65,536 / (127 × 64))` を採っており、実装時に i16 重み・尺度4,096・右シフト12へ変えた後も 1.98 だけが残った（Rust 側の上限は 8,192＝実数±2.0）。公開文書の量子化方式を設計段階で採った痕跡であり、コードの複製ではない。
3. 第2層以降のバイアス尺度を活性の尺度×重みの尺度とする点は、整数推論で積和とバイアスの尺度を揃えるために必須の一致である。

反証（同じ機能で異なる選択）は次のとおりである。

- 入力は王の条件付けを持たないP型（駒状態47×升144×相対陣営2＋先獅子144＝13,680）であり、HalfKP/HalfKAではない。
- 量子化は第1層 i16・尺度127、第2層以降 i16・尺度4,096・右シフト12であり、Stockfish の i8・尺度64・右シフト6と異なる。
- センチポーン換算は学習時のKから `scale = round(K × 65,536 / (127 × 4,096))` を作る固定小数点式であり、Stockfish の FV_SCALE=16 や nnue-pytorch の nnue2score=600 を使わない。
- 学習PSTで初期化した残差線形項（1/8センチポーン単位、`/ self.k` でロジットへ換算）を持つ。Stockfish の PSQT 出力とは初期化元、単位、加算の式が異なる。
- ファイル形式 MNUE は、識別子、版、特徴集合、層幅4個、K（f32）、規則セット名32バイト、本体のSHA-256からなる96バイトのヘッダを持ち、Stockfish の版ハッシュと説明文字列の形式と異なる。
- 学習は nn.EmbeddingBag（padding_idx）、BCE with logits、AdamW（残差線形項だけ重み減衰0）、CosineAnnealingLR、世代別K、50%の左右鏡映拡張を使う。nnue-pytorch 初期版は nn.Linear、600/361 の定数を使うエントロピー差の損失、Ranger、StepLR（75エポック、γ=0.3）である。第1層の初期化 N(0, 0.05) の根拠コメントも minase 独自の計算による。
- 推論は Rust の単純なスカラーループで、Stockfish の AffineTransform、ClippedReLU、SIMD 分割、dirty piece の構造を持たない。差分更新は minase の Undo（獅子の2枚取り、先獅子特徴）に基づく。

Stockfish と YaneuraOu の実物との照合結果は次のとおりである。

- Stockfish@sf_12:src/nnue/nnue_common.h:72-76 は `kVersion = 0x7AF32F16`、`FV_SCALE = 16`、`kWeightScaleBits = 6` を定める。同 layers/affine_transform.h:254-255 は隠れ層の重みを i8、同 nnue_feature_transformer.h:368-369 は第1層の重みとバイアスを i16（アキュムレータも i16）とし、:165-167 で `clamp(sum, 0, 127)` を取る。同 evaluate_nnue.cpp:93-112 のヘッダは版、ハッシュ値、アーキテクチャ文字列であり、:138 で `output / FV_SCALE` を返す。minase は第1層バイアスとアキュムレータが i32、隠れ層の重みが i16、右シフト12、Kから作る換算、MNUE ヘッダであり、共通するのは0〜127の切り詰めだけである（ClippedReLU の定義上の必須の一致）。
- YaneuraOu@v7.50-wcsc32:source/eval/nnue/nnue_common.h:51-57 は Stockfish と同じ定数（FV_SCALE はエンジンオプション）を持つ。同 architectures/k-p_256x2-32-32.h は K と P の特徴に 256×2-32-32 を重ねる。同 features/p.h は「特徴量P：玉以外の駒のBonaPiece」と定義する。minase の「P型」（王の条件付けのない駒×升の特徴）はこの呼称と重なるが、minase の定義は王駒も含む中将棋の駒状態47種で、K特徴を併用しない。呼称の一致だけで、コードの対応はない。
- YaneuraOu@0a6dd2cb^:source/learn/learner.cpp:1579, 1982 は `mirror_percentage` の確率で左右反転して学習する。minase の50%の鏡映拡張は同じ手法だが、比率は固定で、局面ではなく特徴番号の側で反転を選ぶ。

判定は **2（標準的な方式または仕様で説明可能）** とする。1.98 の残存は nnue-pytorch の公開文書に従って設計した痕跡として記録するが、実装固有のコード一致はない。
ライセンスは nnue-pytorch が GPL-3.0-only、Stockfish が GPL-3.0-or-later、YaneuraOu が GPL-3.0-only である。

## 判定2　学習PSTの学習器（tools/train/pst/train_pst.py、features.py、mnsd.py、taper.py、taper_report.py、pst_workflow.py）

- minase 側: a05478a:tools/train/pst/train_pst.py:1-1030 ほか。学習器の導入は 588363a（2026-08-26）、補間は b687022（2026-09-07）。
- 対照: YaneuraOu@0a6dd2cb^:source/learn/learner.cpp:1007-1060（winning_percentage は sigmoid(value/600)、calc_grad）、elmo_for_learn（GPL-3.0-or-later）、texel、Caissa（MIT）。
- 教師値 `λ·sigmoid(v/K) + (1−λ)·result` と BCE は elmo 式の混合で、YaneuraOu@0a6dd2cb^:source/learn/learner.cpp:1094-1141（`m = (1.0 - lambda) * t + lambda * p`、ELMO_LAMBDA=0.33、ELMO_LAMBDA_LIMIT=32000、勝率は sigmoid(v/600)）と nnue-pytorch の文書も同じ形を示す。必須の一致である。minase の既定 λ=0.75、分類別のK、λの上限切替なしは異なる選択である。
- elmo_for_learn@f825ff0:src/learner.hpp:278-291, 650-672 は Apery 由来の Bonanza 法（PVの末端評価の比較）の学習器であり、λ混合の実装を含まない。minase に対応する部分はない。
- texel@b1883f8:app/texelutil/chesstool.cpp:947-1035（localOptimize）はパラメータの局所探索であり、Kの黄金分割探索に当たるコードは見つからなかった。Caissa@ee73607 の学習器（src/utils/CudaNetworkTrainer.cpp ほか）は CUDA による NNUE 学習で、minase に対応する部品はない。
- mnsd.py:165-175 の `hash64` は SplitMix64 の定数（0x9E3779B97F4A7C15、0xBF58476D1CE4E5B9、0x94D049BB133111EB）を使う。公有の標準アルゴリズムである。
- 反証: Kは固定の600ではなく、教師の分類ごとに区間50〜4,000の黄金分割探索で推定する。補間係数は中将棋固有の盤上総駒数 `q = min(90, max(0, N − 2))` による。MNPT 形式（80バイトヘッダ、1/8センチポーン i16、探索用駒価値47個）と MNSD 形式（144バイトの盤面をそのまま持つ160バイトのレコード）は、PackedSfenValue（40バイト、Huffman 符号の局面）とも bullet/tatara の形式とも異なる。識別性の偏差平方和、除去罰則も独自である。

判定は **2**。

## 判定3　先読み教師値（tools/train/pst/lookahead.py、lookahead_diag.py）

- minase 側: a05478a:tools/train/pst/lookahead.py:1-78。導入は e783dc8（2026-09-22）。設計書 lookahead-teacher.md は KnightCap の TD(λ) を着想元、nnue-pytorch docs を比較対象として挙げる。
- 対照: KataGo@d91ea85:cpp/dataio/trainingwrite.cpp:411-430（fillValueTDTargets、MIT）、cpp/program/play.cpp:1345-1353。
- 反証: KataGo は現局面と終局結果を含む無限地平の指数移動平均（nowFactor は盤面積から計算）で、白視点の値を使う。minase は現局面を除く t+1〜t+40 手の有限窓、手数の偶奇による符号反転、記録された手数だけでの正規化、記録がない場合の退避を持つ。

判定は **2**。

## 判定4　自己対局の教師生成（src/bin/selfplay_gen/、src/datagen/）

- minase 側: a05478a:src/bin/selfplay_gen/play.rs:214-252（plan_injections_with_rng）、opening.rs:120-150。ランダム着手の注入の導入は c77f2ef（2026-08-27）。設計書 evaluation-gen1.md は Stockfish 旧 gensfen の random_move_count/minply/maxply と issue #3762 を挙げる。
- 対照: variant-nnue-tools@445b53c:src/tools/training_data_generator.cpp:536-570（generate_random_move_flags、GPL-3.0）、YaneuraOu@0a6dd2cb^:source/learn/learner.cpp:435-461（同じ部分 Fisher-Yates）。
- 一致点: 注入する手数を部分 Fisher-Yates で選ぶこと。k個を非復元抽出する標準的な方法であり、必須の一致に近い。
- 反証: minase は注入回数を0〜上限から一様に選び、窓は開始局面から80手、記録は最後の注入の次の手から始める。gensfen は minply〜maxply（既定1〜24）に固定回数5回を入れ、write_minply 以降を書く。除外条件は、minase が詰みの帯、捕獲または成りの最善手、対局内の再出現、gensfen が eval_limit、捕獲の最善手、ハッシュ表による既出である。コードの分割、変数名、コメントに対応はない。

判定は **2**。

## 判定5　付け直し（src/training/rescore.rs、src/bin/selfplay_gen/rescore.rs）と学習データ形式（src/training/records.rs、provenance.rs）

- minase 側: a05478a:src/training/rescore.rs:1-667、records.rs:1-961。付け直しの導入は b704452（2026-09-22）。strength-stage9.md は、行対応の別ファイルとする形を tatara の方式に倣ったと明記し、variant-nnue-tools の `transform rescore` も挙げる。
- 対照: tatara@536beb6:crates/nnue-train/src/rescore.rs:1-120, 541-586（MIT）、variant-nnue-tools@445b53c:src/tools/transform.cpp（GPL-3.0）。
- 一致点: 元データを書き換えず、行 i が元の記録 i に対応する別ファイルへ新しい値を書く、という設計上の考え方。
- 反証: tatara の sidecar は i16 の値だけを並べる GPU の静的評価による付け直しであり、minase の MNRS は240バイトのヘッダ（元データと対象一覧のSHA-256、ノード数、規則セット、置換表容量、生成コミット、実行バイナリのSHA-256）と16バイトの記録（状態、値、捕獲・成りの旗、ノード数、深さ）を持つ CPU 探索の付け直しである。コードの構造は共有していない。

判定は **2**。

## 判定6　学習PSTの整数評価（src/eval/pst/）

- minase 側: a05478a:src/eval/pst/mod.rs:1-184、accumulator.rs:1-145、features.rs、format.rs。
- 対照: YaneuraOu eval/nnue のアキュムレータ、Stockfish nnue_accumulator。
- 両視点の累算値を持ち差分更新する構造は NNUE 以降の標準である。補間式（分子 `q·s_mg + (90−q)·s_eg`、除数720）、先獅子特徴、Undo の獅子2枚取りの扱いは中将棋固有である。

判定は **2**。

## 判定7　その他の削除済み評価コード

- e80cbf4 で削除した src/eval/king_features.rs（ad15b4f 版、導入 327bfe0）: 王の前後±2筋について、自駒の歩兵・仲人がない筋を、敵の歩兵の有無と縦の走り駒の枚数で分けて距離別に数える24列。Stockfish 旧評価の shelter/storm の表（王の筋±1、歩の段で引く表）とは特徴の定義が異なる。判定は **2**。
- tools/train/pst/convert_mnpt.py、shelter_contribution.py: MNPT の列追加と寄与集計だけの小さな道具。判定は **2**。
- fm-eval ブランチの src/eval/fm.rs、tools/train/pst/train_fm.py: Factorization Machine の標準恒等式 `0.5·((Σv)² − Σv²)` を使う。量子化と符号・指数の形式は独自である。判定は **2**。
- strength-stage9-table ブランチの src/eval/attack_table.rs（削除ではなく未統合。参考として確認）: 設計書が先行実装として YaneuraOu の long_effect.cpp を挙げる。升ごとの利き数と、届く長い利きの方向ビットを持つ点は同じ考え方だが、YaneuraOu は先後を1語にまとめた WordBoard と、打つ・取る・取らない・巻き戻しの着手種別ごとのテンプレート関数を持つ。minase は色ごとの u8 配列、駒の除去と配置への分解、ply ごとの複写（巻き戻しなし）、短い利きの重なりの相殺を使う。判定は **2**。

## 判定8　診断器（pst_diagnostics.py、human_signal_diag.py、depth_sensitivity_diag.py、king_features_diag.py、taper_report.py）

- 最大剰余法、ニュートン法によるロジスティック回帰、対局単位の交差検証、ブートストラップなど、標準的な統計手続きを独自の入出力の上に書いたものである。対照に対応するコードはない。判定は **2**。

## 担当範囲に対応する部品がない対照

- bullet@c004ebf:examples/simple.rs:20-37 は QA=255、QB=64、SCALE=400 の量子化を使い、minase の NNUE（127、4,096、Kから作る換算）と一致しない。minase は bullet を使わず PyTorch で学習した。
- tatara@536beb6:bins/nnue_train/src/training.rs:73-110 は FV_SCALE を学習時の score_scale から導く YaneuraOu 互換の書き出しであり、minase の MNUE の換算式とは異なる。
- DeepLearningShogi@559d636:dlshogi/（train.py、data_loader.py、network/）は方策と価値のCNNの学習器であり、担当範囲に対応する部品はない（AlphaZero 型の設計書は担当外で、実装は担当範囲にない）。ファイル構成を確認しただけで、中身の照合はしていない。
- lczero-training@7c5d756 は **4（未確認）** とする。担当範囲に方策・価値ネットの学習器がなく、照合の対象になる部品がないため開いていない。

## 判定の表

| モジュール | 判定 | 理由 |
|---|---|---|
| 削除済みNNUE（nnue.rs、nnue_net.py、train_nnue.py） | 2 | 構成256×2-32-32と上限±1.98は nnue-pytorch の公開文書の方式に由来するが、量子化の尺度、換算式、形式、損失、最適化器、コードの構造はいずれも異なる |
| tools/train/pst の学習器（train_pst.py、features.py、mnsd.py） | 2 | elmo 式の教師混合とBCEは必須の一致。Kの推定、形式、特徴は独自 |
| taper.py、taper_report.py | 2 | 中将棋固有の駒数補間と識別性の診断 |
| lookahead.py、lookahead_diag.py | 2 | KataGo の TD 値とは窓、符号、正規化が異なる独自式 |
| src/bin/selfplay_gen、src/datagen | 2 | 部分 Fisher-Yates は標準手法。回数、窓、記録開始、除外条件は gensfen と異なる |
| src/training（records、rescore、provenance） | 2 | tatara の行対応の考え方を明記して採用。形式とコードは独自 |
| src/eval/pst | 2 | 標準的な両視点累算。補間と先獅子は中将棋固有 |
| king_features.rs、convert_mnpt.py、shelter_contribution.py（削除済み） | 2 | 特徴の定義は Stockfish の shelter 表と異なる |
| fm.rs、train_fm.py（未統合・削除） | 2 | FM の標準恒等式 |
| attack_table.rs（未統合、参考） | 2 | YaneuraOu long_effect と同じ考え方で、データ構造と更新の分割は異なる |
| 診断器群 | 2 | 標準統計の独自実装 |

和文コメントの照合（14字窓で一致0件）でも、YaneuraOu、tatara、DeepLearningShogi、elmo 系との文単位の一致は見つからなかった。
