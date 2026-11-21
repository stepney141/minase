# 補助スクリプトの構成

`scripts/` 直下には本書だけを置き、すべてのスクリプトは役割ごとの下位ディレクトリへ分類する。既存の分類に収まらない継続運用のスクリプトが必要になったときは、下位ディレクトリを追加してから本書の分類表へ行を加える。

特定の監査や測定だけで用いる一回限りの再現スクリプトは、`scripts/` ではなく対応する `docs/audits/<監査名>/` または `docs/measurements/<設計書名>/<測定名>/` に同梱する。これにより、`scripts/` には開発と運用で繰り返し使う道具だけが残る。

## ディレクトリ一覧

| ディレクトリ | 役割 | 主なファイル |
|---|---|---|
| [bench/](bench/) | USI の `bench` コマンドを用いた回帰照合と、複数コミット間の探索速度比較を行う。 | `bench_compare.py`、`test_bench_compare.py` |
| [hachu/](hachu/) | 中将棋エンジン HaChu と minase の通信プロトコル（CECP）および規則の互換性を照合する。 | `hachu_selfplay.py`、`hachu_replay.py` |
| [lishogi/](lishogi/) | オンライン対局サイト Lishogi から中将棋棋譜を取得し、学習・検証用の選別や回帰テスト用フィクスチャを生成する。 | `fetch_lishogi_games.py`、`select_validation_games.py`、`fetch_lishogi_replays.py`、`test_fetch_lishogi_games.py`、`test_select_validation_games.py` |
| [match/](match/) | `minase match run` の対局記録や USI 自己対局から、時間消費、対局コスト、および投了条件を診断する。 | `clock_budget_stats.py`、`clock_profile.py`、`byoyomi_game_profile.py`、`match_cost_profile.py`、`resign_diagnosis.py` |
| [release/](release/) | リリースタグの整合、配布パッケージに含まれる評価ネット一覧、および変更履歴の抽出を検証する。 | `release_checks.py`、`test_release_checks.py` |
| [vast/common/](vast/common/) | 計算機市場 vast.ai のインスタンス作成、照会、ファイル転送、停止、および削除の共通処理を提供する。 | `common.py`、`onstart.sh`、`test_common.py` |
| [vast/nnue/](vast/nnue/) | vast.ai の計算機で NNUE の教師データ生成、回収、および世代反復の学習を制御する。 | `generation_chain.py`、`generation_worker.py`、`teacher_generation.py`、`teacher_generation_probe.py`、`test_generation_chain.py`、`test_teacher_generation.py` |
| [vast/spsa/](vast/spsa/) | vast.ai の計算機で探索および時間管理係数の同時摂動確率近似（SPSA）調整を実行し、結果を回収する。 | `launch.py`、`remote.py`、`fetch.py`、`bench.py`、`sync_run.py`、`test_spsa.py` |

## テストの実行

単体テストを持つスクリプトは GitHub Actions（`.github/workflows/ci.yml`）で自動実行される。各スクリプトを変更したときは、リポジトリのルートから次の該当テストを実行して契約の維持を確認する。

```bash
python3 scripts/bench/test_bench_compare.py
python3 scripts/lishogi/test_fetch_lishogi_games.py
python3 scripts/lishogi/test_select_validation_games.py
python3 scripts/release/test_release_checks.py
python3 scripts/vast/common/test_common.py
python3 scripts/vast/nnue/test_teacher_generation.py
python3 scripts/vast/nnue/test_generation_chain.py
python3 scripts/vast/spsa/test_spsa.py
```
