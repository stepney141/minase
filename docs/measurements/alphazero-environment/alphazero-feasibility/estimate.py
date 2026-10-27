"""設計書の「一次見積もりの式」で、GPU種別ごとに1サイクルの所要時間とCUを求める。

使い方: estimate.py <入力のJSON>
入力は機械ごとの実測値をまとめたJSONで、欄の意味は MACHINE_FIELDS にある。
"""

import json
import sys

POSITIONS_PER_CYCLE = 1_000_000
WINDOW_CYCLES = 4
PROGRESS_SIMULATIONS_PER_MOVE = 800
PROGRESS_GAMES = 400
PROGRESS_INTERVAL_CYCLES = 10

MACHINE_FIELDS = {
    "gpu_evals_per_sec": "10ブロック128チャネルのFP16の毎秒の評価数（ストリームの数とバッチの大きさで最大の値）",
    "cpu_simulations_per_sec": "全vCPUで並列に測った毎秒のシミュレーション数の合計",
    "train_positions_per_sec": "10ブロック128チャネルのbf16の学習の毎秒の局面数（バッチの大きさで最大の値）",
    "session_secs": "colab new の所要秒数",
    "setup_secs": "新しい機械での setup.sh の所要秒数",
    "build_secs": "minase のリリースビルドの所要秒数",
    "engine_secs": "TensorRTのエンジンの作成を含むセッション作成の秒数",
    "cu_per_hour": "colab usage の消費レート",
}


def estimate(machine: dict, common: dict) -> dict:
    rate = min(machine["gpu_evals_per_sec"], machine["cpu_simulations_per_sec"])
    # 1局面あたりのネットの評価は、根の展開の1回と1手n回の探索である（木の再利用とNNキャッシュは数えない）。
    evals_per_position = common["simulations_per_move"] + 1
    generation = POSITIONS_PER_CYCLE * evals_per_position / rate
    training = WINDOW_CYCLES * POSITIONS_PER_CYCLE / machine["train_positions_per_sec"]
    progress_evals = PROGRESS_GAMES * common["mean_game_plies"] * PROGRESS_SIMULATIONS_PER_MOVE
    progress = progress_evals / rate
    cycle_bytes = POSITIONS_PER_CYCLE * common["bytes_per_position"]
    # 送るのはソースの書庫、学習窓のデータ、最後のチェックポイント、生成に使うネット、および進捗測定で比べるネット。
    upload = (
        common["source_bytes"]
        + WINDOW_CYCLES * cycle_bytes
        + 2 * common["net_bytes"]
        + common["checkpoint_bytes"]
    ) / common["upload_bytes_per_sec"]
    download = (cycle_bytes + common["net_bytes"] + common["checkpoint_bytes"]) / common["download_bytes_per_sec"]
    fixed = machine["session_secs"] + machine["setup_secs"] + machine["build_secs"] + machine["engine_secs"] + upload + download
    cycle = generation + training + progress / PROGRESS_INTERVAL_CYCLES + fixed
    return {
        "bound": "gpu" if machine["gpu_evals_per_sec"] <= machine["cpu_simulations_per_sec"] else "cpu",
        "positions_per_day": round(rate * 86_400 / evals_per_position),
        "generation_secs": round(generation),
        "training_secs": round(training),
        "progress_secs": round(progress),
        "fixed_secs": round(fixed),
        "transfer_secs": round(upload + download),
        "cycle_secs": round(cycle),
        "cycle_without_progress_secs": round(generation + training + fixed),
        "cycle_with_progress_secs": round(generation + training + progress + fixed),
        "cu_per_cycle": round(cycle / 3600 * machine["cu_per_hour"], 2),
    }


def main() -> None:
    data = json.load(open(sys.argv[1]))
    result = {name: estimate(machine, data["common"]) for name, machine in data["machines"].items()}
    print(json.dumps(result, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
