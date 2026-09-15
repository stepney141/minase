# 単位D（PGO）の手動ビルド比較と継続負担の測定（bench深さ5）

## 目的

捕獲生成とSEEの改修が終わった単位Cの採用版（コミット`888bd10`）の同じソースについて、通常版とプロファイル誘導最適化（PGO）版を手動ビルドで比較し、NPS中央値が10%以上増えるかで再現基盤を作るかを判定する。あわせて、再現基盤を作る場合の継続負担（再生成時間、プロファイル容量、複数回更新した場合のgitオブジェクトの増分）を測る。

## コマンドライン

学習入力は測定用の`bench`15局面（lishogiの対局から採った局面）と分離し、校正用エンジン`usi_random`で作ったランダム対局12局から手数12、40、80、120、160、200、260、320の局面（対局長未満のもの、計55局面）を`go depth 5`で探索して集めた。学習の実行は`minase --protocol usi --rules engine-default`をUSIで駆動するドライバで行った。

```console
git archive 888bd10 | tar -x -C <作業ディレクトリ>
cargo build --release --bin usi_random --bin bench
RUSTFLAGS="-C target-cpu=native -C profile-generate=<プロファイル出力先>" CARGO_TARGET_DIR=<計測用> cargo build --release --bin minase
<ドライバ> --minase <計測用>/release/minase --random target/release/usi_random --games 12 --depth 5 --seed 9001
llvm-profdata merge -o merged.profdata <プロファイル出力先>/*.profraw
RUSTFLAGS="-C target-cpu=native -C profile-use=<merged.profdata>" CARGO_TARGET_DIR=<PGO用> cargo build --release --bin bench
target/release/bench --depth 5 --threads 1 --repetitions 3
<PGO用>/release/bench --depth 5 --threads 1 --repetitions 3
```

`llvm-profdata`はrustupの`llvm-tools`（`rustc 1.98.0`、LLVM 22.1.8）のものを使った。リンク時最適化と`target-cpu=native`は両版で同じである。

## エンジン

通常版とPGO版はいずれもコミット`888bd10`のソースであり、違いはプロファイルの適用だけである。`bench`は規則セット`engine-default`を固定で使う。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、2026年9月15日に他の負荷を走らせずに1スレッドで測った。置換表は既定容量を1個使い回し、局面ごとの`clear()`は計測区間外である。

## 結果

通常版とPGO版を交互に2回ずつ測った3回実行の中央値は次のとおりである。局面別の深さ、ノード数、最善手、探索値は両版で一致した。

| 版 | 1回目の経過（NPS） | 2回目の経過（NPS） |
|---|---:|---:|
| 通常版 | 0.565秒（2,246,285） | 0.570秒（2,224,648） |
| PGO版 | 0.475秒（2,672,121） | 0.482秒（2,630,812） |

NPSの比は1回目が1.190倍、2回目が1.183倍であり、判断値の10%を超える。

継続負担は次のとおりである。

| 項目 | 測定値 |
|---|---|
| 再生成の所要時間 | 計測用ビルド約12秒、学習の実行43秒（うち探索2.2秒、残りはランダム対局の生成）、統合1秒未満、適用ビルド約9秒の計約65秒 |
| プロファイルの容量 | 6,089,008バイト（`merged.profdata`、23,581関数） |
| gitオブジェクトの増分 | 初回1,332 KB（`git gc`後のpack）。同じソースで学習を再実行して更新すると32 KBと28 KB、別のソース（コミット`5c7463c`）のプロファイルへ更新すると56 KBの増分 |
| 再現性 | 同じ学習入力でも`merged.profdata`のSHA-256は実行ごとに異なる（探索時間に依存する経路の計数差）。速度比は再実行で1.183〜1.190倍の範囲だった |

再現基盤を整えた後の作業ツリー（rustcラッパーでプロファイルを適用し、`build.rs`が照合記録を検証する構成）を、通常の`scripts/bench_compare.py --reference 5c1db50 --baseline 0e3dfa6`で測った結果は次のとおりである。学習入力は`scripts/pgo_profile.py`の既定（`usi_random`に局ごとのシード9001〜9012を与えた12局から75局面、`go depth 5`）であり、生成時間は65.5秒、`pgo/minase.profdata`は7,112,296バイトである。

| 版 | 総ノード数 | 経過（中央値） | NPS（中央値） |
|---|---:|---:|---:|
| 基準 0e3dfa6 | 1,268,805 | 0.844秒 | 1,503,101 |
| 参照 5c1db50 | 1,268,607 | 0.851秒 | 1,491,277 |
| 候補（単位D、PGO適用） | 1,268,607 | 0.470秒 | 2,696,887 |

局面別の深さ、ノード数、最善手、探索値は15局面すべてで参照と一致し、候補の基準比はNPS 1.794倍、参照比は1.808倍である。

## 結論

PGOは同じソースでNPSを約1.19倍にし、判断値10%を超える。継続負担は再生成約65秒と再生成ごとの数十KBのgit増分にとどまるので、再現基盤（rustcラッパーによる適用、照合記録の検証、生成スクリプト）を整えて採用版に含め、単位Dを採用する。
