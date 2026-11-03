# tataraの調査の数値と出典

本書は[将棋のNNUE学習器tataraからminaseへ移せる技法](../tatara-nnue-transfer.md)で引用した対局の数値を、出典ごとにまとめた付録である。
数値はいずれも各資料の表のとおりであり、本調査で新しい対局は行っていない。
nEloは先後を入れ替えた2局の組の結果から求めた正規化Elo、Eloはロジスティックな勝率から求めた値であり、算出法が違うので換算しない。

## tataraのリリースとPR

活性化と入力の比較は[v0.1.0](https://github.com/SH11235/tatara/releases/tag/v0.1.0)と[issue #165](https://github.com/SH11235/tatara/issues/165)にある。
構成は`simple`の`512x2-8-64`、60 superbatch、Ranger、WRM損失、およびbatch 65,536である。

| 比較 | 条件 | 結果 |
|---|---|---|
| Pairwise対SCReLU | SPRT、H0=0、H1=+5 nElo、α=β=0.05 | 1431勝1210敗36分、+29.5 ±13.2 nElo、LLR +2.97、採択 |
| 活性化の3者総当たり | 1秒秒読み、各組200局、各組の誤差±48 nElo | Pairwise 53.5%、SCReLU 49.5%、CReLU 45.5% |
| 3者の探索速度 | 同上 | 372.0K、361.0K、359.7K局面/秒 |
| 入力の5者総当たり | 1秒秒読み、各組1,000局、各組の誤差±22 Elo | hm-split 51.8%、hm-merged 51.1%、halfkp 50.8%、merged 48.0%、split 45.1% |

対局の記録のあるPRは次のとおりである。

| PR | 変更 | 結果 |
|---|---|---|
| [#363](https://github.com/SH11235/tatara/pull/363) | 第1層の初期値を半値幅0.01からfan-inへ | 240 sbで+23.86 ±14.45 nElo（判定中）、400 sb、1,220ペアで+31.17 ±13.79 nElo、LLR +2.874、採択 |
| [#363](https://github.com/SH11235/tatara/pull/363) | L1からL3の初期値もfan-inへ | 400 sb、1,500ペアで+5.86 ±12.43 nElo、未決着 |
| [#303](https://github.com/SH11235/tatara/pull/303) | 特徴分解を既定に | +8 ±10 nElo（回帰なし）、40 sbの時点で+29 Elo、学習速度+52.8%（fold/reduce方式による） |

本文で「測られていない」と書いた項目の根拠は次のとおりである。

| 項目 | PRまたは文書 | 記述 |
|---|---|---|
| norm loss | [#284](https://github.com/SH11235/tatara/pull/284)、[#283](https://github.com/SH11235/tatara/issues/283) | 棋力検証の項目が未実施のまま |
| WRM損失の一般化 | [#277](https://github.com/SH11235/tatara/pull/277) | 有効化時の棋力の影響はSPRTが必須とあるだけ |
| per-group学習率と重み減衰 | [#292](https://github.com/SH11235/tatara/pull/292) | SPRTはPRの範囲外 |
| 重みの切り詰めの変更 | [#281](https://github.com/SH11235/tatara/pull/281) | SPRTの項目が未実施のまま |
| 学習率の減衰率0.992 | [#275](https://github.com/SH11235/tatara/pull/275) | 差の見積もりだけで、測定は範囲外 |
| L2とL3のshared項 | [#391](https://github.com/SH11235/tatara/pull/391) | 学習の完走の確認だけ |
| dense QAT | [#413](https://github.com/SH11235/tatara/pull/413) | 本格学習と対局評価を実施していないと明記 |
| チェックポイントの平均 | [#431](https://github.com/SH11235/tatara/pull/431) | 量子化の一致の検査だけ |
| threatの学習器 | [#306](https://github.com/SH11235/tatara/pull/306)、[#307](https://github.com/SH11235/tatara/pull/307) | 学習速度とGPUメモリだけ。棋力は対象外 |
| effect bucketの学習器 | [#359](https://github.com/SH11235/tatara/pull/359) | 等深さのSPRTは別途進行中との記述だけ |

threatを入れた学習器の学習速度とGPUメモリは、[PR #306](https://github.com/SH11235/tatara/pull/306)と[#307](https://github.com/SH11235/tatara/pull/307)が測っている。
いずれもRTX 3080 Ti（12 GB）、入力halfka-hm-merged、幅1536である。

| 構成 | 学習速度 | GPUメモリ |
|---|---:|---:|
| threatなし | 957K局面/秒 | 5.67 GB |
| cross-side（96,320次元） | 721K局面/秒 | 8.90 GB |
| same-class-major-pawn（173,568次元） | 524K局面/秒 | 11.17 GB |
| same-class（192,640次元）、半精度の状態 | 504K局面/秒 | 10.15 GB |
| full（216,720次元）、半精度の状態 | 493K局面/秒 | 10.73 GB |

## rshogiの公開比較実験

[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)の教師は公開データの約80億局面（深さ9）、学習は160 superbatch（約160億局面）であり、検証は別の出所の856,923局面である。
条件が違う行のnEloは直接比べられない。

| ネット | 学習器 | 符号一致率 | 対局の条件 | 水匠5に対する成績 | nElo |
|---|---|---:|---|---|---:|
| HalfKP 256x2-32-32 | tatara | 0.6806 | 固定ノード | 764勝1225敗11分 | −83 ±15 |
| HalfKP 256x2-32-32 | bullet-shogi | 0.67651 | 固定ノード | 774勝1215敗11分 | −81 ±15 |
| HalfKP 256x2-32-32 | nnue-pytorch | 0.65691 | 固定ノード | 215勝1779敗6分 | −437 ±15 |
| 幅1536、王の段のbucket | tatara | 0.69850 | 1秒秒読み | 580勝411敗9分 | +61 ±22 |
| 幅1536、王の段のbucket | BulletOu | 0.68457 | 1秒秒読み | 393勝602敗5分 | −78 ±22 |

[公開記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187)によると、nnue-pytorchの行は学習率の条件が最適でない参考値であり、BulletOuの行も最良の設定ではなく、特徴分解の掛け方も違う。
tataraのネットの探索速度は毎秒565k局面で、水匠5は毎秒701k局面である。
同じ構成のtatara対BulletOuは固定ノードで1377勝601敗22分（+150 ±15 nElo）だった。

教師の加工の比較は、すべてtataraで同じレシピであり、比較相手は同じ構成の加工なしのネットである（固定300kノード、約2,000局）。

| 構成 | 加工 | 符号一致率 | 成績 | nElo |
|---|---|---:|---|---:|
| HalfKP | 重複の除去 | 0.6797 | 1072勝911敗17分 | +29 ±15 |
| HalfKP | 静止探索の置換 | 0.6760 | 1293勝692敗15分 | +111 ±15 |
| HalfKP | 両方 | 0.6759 | 1279勝710敗11分 | +106 ±15 |
| 幅1536 | 重複の除去 | 0.6983 | 985勝995敗20分 | −2 ±15 |
| 幅1536 | 静止探索の置換 | 0.6887 | 977勝1005敗16分 | −5 ±15 |
| 幅1536 | 両方 | 0.6886 | 1040勝942敗18分 | +17 ±15 |

## nodchip氏の記事

いずれもtanuki-の実験であり、学習器はnnue-pytorchの派生で、教師は深層学習系の評価器で付け直している。
対局は8秒＋0.08秒のSPRTである。
検証損失は記事によって式が違うので、記事をまたいで比べない。

| 記事 | 比較 | 結果 | 検証損失 |
|---|---|---|---|
| [2026年1月21日](https://nodchip.hatenablog.com/entry/2026/01/21/000000) | データローダー修正後の幅1536と対戦相手 | −11.81 ±9.55 Elo、4,796局 | 0.0125→0.0123 |
| [2026年1月24日](https://nodchip.hatenablog.com/entry/2026/01/24/000000) | SGD+Momentum（学習率0.5）とRanger21の系統 | −188.75 ±38.45 Elo、440局 | 0.012571と0.0123 |
| [2026年1月29日](https://nodchip.hatenablog.com/entry/2026/01/29/000000) | やねうら王の勝率モデル（交差エントロピー型）と元のモデル（二乗誤差型） | −56.52 ±19.45 Elo、1,172局 | 0.0415と0.0123 |
| [2026年2月4日](https://nodchip.hatenablog.com/entry/2026/02/04/000000) | 進行度の8区分と王の位置のbucket | +30.72 ±12.10 Elo、3,028局 | 0.0122と0.0123 |
| [2026年3月11日](https://nodchip.hatenablog.com/entry/2026/03/11/000000) | 960億局面の幅1536と幅1024のHalfKP | +20.56 ±9.81 Elo、4,636局 | 式が違うので比較しない |
| [2026年4月5日](https://nodchip.hatenablog.com/entry/2026/04/05/000000) | 幅2048と幅1536 | −39.99 ±16.97 Elo、1,632局 | 記載なし |

過学習の記述はどの記事にもない。

## Stockfishのコミットの再計算

手元のStockfishのリポジトリ（コミット`5c4cef0b`）のコミットメッセージの勝敗数から、ロジスティックEloと95%区間を計算した。
区間は引き分けを含む三項の分散による近似である。
fishtestのSPRTの境界`<0.5,2.5>`などは正規化Eloの単位であり、下の値とは単位が違う。

| コミット | 内容 | 条件 | 局数 | Elo | 95%区間 |
|---|---|---|---:|---:|---|
| [`8e5392d7`](https://github.com/official-stockfish/Stockfish/commit/8e5392d7) | SFNNv10、threat | STC | 63,424 | +2.00 | +0.03〜+3.97 |
| 同 | 同 | LTC | 27,876 | +3.83 | +0.88〜+6.77 |
| 同 | 同 | VVLTC | 12,458 | +7.00 | +2.61〜+11.39 |
| [`a6d055d7`](https://github.com/official-stockfish/Stockfish/commit/a6d055d7) | SFNNv13、第2層16→32 | STC（棄却） | 189,184 | +0.03 | −1.11〜+1.16 |
| 同 | 同 | LTC | 37,902 | +2.97 | +0.47〜+5.47 |
| [`f4bcd404`](https://github.com/official-stockfish/Stockfish/commit/f4bcd404) | SFNNv16、ポーンの対 | STC（棄却） | 144,768 | −0.12 | −1.41〜+1.18 |
| 同 | 同 | LTC | 189,888 | +1.15 | +0.02〜+2.28 |
| [`a17c0ce2`](https://github.com/official-stockfish/Stockfish/commit/a17c0ce2) | SFNNv17、PSQTの廃止 | STC（非劣性） | 40,416 | +1.82 | −0.60〜+4.25 |
| 同 | 同 | LTC（非劣性） | 36,750 | +1.91 | −0.63〜+4.45 |
| [`5c4cef0b`](https://github.com/official-stockfish/Stockfish/commit/5c4cef0b) | SFNNv18、K32Q2と32 bucket | STC | 23,808 | +4.20 | +1.06〜+7.35 |
| 同 | 同 | LTC | 28,680 | +3.60 | +0.71〜+6.48 |

SFNNv10のコミットには、別のARM機での長時間の推定値+13.9 ±1.9 Eloも記載されている。
SFNNv18のコミットは、3つの単独の追加はいずれも非劣性の検定を通らず、最も近かったのは出力bucketだけの変更だったと記している。

## minaseの速度換算

[速度の記録](../../measurements/nnue-hidden-layer/nnue-hidden-layer-bench.md)の探索時間の増分を、[感度の記録](../../measurements/evaluation-gen1/sensitivity-time2x.md)の持ち時間2倍で+216 Eloで換算した。
式は`216 × log2(元の時間 / 新しい時間)`であり、探索速度の比r（1未満）なら`216 × log2(1/r)`である。
感度の記録自身が、線形性を仮定せず採否に使わないと書いているので、目安にとどめる。

| 探索速度の比 | Elo相当 |
|---:|---:|
| 0.98 | −6.3 |
| 0.95 | −16.0 |
| 0.90 | −32.8 |
| 0.88 | −39.8 |
| 0.85 | −50.6 |
| 0.816 | −63.4 |

threatの次元の見積もりは、将棋で`(9×2)² × 81 × 8 ≈ 21万`が実際の216,720に近いことを確かめ、同じ式に中将棋の値を入れたものである。
利き先の平均は、将棋の8に対して、走り駒の多い中将棋では20と仮定した。
29駒種では`(29×2)² × 144 × 20 ≈ 969万`、12駒種へまとめると`(12×2)² × 144 × 20 ≈ 166万`になる。
実際の次元は駒種の利きを数えて確かめていない。
