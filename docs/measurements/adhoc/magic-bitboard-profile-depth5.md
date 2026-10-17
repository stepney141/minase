# Magic bitboard検討のための走り計算の負荷

## 目的

現行版の探索に占める走りの利き計算の割合を測り、参照表による置換を検討する際の費用の目安を得る。
候補実装との速度比較は行っていない。

## コマンドライン

現在のrelease設定にフレームポインタと行情報を加え、ソースのインライン化指定は変更しなかった。
行情報の除去を防ぐため、Cargoの`debug`と`strip`を明示した。
以下はリポジトリのルートで実行する。

```sh
CARGO_TARGET_DIR=/tmp/minase-magic-profile \
CARGO_PROFILE_RELEASE_DEBUG=1 \
CARGO_PROFILE_RELEASE_STRIP=none \
RUSTFLAGS='-C target-cpu=native -C force-frame-pointers=yes' \
cargo build --release --locked --bin bench

taskset -c 0 perf record -e cpu_core/cycles/u -F 4000 --call-graph fp \
  -o /tmp/minase-magic-profile/perf-debug.data \
  /tmp/minase-magic-profile/release/bench \
  --depth 5 --threads 1 --repetitions 10 \
  > /tmp/minase-magic-profile/bench-debug.txt

perf script -i /tmp/minase-magic-profile/perf-debug.data --no-inline \
  -F period,ip,sym,dso > /tmp/minase-magic-profile/perf-script-physical.txt
perf script -i /tmp/minase-magic-profile/perf-debug.data --inline \
  -F period,ip,sym,dso > /tmp/minase-magic-profile/perf-script-debug.txt
python3 docs/measurements/magic-bitboard-profile-depth5/summarize.py \
  /tmp/minase-magic-profile/perf-script-physical.txt \
  /tmp/minase-magic-profile/perf-script-debug.txt
```

集計は、物理的な呼び出しスタックに`minase::search::run_search_team::{closure#3}`を含む標本だけを選ぶ。
これにより、起動時の処理と探索区間外の置換表クリアを除く。
各標本はサンプリング周期に相当するサイクル数で重みづけし、同じ標本を2回加算しない。
その標本の展開されたスタックに`sliding_control`があれば、同関数にインライン化された減算、ビット走査、表参照を含む走り計算へ帰属させる。
物理的な関数の自己時間と、このインライン化された部分の割合は区別して報告する。

## エンジン

コミット`1633f534cf0e8c96738b11158e68f31f0f144f01`、minase v1.1.0を対象とした。
本体ソースへの計測用変更はない。
`bench`の固定15局面を深さ5、1スレッド、規則セット`engine-default`で探索した。
置換表は256 MiBを使い回し、局面ごとのクリアは探索の計測区間外である。
10反復の前に1回のウォームアップがあり、プロファイルにはこの探索も含まれる。

## 環境

2026年9月21日に、Intel Core Ultra 7 265KF、20物理コア、20論理コア、rustc 1.98.0で測定した。
性能コアのCPU 0へ固定し、ユーザー空間の`cpu_core/cycles/u`だけを記録した。
計測時に別のベンチマークや自己対局は実行していない。
プロファイル用の設定とperfの費用を含む実行なので、下記の探索速度を通常ビルドの採否判定には使わない。

## 結果

全15,889標本のうち、探索中の12,809標本を集計対象とした。
探索標本の周期の合計は16,497,917,882サイクルであり、走り計算に帰属した546標本はその4.263%だった。
以下の割合の分母は、すべてこの探索標本の合計である。
元の集計値は[summary.json](magic-bitboard-profile-depth5/summary.json)、集計方法は[summarize.py](magic-bitboard-profile-depth5/summarize.py)にある。

| 処理 | 物理的な関数の自己時間に相当する割合 | うち走り計算に帰属した割合 |
|---|---:|---:|
| `attackers_to_by`による利きの逆引き | 14.40% | 2.53% |
| `ordinary_capturer`による通常駒の捕獲先の計算 | 12.76% | 0.55% |
| `piece_control_without_special`による固定利きと走りの計算 | 5.91% | 1.18% |
| 上記3関数の合計 | 33.07% | 4.26% |

3関数の33.07%には、固定利き、捕獲対象との交差、駒の参照、方向の選択などが含まれる。
これをmagicで置換できる走り計算の割合とみなしてはならない。
また、4.26%も最適化後の命令に付いたデバッグ情報とサイクル標本に基づく帰属であり、命令移動やメモリ待ちを含めた厳密な費用分解ではない。

全10反復で総ノード数は598,014で一致した。
探索時間の中央値は0.290664秒、1秒あたりの探索ノード数の中央値は2,057,403だった。
各反復の値は[bench.txt](magic-bitboard-profile-depth5/bench.txt)に保存した。
過去の第2期のプロファイルとは探索木が異なるため、過去の所要時間や件数から今回の速度改善率を計算しない。

探索全体に占める対象処理の費用を`f`、その処理の速度倍率を`s`とすると、他の費用が不変の場合の全体倍率は`1 / (1 - f + f / s)`となる。
仮に`f = 0.04263`と置けば、走り計算が2倍になる場合の全体倍率は約1.022、同部分の費用を完全に取り除く場合は約1.045である。
これは標本の割合を使った条件付きの概算であり、magic実装の予測値や厳密な改善上限ではない。
新しい表が他の処理のキャッシュ利用やコンパイラの最適化へ及ぼす影響は、候補を組み込んだ探索で測る必要がある。

### PEXTの呼び出し条件

現在のビルド条件で、PEXT命令をRustから直接呼べるかも確認した。
次のコードを`rustc --edition 2024 -C target-cpu=native`でコンパイルすると、rustc 1.98.0はE0133を返した。

```rust
#![forbid(unsafe_code)]
fn main() {
    assert_eq!(std::arch::x86_64::_pext_u64(0b1010, 0b1100), 0b10);
}
```

診断は、呼び出し側にも`bmi2`の`#[target_feature]`が必要であり、ビルドのフラグだけでは通常関数からの安全な呼び出しにならないと説明している。
このため、`unsafe_code = "forbid"`を保った直接intrinsicの呼び出しを、`target-cpu=native`だけで実現できるとは扱わない。
これは[Rust Referenceの制約](https://doc.rust-lang.org/reference/attributes/codegen.html#attributes.codegen.target_feature.safety-restrictions)とも一致する。
乗算、シフト、マスクによる方式には、このintrinsic固有の制約はない。

## 結論

走り計算は現行の探索でも計測可能な費用を持つが、その置換だけで大幅な高速化を得られる根拠はない。
小さな表を使う候補を比較する際は数%程度の変化を見分けられる測定を用い、採否は入力変換とキャッシュへの影響を含む通常ビルドの探索と、最終的な自己対局測定で判定する。
