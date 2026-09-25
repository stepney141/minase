# チェスエンジンのSPSA調整で使われた対局数

## 結論

確認した公開記録では、SPSAの実行規模はStockfishの49,923ペアからOpenBench上のKoivistoの200,027ペアまで異なる。
これらは個別の調整例であり、必要な対局数の下限や、予定した反復数の終了による収束を示さない。
Fishtestは係数の明確な移動と偶然による移動の抑制を良い調整の目安とする一方、変化がほとんどない調整の早期停止も勧めている。[Fishtestの公式手引き](https://github.com/official-stockfish/fishtest/wiki/Creating-my-first-test#spsa-tests)。

## 公開された実行規模

StockfishのFishtestでは1反復を2局と定義する。[Fishtestの公式手引き](https://github.com/official-stockfish/fishtest/wiki/Creating-my-first-test#spsa-algorithm-in-fishtest)。
OpenBenchも2局を1ペアとして扱い、公開記録の `Pairs Per Point` は1つの調整点に用いるペア数である。[OpenBenchの公式手引き](https://github.com/AndyGrant/OpenBench/wiki/SPSA-Tuning-Workloads#spsa-methodologies)。
表のペア数は各記録の実績局数を2で割ったもので、予定局数とは区別した。

| エンジンと実行記録 | 係数 | 予定反復数 | 実績局数 | 実績ペア数 |
|---|---:|---:|---:|---:|
| Stockfishの探索係数の実行[^sf22] | 22 | 50,000 | 99,846 | 49,923 |
| Stockfishの着手順序係数の実行[^sf20] | 20 | 50,000 | 100,000 | 50,000 |
| [WinterのOpenBench実行](https://chess.grantnet.us/tune/37334/) | 14 | 10,000 | 160,000 | 80,000 |
| [Stashの駒交換評価係数のOpenBench実行](https://chess.grantnet.us/tune/35977/) | 8 | 10,000 | 160,008 | 80,004 |
| [KoivistoのOpenBench実行](https://chess.grantnet.us/tune/37596/) | 256 | 25,000 | 400,054 | 200,027 |

[^sf22]: [Fishtestの公式API記録](https://tests.stockfishchess.org/api/get_run/689903860049e8ccef9d6415)。
[^sf20]: [Fishtestの公式API記録](https://tests.stockfishchess.org/api/get_run/6a201d3d818cacc1db0ad315)。

Stockfishの22係数の記録は `finished: true`、予定100,000局、実績99,846局、`iter: 49923`、`num_iter: 50000` を示す。
この実行は予定数に154局届く前に、実行者が手動で停止した。[Fishtestの操作履歴API](https://tests.stockfishchess.org/api/actions)に実行ID `689903860049e8ccef9d6415` をPOSTすると、`stop_run` と `User stop` が記録されている。
20係数の記録は予定100,000局と実績100,000局が一致し、反復数も50,000に達している。
どちらの記録も `param_history` は空であり、公開APIの最終値から係数の推移は点検できない。
この調整を参照した[Stockfishのコミット](https://github.com/official-stockfish/Stockfish/commit/2e91a8635468e40c89a2303ce50384864d088611)は、調整値の一部を別の対局で退けたうえで、残る変更を短時間条件と長時間条件の対局で検定している。
したがって、同コミットの検定局数は表のSPSA局数に含めない。

OpenBenchの3件は、画面に表示された完了反復数と実績局数をそのまま採った。
StashとKoivistoでは実績が予定局数をわずかに超えるが、表は実績の値で換算した。
これらの公開画面に表示された完了反復数は、OpenBenchの手引きでいう「評価する調整点の数」の達成を表し、最適値への到達を判定する指標ではない。[OpenBenchの公式手引き](https://github.com/AndyGrant/OpenBench/wiki/SPSA-Tuning-Workloads#spsa-hyperparameters)。

## 収束について言えること

Fishtestの手引きは、開始時に指定した総局数から最後の摂動幅と更新率を定め、実行中に総局数を変更できないと明記する。[Fishtestの公式手引き](https://github.com/official-stockfish/fishtest/wiki/Creating-my-first-test#spsa-tests)。
同じ手引きは、数千局の後にも係数がほぼ動かなければ調整を止めるよう勧めるが、係数の動きが止まれば最適値へ収束したとは定義していない。
Fishtestの[実装上の終了判定](https://github.com/official-stockfish/fishtest/blob/master/server/fishtest/schemas.py#L678-L690)も、SPSAに係数の収束条件を設けていない。
OpenBenchの手引きも反復数を実行予算として定義し、収束の自動判定条件を示していない。[OpenBenchの公式手引き](https://github.com/AndyGrant/OpenBench/wiki/SPSA-Tuning-Workloads#spsa-hyperparameters)。

したがって、これらの記録から確認できるのは、指定した条件で何局を指したか、または調整値を組み込んだ変更が別の対局で採用条件を満たしたかまでである。
真の最適値は記録に与えられておらず、完了反復数と最終係数だけでは「収束した」と判定できない。
minaseの[1,500反復と12,000ペア](../plans/spsa.md#調整セッションの規模と所要時間)についても、上記の個別例の大きさだけから不足または十分とは結論できない。
