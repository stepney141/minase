#!/usr/bin/env bash
# 学習の再開の確認：機械Bで止めた学習（phase1/train-b）を、NASから送ったチェックポイントと
# 同じ相対パスの学習窓だけから再開し、続けて同じ設定の止めない学習（phase1/train-a）を行う。
# 使い方: train-c.sh <学習器のコミット>
set -euo pipefail
commit=$1
cd /content/az
data=$(python3 -c "import json; print(' '.join('phase1/gen/' + f for f in json.load(open('phase1/gen/summary.json'))['training_files']))")
bash /content/train.sh train --resume phase1/train-b --device cuda > phase1/train-b-run2.log 2>&1
bash /content/train.sh train --init cycles/cycle-0000/checkpoint.pt --data $data --out-dir phase1/train-a \
    --cycle 1 --seed 1001 --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 \
    --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 200 --device cuda \
    --trainer-commit "$commit" > phase1/train-a.log 2>&1
echo train-c-done
