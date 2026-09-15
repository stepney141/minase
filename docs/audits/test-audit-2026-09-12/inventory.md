監査対象のテストは43ファイルに636件あり、全件の本文を確認した。
件数は対象範囲を照合するためのものであり、維持目標ではない。
Rustの `#[test]` とPythonのunittestテスト関数を追跡ファイルから抽出し、全体の実行結果と突き合わせた。

| ファイル | 件数 |
|---|---:|
| [src/bin/bench.rs](../../../src/bin/bench.rs) | 5 |
| [src/bin/match_report.rs](../../../src/bin/match_report.rs) | 4 |
| [src/bin/match_runner.rs](../../../src/bin/match_runner.rs) | 47 |
| [src/bin/match_runner/storage.rs](../../../src/bin/match_runner/storage.rs) | 12 |
| [src/bin/minase.rs](../../../src/bin/minase.rs) | 2 |
| [src/bin/random_play.rs](../../../src/bin/random_play.rs) | 5 |
| [src/bin/selfplay_gen.rs](../../../src/bin/selfplay_gen.rs) | 11 |
| [src/bin/usi_random.rs](../../../src/bin/usi_random.rs) | 5 |
| [src/core/adjudication.rs](../../../src/core/adjudication.rs) | 3 |
| [src/core/attacks/fixed.rs](../../../src/core/attacks/fixed.rs) | 2 |
| [src/core/attacks/tables.rs](../../../src/core/attacks/tables.rs) | 2 |
| [src/core/bitboard.rs](../../../src/core/bitboard.rs) | 3 |
| [src/core/direction.rs](../../../src/core/direction.rs) | 1 |
| [src/core/game.rs](../../../src/core/game.rs) | 54 |
| [src/core/movegen/tests/attackers.rs](../../../src/core/movegen/tests/attackers.rs) | 2 |
| [src/core/movegen/tests/lion_moves.rs](../../../src/core/movegen/tests/lion_moves.rs) | 19 |
| [src/core/movegen/tests/movement.rs](../../../src/core/movegen/tests/movement.rs) | 12 |
| [src/core/movegen/tests/pieces.rs](../../../src/core/movegen/tests/pieces.rs) | 30 |
| [src/core/movegen/tests/promotion.rs](../../../src/core/movegen/tests/promotion.rs) | 37 |
| [src/core/movegen/tests/properties.rs](../../../src/core/movegen/tests/properties.rs) | 6 |
| [src/core/piece.rs](../../../src/core/piece.rs) | 4 |
| [src/core/position.rs](../../../src/core/position.rs) | 22 |
| [src/core/repetition.rs](../../../src/core/repetition.rs) | 1 |
| [src/core/rules.rs](../../../src/core/rules.rs) | 48 |
| [src/core/square.rs](../../../src/core/square.rs) | 3 |
| [src/eval/features.rs](../../../src/eval/features.rs) | 3 |
| [src/eval/handcrafted.rs](../../../src/eval/handcrafted.rs) | 4 |
| [src/eval/pst.rs](../../../src/eval/pst.rs) | 22 |
| [src/eval/training_data.rs](../../../src/eval/training_data.rs) | 6 |
| [src/notation/cecp.rs](../../../src/notation/cecp.rs) | 7 |
| [src/notation/sfen.rs](../../../src/notation/sfen.rs) | 17 |
| [src/notation/usi.rs](../../../src/notation/usi.rs) | 10 |
| [src/protocol/cecp.rs](../../../src/protocol/cecp.rs) | 32 |
| [src/protocol/engine.rs](../../../src/protocol/engine.rs) | 7 |
| [src/protocol/usi.rs](../../../src/protocol/usi.rs) | 47 |
| [src/search/see.rs](../../../src/search/see.rs) | 9 |
| [src/search/tests.rs](../../../src/search/tests.rs) | 88 |
| [src/stats.rs](../../../src/stats.rs) | 13 |
| [tests/lishogi_replay.rs](../../../tests/lishogi_replay.rs) | 1 |
| [tests/match_runner.rs](../../../tests/match_runner.rs) | 4 |
| [tools/train/pst/test_pst_diagnostics.py](../../../tools/train/pst/test_pst_diagnostics.py) | 5 |
| [tools/train/pst/test_pst_workflow.py](../../../tools/train/pst/test_pst_workflow.py) | 10 |
| [tools/train/pst/test_train_pst.py](../../../tools/train/pst/test_train_pst.py) | 11 |
| 合計 | 636 |

`scripts/` 内に独立テストはなく、Rustの文書内テストも0件だった。
別リポジトリを指す `minase-gui` のシンボリックリンク先と、仮想環境・依存ライブラリ・生成物は対象外とした。
