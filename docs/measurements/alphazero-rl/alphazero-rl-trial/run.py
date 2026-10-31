# colab exec で送り、CMD のシェルのコマンドを実行して出力と終了コードを表示する。
import os
import subprocess

result = subprocess.run(["bash", "-c", os.environ["CMD"]], capture_output=True, text=True)
print(result.stdout, end="")
print(result.stderr, end="")
print(f"exit {result.returncode}")
