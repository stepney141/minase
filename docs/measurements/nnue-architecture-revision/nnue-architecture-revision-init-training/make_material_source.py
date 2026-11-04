#!/usr/bin/env python3
"""ネットの線形の経路を、学習の初期値（探索用駒価値）に置き換えた元ファイルを作る。

make_epoch0.pyの入力に使う。自駒はその駒状態の駒価値、相手駒は符号を反転した値を144升と2列に置き、
王駒（駒状態11と21）と先獅子の対象升の行は0とする。単位は1/8 cpである。

使い方: make_material_source.py <入力のnnue.bin> <出力>
"""
import struct
import sys
from pathlib import Path

HEADER = 192
FEATURES = 13_680
STATES = 47
ROYAL = {11, 21}

source = Path(sys.argv[1]).read_bytes()
offset = HEADER + FEATURES * 4
values = struct.unpack_from(f"<{STATES}i", source, offset)
rows = []
for color in range(2):
    for state in range(STATES):
        value = 0 if state in ROYAL else values[state] * 8 * (1 if color == 0 else -1)
        rows += [(value, value)] * 144
rows += [(0, 0)] * 144
assert len(rows) == FEATURES
body = b"".join(struct.pack("<hh", *row) for row in rows)
Path(sys.argv[2]).write_bytes(source[:HEADER] + body + source[offset:])
