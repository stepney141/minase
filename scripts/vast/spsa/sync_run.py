#!/usr/bin/env python3
"""JSONと適用番号を検査して再開用の写しを入れ替える。

使い方: from sync_run import sync_run, check_runで関数を読み込む。
`run/`は`incoming/`の各ファイルへのハードリンクで構成し、内容を複製しない。
"""

import ctypes
import json
import os
from pathlib import Path
import shutil
import tempfile


class IncompleteRun(ValueError):
    """取得した記録が再開に必要な条件を満たしていないことを表す。"""


def read_run(root: Path) -> tuple[list[Path], int]:
    """再開に必要なファイルを読み、適用番号が1から連続することを確かめる。"""
    files = [root / "manifest.json", root / "invocations.json"]
    if not (root / "iterations").is_dir():
        raise IncompleteRun("iterations/がありません")
    files.extend(sorted((root / "iterations").glob("*.json")))
    documents = []
    for path in files:
        if path.is_symlink() or not path.is_file():
            raise IncompleteRun(f"通常のファイルではありません: {path}")
        try:
            documents.append(json.loads(path.read_text()))
        except (ValueError, UnicodeError) as error:
            raise IncompleteRun(f"JSONを読めません: {path}: {error}") from error
    manifest, invocations, *records = documents
    if not isinstance(manifest, dict) or not isinstance(invocations, list) or not invocations:
        raise IncompleteRun("manifest.jsonまたはinvocations.jsonの形式が不正です")
    numbers = []
    for path, record in zip(files[2:], records):
        if not isinstance(record, dict) or "application" not in record or type(record["application"]) is not int:
            raise IncompleteRun(f"適用番号が不正です: {path}")
        numbers.append(record["application"])
    if sorted(numbers) != list(range(1, len(numbers) + 1)):
        raise IncompleteRun("適用番号が1から連続していません")
    return files, len(numbers)


def exchange_directories(left: Path, right: Path) -> None:
    """Linuxのrenameat2で、既存のrun/を欠落期間なしに入れ替える。"""
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renameat2
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(-100, os.fsencode(left), -100, os.fsencode(right), 2) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def sync_run(source: Path, destination: Path) -> int:
    """連続した記録だけをハードリンクで複製し、再開用の写しを入れ替える。"""
    files, count = read_run(source)
    if destination.is_symlink():
        raise ValueError("再開用ディレクトリはシンボリックリンクにできません")
    # 取得元は毎回新しいディレクトリであり、ここから先は書き換えない。
    staged = Path(tempfile.mkdtemp(prefix=".run-", dir=destination.parent))
    try:
        (staged / "iterations").mkdir()
        for path in files:
            os.link(path, staged / path.relative_to(source))
        if destination.exists():
            exchange_directories(staged, destination)
        else:
            os.rename(staged, destination)
        return count
    finally:
        if staged.exists():
            shutil.rmtree(staged)


def check_run(root: Path, iterations: int) -> None:
    """必要な反復が欠けずにそろっていることを検査する。"""
    _, count = read_run(root)
    if count != iterations:
        raise IncompleteRun(f"完了反復数は{count}です。必要数は{iterations}です")
