"""MNSDの盤面の符号から駒の損得を求める表。

符号の駒種は成った後の駒種であり、成りの印は同じ駒種の価値を変えない。
"""

import numpy as np

from common import VALUES

KINDS = ("Pawn GoBetween Lance ReverseChariot SideMover VerticalMover Bishop Rook DragonHorse "
         "DragonKing FreeKing King DrunkElephant FerociousLeopard BlindTiger CopperGeneral "
         "SilverGeneral GoldGeneral Kirin Phoenix Lion CrownPrince WhiteHorse Whale FlyingOx "
         "FreeBoar FlyingStag HornedFalcon SoaringEagle").split()
TABLE = np.zeros(128, dtype=np.int32)
for code in range(128):
    if 1 <= code <= 58:
        payload, sign = code - 1, 1
    elif 65 <= code <= 122:
        payload, sign = code - 65, -1
    else:
        continue
    TABLE[code] = sign * VALUES[KINDS[payload % 29]]
