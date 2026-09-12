監査対象のテストは43ファイルに636件あり、全件の本文を確認した。
件数は対象範囲を照合するためのものであり、維持目標ではない。
Rustの `#[test]` とPythonのunittestテスト関数を追跡ファイルから抽出し、全体の実行結果と突き合わせた。

| ファイル | 件数 |
|---|---:|
| [src/bin/bench.rs](/home/stepney141/board-games/minase/src/bin/bench.rs) | 5 |
| [src/bin/match_report.rs](/home/stepney141/board-games/minase/src/bin/match_report.rs) | 4 |
| [src/bin/match_runner.rs](/home/stepney141/board-games/minase/src/bin/match_runner.rs) | 47 |
| [src/bin/match_runner/storage.rs](/home/stepney141/board-games/minase/src/bin/match_runner/storage.rs) | 12 |
| [src/bin/minase.rs](/home/stepney141/board-games/minase/src/bin/minase.rs) | 2 |
| [src/bin/random_play.rs](/home/stepney141/board-games/minase/src/bin/random_play.rs) | 5 |
| [src/bin/selfplay_gen.rs](/home/stepney141/board-games/minase/src/bin/selfplay_gen.rs) | 11 |
| [src/bin/usi_random.rs](/home/stepney141/board-games/minase/src/bin/usi_random.rs) | 5 |
| [src/core/adjudication.rs](/home/stepney141/board-games/minase/src/core/adjudication.rs) | 3 |
| [src/core/attacks/fixed.rs](/home/stepney141/board-games/minase/src/core/attacks/fixed.rs) | 2 |
| [src/core/attacks/tables.rs](/home/stepney141/board-games/minase/src/core/attacks/tables.rs) | 2 |
| [src/core/bitboard.rs](/home/stepney141/board-games/minase/src/core/bitboard.rs) | 3 |
| [src/core/direction.rs](/home/stepney141/board-games/minase/src/core/direction.rs) | 1 |
| [src/core/game.rs](/home/stepney141/board-games/minase/src/core/game.rs) | 54 |
| [src/core/movegen/tests/attackers.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/attackers.rs) | 2 |
| [src/core/movegen/tests/lion_moves.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/lion_moves.rs) | 19 |
| [src/core/movegen/tests/movement.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/movement.rs) | 12 |
| [src/core/movegen/tests/pieces.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/pieces.rs) | 30 |
| [src/core/movegen/tests/promotion.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/promotion.rs) | 37 |
| [src/core/movegen/tests/properties.rs](/home/stepney141/board-games/minase/src/core/movegen/tests/properties.rs) | 6 |
| [src/core/piece.rs](/home/stepney141/board-games/minase/src/core/piece.rs) | 4 |
| [src/core/position.rs](/home/stepney141/board-games/minase/src/core/position.rs) | 22 |
| [src/core/repetition.rs](/home/stepney141/board-games/minase/src/core/repetition.rs) | 1 |
| [src/core/rules.rs](/home/stepney141/board-games/minase/src/core/rules.rs) | 48 |
| [src/core/square.rs](/home/stepney141/board-games/minase/src/core/square.rs) | 3 |
| [src/eval/features.rs](/home/stepney141/board-games/minase/src/eval/features.rs) | 3 |
| [src/eval/handcrafted.rs](/home/stepney141/board-games/minase/src/eval/handcrafted.rs) | 4 |
| [src/eval/pst.rs](/home/stepney141/board-games/minase/src/eval/pst.rs) | 22 |
| [src/eval/training_data.rs](/home/stepney141/board-games/minase/src/eval/training_data.rs) | 6 |
| [src/notation/cecp.rs](/home/stepney141/board-games/minase/src/notation/cecp.rs) | 7 |
| [src/notation/sfen.rs](/home/stepney141/board-games/minase/src/notation/sfen.rs) | 17 |
| [src/notation/usi.rs](/home/stepney141/board-games/minase/src/notation/usi.rs) | 10 |
| [src/protocol/cecp.rs](/home/stepney141/board-games/minase/src/protocol/cecp.rs) | 32 |
| [src/protocol/engine.rs](/home/stepney141/board-games/minase/src/protocol/engine.rs) | 7 |
| [src/protocol/usi.rs](/home/stepney141/board-games/minase/src/protocol/usi.rs) | 47 |
| [src/search/see.rs](/home/stepney141/board-games/minase/src/search/see.rs) | 9 |
| [src/search/tests.rs](/home/stepney141/board-games/minase/src/search/tests.rs) | 88 |
| [src/stats.rs](/home/stepney141/board-games/minase/src/stats.rs) | 13 |
| [tests/lishogi_replay.rs](/home/stepney141/board-games/minase/tests/lishogi_replay.rs) | 1 |
| [tests/match_runner.rs](/home/stepney141/board-games/minase/tests/match_runner.rs) | 4 |
| [tools/train/pst/test_pst_diagnostics.py](/home/stepney141/board-games/minase/tools/train/pst/test_pst_diagnostics.py) | 5 |
| [tools/train/pst/test_pst_workflow.py](/home/stepney141/board-games/minase/tools/train/pst/test_pst_workflow.py) | 10 |
| [tools/train/pst/test_train_pst.py](/home/stepney141/board-games/minase/tools/train/pst/test_train_pst.py) | 11 |
| 合計 | 636 |

`scripts/` 内に独立テストはなく、Rustの文書内テストも0件だった。
別リポジトリを指す `minase-gui` のシンボリックリンク先と、仮想環境・依存ライブラリ・生成物は対象外とした。
