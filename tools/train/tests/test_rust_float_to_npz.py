"""Rustチェックポイントの仕様から組んだバイト列で診断用npzの契約を検査する。"""

import hashlib
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

import numpy as np

from helpers import records_with, write_mnsd
from minase_train.data.mnpt import initial_piece_values
from minase_train.diagnostics.nnue import removal
from minase_train.nnue.format import write_mnue
from minase_train.nnue.train import quantized_weights


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "rust_float_to_npz.py"


def checkpoint_bytes(feature_set: str, width: int) -> bytes:
    """nnue-trainer/README.mdの版2の配置を、Rustや変換側の読み手を使わず構築する。"""
    shared = feature_set == "P"
    rows, virtual_rows = (6841, 0) if shared else (109441, 13681)
    units = width // 2 if shared else width
    linear = np.zeros((6841, 2), dtype="<f4")
    # 正準行0、5、次の段、次の駒種、および末尾の先獅子行。
    for row, value in ((0, 1.25), (5, 2.5), (6, 3.75), (72, 5), (6839, 6.25)):
        linear[row] = [value, -2 * value]
    embedding = np.zeros((rows, width), dtype="<f4")
    virtual = np.zeros((virtual_rows, width), dtype="<f4")
    lanes = np.arange(1, width + 1, dtype="<f4") / 32
    if shared:
        for row, multiplier in ((0, 1), (5, 2), (6, 3), (72, 4), (6839, 5)):
            embedding[row] = multiplier * lanes
    else:
        # 異なる区分の同じ基底行は、同じ仮想行を足す。
        embedding[0], embedding[13680], embedding[109439] = lanes, 2 * lanes, 3 * lanes
        virtual[0], virtual[13679] = 4 * lanes, -lanes
    parameters = [linear, embedding, virtual,
                  np.arange(1, units + 1, dtype="<f4") / 16,
                  np.stack((np.arange(1, units + 1, dtype="<f4"),
                            -np.arange(1, units + 1, dtype="<f4"))) / 8,
                  np.array(0.375, dtype="<f4")]
    architecture = f"features={feature_set};ft={width};linear=tapered2;head=direct-screlu"
    raw = bytearray(struct.pack("<8sI96sIfQQQddQ", b"MNTRCKPT", 2, architecture.encode(),
                                width, 512, 3, 1, 1, 0.5, 1, 7))
    raw.extend(b"c" * 32 + b"i" * 32)
    raw.extend(struct.pack("<8d", 0.03, 0, 0.001, 0.0001, 0.001, 0.0001, 0, 0.5))
    for values in (parameters, [np.zeros_like(p) for p in parameters],
                   [np.zeros_like(p) for p in parameters]):
        for value in values:
            raw.extend(value.tobytes())
    for count in (6841, rows, virtual_rows):
        if count:
            mask = np.ones(count, dtype=np.uint8)
            mask[-1] = 0
            if count == 6841:
                mask[2] = 0
            raw.extend(mask.tobytes())
    raw.extend(np.zeros_like(linear).tobytes())
    raw.extend(np.zeros_like(virtual).tobytes())
    return bytes(raw) + hashlib.sha256(raw).digest()


class RustFloatToNpzTest(unittest.TestCase):
    def test_conversion_expands_parameters_and_is_accepted_by_removal(self):
        """指示書の形・単精度・鏡映回転・仮想行の和を検査し、removalを実行する。"""
        for feature_set, width in (("P", 2), ("P", 4), ("HalfKP-hm-b8", 4)):
            with self.subTest(feature_set=feature_set, width=width), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source, output = root / "nnue-float.bin", root / "converted.npz"
                source.write_bytes(checkpoint_bytes(feature_set, width))
                result = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source),
                                         "--output", str(output)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                with np.load(output, allow_pickle=False) as archive:
                    weights = dict(archive)
                count = 13680 if feature_set == "P" else 109440
                shapes = {"middlegame": (13680,), "endgame": (13680,),
                          "weights": (count, width), "biases": (width,),
                          "output_weights": (2, width), "output_bias": ()}
                self.assertEqual(set(weights), set(shapes) | {"mask"})
                for name, shape in shapes.items():
                    self.assertEqual(weights[name].shape, shape, name)
                    self.assertEqual(weights[name].dtype, np.dtype(np.float32), name)
                    self.assertTrue(np.isfinite(weights[name]).all(), name)
                self.assertEqual(weights["mask"].shape, (13680,))
                self.assertEqual(weights["mask"].dtype, np.dtype(bool))
                self.assertFalse(weights["mask"][2])
                self.assertFalse(weights["mask"][9])
                self.assertTrue(weights["mask"][0])
                for features, value in (((0, 11), 1.25), ((5, 6), 2.5), ((12, 23), 3.75),
                                        ((144, 155), 5), ((13673, 13674), 6.25)):
                    for feature in features:
                        self.assertEqual(weights["middlegame"][feature], value)
                        self.assertEqual(weights["endgame"][feature], -2 * value)
                self.assertEqual(weights["output_bias"], 0.375)
                lanes = np.arange(1, width + 1, dtype=np.float32) / 32
                if feature_set == "P":
                    # 回転の向きと複製の順序は、幅4の手計算で固定する。
                    reflected = np.array([3, 4, 1, 2] if width == 4 else [2, 1], dtype=np.float32) / 32
                    for features, multiplier in (((0, 11), 1), ((5, 6), 2), ((12, 23), 3),
                                                  ((144, 155), 4), ((13673, 13674), 5)):
                        np.testing.assert_array_equal(weights["weights"][features[0]], multiplier * lanes)
                        np.testing.assert_array_equal(weights["weights"][features[1]], multiplier * reflected)
                    units = np.array([1, 2, 1, 2] if width == 4 else [1, 1], dtype=np.float32)
                else:
                    for feature, multiplier in ((0, 5), (13680, 6), (27360, 4), (13679, -1), (109439, 2)):
                        np.testing.assert_array_equal(weights["weights"][feature], multiplier * lanes)
                    units = np.array([1, 2, 3, 4], dtype=np.float32)
                np.testing.assert_array_equal(weights["biases"], units / 16)
                np.testing.assert_array_equal(weights["output_weights"], np.stack((units, -units)) / 8)
                # 小さな実データ形式の局面で、診断の読み込みから独立再計算まで通す。
                data, net = root / "positions.bin", root / "nnue.bin"
                records = records_with([[(0, 12), (143, 76), (12, 1)]])
                write_mnsd(data, seed=0, checksum=b"a" * 32, games=[0], board=records["board"])
                linear, hidden, biases, head, bias = quantized_weights(weights)
                write_mnue(net, linear, initial_piece_values(), 512, hidden, biases, head, bias,
                           feature_set=feature_set)
                report = removal(output, net, [data], sample=1, seed=7)
                self.assertEqual(report["removal_count"], 1)
                self.assertTrue(report["float_batch_vs_recomputed"]["passed"])
                self.assertNotIn("error", report)

    def test_invalid_checkpoint_is_rejected_without_output(self):
        """共通の読み手が検査するSHA-256を、変換コマンドでも必須にする。"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / "broken.bin", root / "converted.npz"
            damaged = bytearray(checkpoint_bytes("P", 2))
            damaged[-1] ^= 1
            source.write_bytes(damaged)
            result = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source),
                                     "--output", str(output)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA-256", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
