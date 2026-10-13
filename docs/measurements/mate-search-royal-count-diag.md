# 王駒の枚数と終局理由の診断

## 目的

[詰み探索（df-pn）](../plans/mate-search.md)の対象範囲を決めるため、保存済みの対局記録について、終局付近で詰まされる側が王駒（王将または玉将と太子）を2枚持つ頻度と、終局理由の分布を数える。
対局は行わず、記録の再生だけを行う。

## コマンドライン

集計ツールは[main.rs](mate-search-royal-count-diag/main.rs)（minase-coreにパスで依存する独立したcargoプロジェクト、[Cargo.toml](mate-search-royal-count-diag/Cargo.toml)）であり、[format_report.py](mate-search-royal-count-diag/format_report.py)が集計JSONから表を作る。
ツールはリポジトリ外のディレクトリへ複製してビルドする。

```console
cargo build --release -j 1
nice -n 19 target/release/royal-diag lishogi <out>/lishogi.json data/lishogi/games.ndjson
nice -n 19 target/release/royal-diag matches <out>/matches.json \
  data/matches/move-ordering-all-ltc data/matches/move-ordering-all-stc
python3 format_report.py <out>
```

本記録の数値を出した初版のツールは、`lishogi`の引数を「入力、出力」の順に取っており、出力先の指定を誤って入力ファイル`data/lishogi/games.ndjson`に集計JSONを上書きした。
ツールは全件を読み終えてから書き込むので集計値は無傷であり、入力ファイルは同一内容の`data/strength-stage9/human/games-final.ndjson`（31,796,374バイト、23,567局、`data/lishogi/state.json`の`game_ids`と一致）から復元した。
保存したツールは引数順を「出力、入力」に直し、出力先が`.json`でなければ停止する。
集計JSONは[lishogi.json](mate-search-royal-count-diag/lishogi.json)と[matches.json](mate-search-royal-count-diag/matches.json)である。

## エンジン

再生にはminase-core（master、コミット8d0e382時点）の`Game::play`を使った。
lishogiの棋譜（`data/lishogi/games.ndjson`、2026年9月22日取得、23,567局）は`Rules::LISHOGI`（L1、L2、P0、P3、R1、E1、E3）で再生し、同じ着手列を`e1: false`にした双子対局にも与えて、盤上に詰みが成立した手数を記録した。
自己対局は[手の順序付けの改善](../plans/archived/move-ordering.md)の[LTC](move-ordering-all-ltc.md)（599ペア）と[STC](move-ordering-all-stc.md)（309ペア）の全1,816局であり、候補ea0c75a対基準cd081f4、規則L0、P0、R1、E0である。
自己対局は全記録からの無作為標本ではなく、直近2測定の全局である。
王駒の枚数は`Position::royal_pieces(color).popcount()`、太子の有無は`pieces_of_kind(color, PieceKind::CrownPrince)`で数えた。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア）であり、他の測定は走っていなかった。
単一スレッドで、lishogiの再生に87秒、自己対局の再生に16秒を要した。

## 結果

対象局数は、lishogiが23,567局のうち22,855局（除外は`initialSfen`つき459、着手なし247、`noStart` 5、`Rules::LISHOGI`で不合法手が現れた1局）、自己対局が1,816局のうち1,790局（手数上限4096による打ち切り26局を除く）である。
自己対局では、保存された終局理由と再生の裁定が1,790局すべてで一致した。

終局理由は次のとおりである。

| 情報源 | 終局理由 | 局数 | 割合 |
|---|---|---:|---:|
| lishogi | 投了 | 11,394 | 49.85% |
| lishogi | 王駒捕獲 | 5,744 | 25.13% |
| lishogi | 時間切れ（outoftime） | 4,187 | 18.32% |
| lishogi | 裸玉（E3） | 822 | 3.60% |
| lishogi | 切断（timeout） | 460 | 2.01% |
| lishogi | 引き分け（合意など） | 194 | 0.85% |
| lishogi | 連続王手の反復（R1の勝ち） | 27 | 0.12% |
| lishogi | 反復の引き分け | 22 | 0.10% |
| lishogi | 裸玉の引き分け、その他 | 5 | 0.02% |
| 自己対局 | 詰み | 1,629 | 91.01% |
| 自己対局 | 反復の勝ち（R1） | 92 | 5.14% |
| 自己対局 | 反復の引き分け | 56 | 3.13% |
| 自己対局 | 駒枯れ（E0） | 7 | 0.39% |
| 自己対局 | 王駒捕獲 | 6 | 0.34% |

lishogiの王駒捕獲5,744局のうち、双子対局で捕獲の前に詰みが成立していたのは3,807局（66.28%）、詰みを経ない捕獲（主に王手放置）は1,937局（33.72%）である。
投了11,394局のうち、投了時点で盤上に詰みが成立していたのは540局（4.74%）である。
自己対局の王駒捕獲6局は、いずれも無作為な開始手順の直後にエンジンの初手で王将を取られた局である。

勝敗の決まった局について、終局からの時点ごとの（敗者の王駒数、勝者の王駒数）の分布は次のとおりである。

| 情報源 | 時点 | 対象局数 | 敗者2枚の割合 | 勝者2枚の割合 |
|---|---|---:|---:|---:|
| lishogi | 終局局面 | 22,610 | 0.25% | 2.06% |
| lishogi | 1プライ前 | 22,610 | 0.38% | 2.08% |
| lishogi | 10プライ前 | 20,123 | 0.67% | 2.81% |
| lishogi | 20プライ前 | 18,434 | 0.79% | 2.96% |
| 自己対局 | 終局局面 | 1,734 | 0.00% | 4.73% |
| 自己対局 | 1プライ前 | 1,734 | 0.12% | 4.73% |
| 自己対局 | 10プライ前 | 1,734 | 0.00% | 4.79% |
| 自己対局 | 20プライ前 | 1,728 | 0.35% | 4.34% |

全局面のうち手番の相手が王駒2枚である局面は、lishogiで3,636,001局面中49,610（1.36%）、自己対局で903,094局面中7,151（0.79%）である。
太子が一度でも現れた局は、lishogiで1,171局（5.12%）、自己対局で238局（13.30%）である。

## 結論

詰まされる側が王駒2枚である局面を終局付近で扱う頻度は両情報源で1%未満なので、詰み探索の初版が受け方の王駒1枚の局面だけを対象とすることによる取りこぼしは小さい。
詰ます側が王駒2枚である局は2〜5%あるので、攻め方の王駒の安全（受け方が攻め方の最後の王駒を先に取る反証）は2枚を前提に判定する必要がある。
自己対局では決着の91%が詰みであり、lishogiでは王駒捕獲の66%が詰みを経ているので、終局前の局面は詰み探索の固定局面集の供給源になる。
