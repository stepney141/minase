#!/usr/bin/env bash
# Colab機で、setup.shの導入、mnsd_readerとminaseのビルド、および初期ネットの出力の照合を順に行い、各段の時刻を出す。
# 使い方: bash colab-check.sh <ソースの書庫> <ネットのONNX> <PyTorchの出力のJSON>
# 最後まで通ると、標準出力の最後の行にcheck-doneを出す。
set -euo pipefail
date -Is
if [[ $# -ne 3 || ! -f "$1" || ! -f "$2" || ! -f "$3" ]]; then
    printf 'ソースの書庫、ネット、PyTorchの出力を指定してください\n' >&2
    exit 1
fi
archive="$1" model="$2" reference="$3"

mkdir -p /content/src /content/check
tar -xf "$archive" -C /content/src
date -Is
bash /content/src/tools/train/zero/setup.sh
date -Is
source /content/src/tools/train/zero/env.sh
/content/src/tools/train/zero/sync-env.sh
date -Is
cd /content/src
cargo build --release --bin minase
date -Is

# 導入先が環境のディレクトリの中にあることを、実行ファイルのパスで示す。
command -v uv cargo rustc
ls -d "$HOME/.cargo" "$HOME/.rustup" "$HOME/.local/share/uv" 2>&1 || true
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv

run() {  # 実行プロバイダ、バッチの大きさ、出力のパス
    {
        printf 'usi\nsetoption name RuleSet value JCSA\nsetoption name NnModel value %s\n' "$model"
        printf 'setoption name NnBackend value %s\nsetoption name NnBatchSize value %s\n' "$1" "$2"
        if [[ "$1" == tensorrt ]]; then
            printf 'setoption name NnCacheDir value %s\n' /content/check/trt-cache
        fi
        printf 'isready\nnn-eval out %s\nquit\n' "$3"
    } | target/release/minase --protocol usi --search mcts | tee "$3.log" | grep -q '^info string end$'
}
for batch in 1 7 256; do
    run cpu "$batch" "/content/check/cpu-b$batch.json"
    run tensorrt "$batch" "/content/check/trt-b$batch.json"
done
date -Is

cd tools/train/zero
for batch in 1 7 256; do
    uv run --frozen python nn_compare.py --reference "$reference" --candidate "/content/check/cpu-b$batch.json" \
        --policy fp16-logits --policy-tolerance 1e-4 --value-tolerance 1e-4
    uv run --frozen python nn_compare.py --reference "$reference" --candidate "/content/check/trt-b$batch.json" \
        --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2
done
date -Is
echo check-done
