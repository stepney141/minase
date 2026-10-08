# fearless_simdによる安全なSIMDの適用可能性

**fearless_simd**は、Linebenderが2026年9月に1.0を公開したRustのSIMDライブラリであり、利用者側のコードに`unsafe`を書かせずに、実行時に検出した命令集合（SSE2、SSE4.2、AVX2、AVX-512、NEON）へ処理を振り分ける。
minaseのワークスペースは`unsafe_code = "forbid"`を掲げているので、このライブラリはその方針と両立する明示的SIMDの選択肢になる。
本書は、fearless_simd 1.1.0を現行のminaseへ適用した場合にどれだけ速くなるかを、試作を本体へ組み込む前に機械語の読解と移植可能ビルドとの速度比較で見積もった記録である。

結論は次のとおりである。
fearless_simd自体はminaseの方針と両立し、`#![forbid(unsafe_code)]`の下でAVX2向けの**核**（同じ演算を要素の列へ一括して施す小さな関数、カーネル）を書いて実行できた。
しかし、現行のminaseでSIMDの形をした処理はFM（factorization machine）の累算と補正の4つの核だけであり、その4つはLLVMが既にAVX2の命令へ自動ベクトル化している。
fearless_simdで書き直した累算の核は現行と同一の命令列になり、補正の核は符号つきの拡張乗算（`vpmuldq`）をライブラリが公開していないため、乗算に関係する命令が現行の約5倍になった。
したがって、測定機の標準ビルド（`target-cpu=native`）では書き換えで速度向上を期待する根拠が得られず、本体への試作は見送った。
残る利点は、`.cargo/config.toml`を使わない移植可能ビルド（`cargo install minase`など）でもAVX2を使えることだが、標準ビルドと移植可能ビルドの速度差は現行で約1.7%である（[測定記録](../measurements/bench-target-cpu-fm.md)）。
fearless_simdを再検討する時期は、設計中のNNUEで幅64以上の隠れ層の推論を実装するときである。

調査対象は2026年10月8日のmasterのコミット`bc0617a`である。
確認に使った検証用のcrateとベンチの手順は[fearless-simd-feasibility/](fearless-simd-feasibility/)に置き、命令の内訳と速度の数値は[測定記録](../measurements/bench-target-cpu-fm.md)に置いた。

## fearless_simdの仕組みと制約

fearless_simdは、命令集合ごとのトークン型（`Avx2`など）を`Simd`トレイトで抽象化し、`#[simd]`属性を付けた総称関数を`dispatch!`マクロが各命令集合向けに単相化して実行時に選ぶ。
命令集合を検出する`Level::new()`は結果を静的領域に一度だけ保存し、取得済みの`Level`を受け取った`dispatch!`は列挙型の値で処理を選ぶ。
安全性の根拠は、Rust 1.87以降で`#[target_feature]`つき関数の中の組み込み関数が安全に呼べるようになったことにあり、検出済みのトークンを持つ側だけがその関数へ入れる構造になっている。
検証用のcrateでは、`#![forbid(unsafe_code)]`と`[lints.rust] unsafe_code = "forbid"`の両方を掲げたまま、FMの2つの核を`#[simd]`と`dispatch!`で書いてビルドと実行に成功し、検出された水準はAVX2だった。
測定機のCore Ultra 7 265KFはAVX-512を持たないので、本書の比較は256ビット幅のAVX2までであり、AVX-512を持つ機械で16要素幅の型が自動ベクトル化を上回るかは本書では分からない。

採用には2つの制約がある。
第1に、1.1.0の最小対応Rustは1.89であり、minaseが宣言する1.88より新しい。
版を指定しない`cargo info fearless_simd`は、1.88に収まる0.5.0を表示した（`version: 0.5.0 (latest 1.1.0)`）。
採用するなら`rust-version`を1.89へ上げ、[最小対応版の教訓](../lessons/verify-msrv-with-toolchain.md)に従ってその版のツールチェーンで検査する。
lishogiのBotのDockerイメージはRust 1.98でビルドしているので、引き上げの支障はない。
第2に、公開している整数演算の範囲がNNUE向けの組み込み関数より狭い。
1.1.0の`Simd`トレイトにあるのは、整数の加減乗、幅の拡張と縮小（`widen`、`narrow`）、水平和（`reduce_sum`）などであり、符号つき32ビット同士の64ビット積（`vpmuldq`）や16ビット積和（`vpmaddwd`）はない。
64ビット乗算はAVX2に該当命令がないため、32ビット積3回とシフトで模倣している。
不足する命令は`kernel!`マクロで組み込み関数を直接呼ぶ安全な関数として書けるが、その場合は命令集合ごとに関数を分けて書く。

## minaseでSIMDの形をした処理

探索の自己時間の内訳（[第2期高速化の最終プロファイル](../measurements/movegen-speedup-2-final-profile-depth5.md)）では、静止探索の本体が27.7%、通常駒の捕獲可能升の計算が11.9%、SEE（static exchange evaluation、駒の取り合いの静的評価）の逆引きが11.3%、置換表の照合が10.5%、主探索の本体が9.2%を占める。
これらは3個の`u64`からなるビットボードのビット演算、分岐、および置換表のメモリ待ちであり、同じ演算を多数の要素へ並列に施す形をしていない。
`target-cpu=native`と移植可能な`x86-64`のビルドに速度差がなかった[以前の測定](../measurements/bench-target-cpu.md)も、主要な処理が整数のスカラー演算であることを示している。

SIMDの形をしているのは、評価関数のFMである。
FMは特徴ごとに潜在次元32の埋め込み（`i16`）を持ち、累算値（`i32`の32要素）へ特徴の行を足し引きし、評価のたびに累算値の符号つき二乗和（`i64`）から補正を求める。
この処理は`add`、`remove`、`replace`、`correction`の4つの核からなり、通常着手の更新で両視点の`replace`、捕獲で`remove`、先獅子特徴の変化で`add`または`remove`、評価時に手番側の`correction`を呼ぶ。
FMの採用によるNPS（1秒あたりの探索ノード数）の低下は約14%であり、4つの核を速くしても回復できるのはこの費用のうち核の計算にあたる部分に限られる。

## 現行の機械語

4つの核は本体では呼び出し元へインライン展開されるので、`#[inline(never)]`を付けた診断用ビルドで機械語を読んだ。
`target-cpu=native`のビルドでは、4つとも256ビットのAVX2命令へ自動ベクトル化されていた。
累算の3つは16ビットの行を`vpmovsxwd`で32ビットへ符号拡張して`vpaddd`と`vpsubd`で足し引きし、`correction`は符号を`vpmulld`で掛けてから`vpmuldq`で64ビット積を取り、`vpaddq`で水平に加算する。
`vpmuldq`は符号つき32ビットの下位要素同士を64ビットに掛ける命令であり、LLVMは補正の式`i64::from(a * d) * i64::from(a)`（`a`は累算値、`d`は次元ごとの符号）からこの命令を選んでいる。

移植可能な`x86-64`のビルドでは、累算の3つはSSE2の128ビット命令になり、符号拡張が`punpcklwd`と`psrad`の組になるため命令数は`native`の約2倍である。
`correction`は、SSE2に符号つきの32ビット拡張乗算がないためスカラーへ戻り、`imul`64回と符号拡張の`movslq`60回の列になる。
命令の内訳は[測定記録](../measurements/bench-target-cpu-fm.md)にある。

## fearless_simdで書いた同じ核の機械語

検証用のcrateで`replace`と`correction`をfearless_simdの総称関数として書き、AVX2向けの単相化の機械語を数えた。
`replace`は符号拡張、減算、加算、ロードとストアの回数が現行の自動ベクトル化と同一である。
`add`と`remove`は`replace`の片側だけの演算なので、同じ書き方で同じ命令列になると見込めるが、試作はしていない。
`correction`は、拡張乗算に関係する命令が現行の`vpmuldq` 8命令に対して`vpmuludq` 42命令になり、シフトとシャッフルも加わった。
その理由は、fearless_simdが64ビット積を符号なし32ビット積3回の合成で模倣し、符号つきの拡張乗算を公開していないことにある。
この命令列の速度は測っていないが、現行より速くなる根拠はない。
`kernel!`で`_mm256_mul_epi32`を直接呼べば現行と同じ命令になるが、それは自動ベクトル化の結果を手で書き写すことであり、速度は変わらない。

## 移植可能ビルドとの速度差

fearless_simdが速度に寄与し得る唯一の状況は、`.cargo/config.toml`の`target-cpu=native`が効かないビルドである。
`cargo install minase`で導入した利用者や、別の配備手順でビルドした場合がこれにあたる。
その見込みを測るため、同じコミットの`native`と`x86-64`のビルドを[測定記録](../measurements/bench-target-cpu-fm.md)の手順で交互に測った。
NPSの差は約1.7%、命令数の差は4.2%だった。
`target-cpu`はエンジン全体の機械語に作用するので、この差をFMだけに帰属させることはできないが、FM採用前の同じ比較では差が0.3%で測定の幅に埋もれていたので、FMの命令列の差が主に寄与したと読める。
fearless_simdで回復できるのはFMの分だけであり、FM単独の寄与は測っていないので、見込みは約1.7%を超えない目安である。

## 判断

現行のminaseにfearless_simdを組み込む試作は見送る。
標準ビルドでは自動ベクトル化が既に同じ命令を生成しており、`correction`に限ってはライブラリが公開する整数演算の範囲のために乗算の命令が増えるからである。
移植可能ビルドの約1.7%の見込みは、依存の追加と最小対応版の引き上げに見合わない。

再検討の条件は、SIMDの形をした処理が増えることである。
具体的には、設計中のNNUE（efficiently updatable neural network）の推論であり、幅64から128の隠れ層で`i16`の累算値の差分更新、clipped ReLUの二乗、および`i8`または`i16`の内積が1ノードあたり数百要素の規模で現れる。
その時点でfearless_simdを使う場合は、内積に`vpmaddwd`や`vpdpbusd`（VNNI）が要るので、`Simd`トレイトの演算だけでは足りず、`kernel!`で命令集合ごとの関数を書く前提で設計する。
Stockfishとやねうら王のNNUEの全結合層も、`_mm256_madd_epi16`や`vpdpbusd`を命令集合ごとに組み込み関数で書き分けている（[Stockfish](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/nnue/layers/affine_transform.h#L43-L226)、[やねうら王](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/eval/nnue/layers/affine_transform.h#L28-L96)）。

## 再現手順

機械語の読解は、`crates/minase/src/eval/pst/fm.rs`の4つの核に`#[inline(never)]`を付けて別のターゲットディレクトリへビルドし、`objdump -d`でシンボル`Fm::add`などの範囲を切り出して命令名（ニーモニック）ごとの出現数を数える。
検証用のcrateは[fearless-simd-feasibility/](fearless-simd-feasibility/)にあり、minaseのワークスペースから独立した単独のcrateとして`cargo build --release`と実行を行うと、検出された水準と2つの核の結果の一致を表示する。
速度比較の手順は同じディレクトリの`bench-native-vs-x86-64.sh`にある。
