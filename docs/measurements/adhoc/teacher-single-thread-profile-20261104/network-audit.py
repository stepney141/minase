#!/usr/bin/env python3
"""Read the two measured MNUE files using their documented version 5 layouts."""

import hashlib
import json
from pathlib import Path
import struct


BASE = Path(__file__).resolve().parent
BASELINE = json.loads((BASE / "results/baseline-summary.json").read_text())


def audit(name, width, head_name, head_layout):
    path = BASE / f"source-{name}/crates/minase/nets/nnue.bin"
    data = path.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest != BASELINE[name]["network_sha256"]:
        raise ValueError(f"{path}: file SHA-256 differs from the measured network")
    if data[:8] != b"MNUE\x05\x00\x00\x00":
        raise ValueError(f"{path}: expected MNUE version 5")
    architecture = data[8:104].split(b"\x00", 1)[0].decode("ascii")
    expected = f"features=P;ft={width};linear=tapered2;head={head_name}"
    if architecture != expected or struct.unpack_from("<I", data, 104)[0] != width:
        raise ValueError(f"{path}: architecture differs from the audited layout")
    if hashlib.sha256(data[192:]).digest() != data[144:176]:
        raise ValueError(f"{path}: body SHA-256 differs from the header")

    # See eval/nnue/format.rs:8-19 and the corresponding head.rs byte layout.
    layout = [
        ("linear", "h", 13_680 * 2),
        ("piece_values", "i", 47),
        ("input_weights", "h", 13_680 * width),
        ("input_bias", "i", width),
    ] + head_layout
    sections = {}
    offset = 192
    for label, kind, count in layout:
        element_format = "<" + kind
        end = offset + count * struct.calcsize(element_format)
        if end > len(data):
            raise ValueError(f"{path}: truncated {label}")
        values = struct.iter_unpack(element_format, memoryview(data)[offset:end])
        zeros = sum(value == 0 for (value,) in values)
        sections[label] = {
            "offset": offset,
            "elements": count,
            "zero_elements": zeros,
            "nonzero_elements": count - zeros,
        }
        offset = end
    if offset != len(data):
        raise ValueError(f"{path}: trailing bytes after the audited layout")
    nonlinear = [sections[label] for label, _, _ in layout[2:]]
    return {
        "path": str(path.relative_to(BASE)),
        "revision": BASELINE[name]["revision"],
        "sha256": digest,
        "body_sha256_verified": True,
        "architecture": architecture,
        "file_bytes": len(data),
        "nonlinear_all_zero": all(s["nonzero_elements"] == 0 for s in nonlinear),
        "sections": sections,
    }


if __name__ == "__main__":
    results = {
        "old": audit("old", 128, "direct-screlu", [
            ("head_output_weights", "h", 256),
            ("head_output_bias", "i", 1),
        ]),
        "gen0": audit("gen0", 256, "pairwise-l16-l32", [
            ("head_o", "h", 256),
            ("head_b", "i", 1),
            ("head_W1", "h", 15 * 256),
            ("head_b1", "i", 15),
            ("head_W2", "h", 32 * 30),
            ("head_b2", "i", 32),
            ("head_v", "h", 32),
        ]),
    }
    print(json.dumps(results, indent=2))
