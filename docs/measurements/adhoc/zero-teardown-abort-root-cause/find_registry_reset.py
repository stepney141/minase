"""Locate the function that resets the CUDA provider's static kernel registry.

usage: find_registry_reset.py <libonnxruntime_providers_cuda.so> [...]

The CUDA execution provider keeps its kernel registry in a static std::shared_ptr and resets it
from the provider's Shutdown(). The shared libraries are stripped, so this script finds the
function by the shape of its machine code: it loads the control block pointer of a static
shared_ptr, zeroes the 16 bytes of the static, and releases the control block. For each match it
prints the virtual addresses of the function and of the static, which uaf_probe.py takes as
PROBE_FUNCTION and PROBE_STATIC.
"""

import re
import sys
from pathlib import Path

PATTERN = re.compile(
    rb"\xf3\x0f\x1e\xfa\x53"  # endbr64; push rbx
    rb"\x48\x8b\x1d(....)"  # mov rbx, [rip + control block]
    rb"\x66\x0f\xef\xc0"  # pxor xmm0, xmm0
    rb"\x0f\x29\x05(....)"  # movaps [rip + static], xmm0
    rb"\x48\x85\xdb\x74.",  # test rbx, rbx; je
    re.DOTALL,
)


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for name in sys.argv[1:]:
        data = Path(name).read_bytes()
        matches = list(PATTERN.finditer(data))
        print(f"{name}: {len(matches)} match(es)")
        for match in matches:
            # In these libraries the text segment is mapped at its file offset, so the offset of a
            # match is also its virtual address, and RIP-relative operands resolve from it.
            start = match.start()
            control = start + 12 + int.from_bytes(match.group(1), "little", signed=True)
            static = start + 23 + int.from_bytes(match.group(2), "little", signed=True)
            if control != static + 8:
                sys.exit(f"{name}: match at {start:#x} is not a shared_ptr reset")
            print(f"  PROBE_FUNCTION={start:#x} PROBE_STATIC={static:#x}")


if __name__ == "__main__":
    main()
