"""準備測定で共有する固定パスと来歴の記録。"""
from pathlib import Path
import hashlib
import json
import sys

# 読み取り専用の入力側へ __pycache__ を作らない。
sys.dont_write_bytecode = True
WT = Path(__file__).resolve().parents[4]
DATA = Path('/home/stepney141/board-games/minase/data')
BASE = DATA / 'relational-correction'
OUTPUT = BASE / 'prep'


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def identity(path):
    return {'path': str(Path(path).resolve()), 'sha256': sha256(path)}


def write_json(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write('\n')


def invocation():
    return {'cwd': str(Path.cwd()), 'argv': [sys.executable, '-B', *sys.argv]}
