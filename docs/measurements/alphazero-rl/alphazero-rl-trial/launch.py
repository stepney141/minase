# colab exec で送り、JOB_ARGS（JSONの引数の列）をカーネルとは別のプロセスグループで起動し、
# 標準出力と標準エラーを JOB_LOG へ書かせる。起動したプロセスの番号を表示する。
import json
import os
import subprocess

log = open(os.environ["JOB_LOG"], "w")
process = subprocess.Popen(json.loads(os.environ["JOB_ARGS"]), stdout=log, stderr=subprocess.STDOUT,
                           start_new_session=True)
print(process.pid)
