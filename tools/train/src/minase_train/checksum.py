"""ファイル全体のSHA-256を計算する。"""

import hashlib
from pathlib import Path


def sha256_file(path: Path) -> bytes:
    """MNSDヘッダを含むファイル全体を一定サイズのバッファでハッシュする。"""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1 << 20):
            digest.update(chunk)
    return digest.digest()
