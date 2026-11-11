"""Run inside gdb (gdb -batch -x uaf_probe.py --args <command>); probe.py does this.

Reads the control block of the CUDA provider's static kernel registry at two points: when the
process enters exit(), and when the function that resets the registry is entered. If the reset
function is entered after exit() began, the registry's static destructor has already freed the
control block, and the lines printed at that point show what the freed memory holds.

PROBE_FUNCTION and PROBE_STATIC are the virtual addresses, inside
libonnxruntime_providers_cuda.so, that find_registry_reset.py prints.
"""

import os
import struct

import gdb

FUNCTION = int(os.environ["PROBE_FUNCTION"], 16)
STATIC = int(os.environ["PROBE_STATIC"], 16)


def base() -> int:
    pid = gdb.selected_inferior().pid
    with open(f"/proc/{pid}/maps") as maps:
        for line in maps:
            if "libonnxruntime_providers_cuda.so" in line:
                return int(line.split("-")[0], 16)
    raise RuntimeError("libonnxruntime_providers_cuda.so is not loaded")


def read(address: int, count: int) -> tuple[int, ...]:
    data = gdb.selected_inferior().read_memory(address, 8 * count).tobytes()
    return struct.unpack(f"<{count}Q", data)


def report(label: str) -> None:
    pointer, control = read(base() + STATIC, 2)
    print(f"PROBE {label}: static ptr={pointer:#x} ctrl={control:#x}")
    if control:
        vptr, counts = read(control, 2)
        print(
            f"PROBE {label}: control block vptr={vptr:#x} "
            f"use={counts & 0xFFFFFFFF} weak={counts >> 32} raw={counts:#x}"
        )


def exited() -> bool:
    return gdb.selected_inferior().pid == 0


gdb.execute("set pagination off")
gdb.execute("set breakpoint pending on")
gdb.execute("set print thread-events off")
gdb.execute("break exit")
gdb.execute("run")
if exited():
    raise RuntimeError("the process ended without calling exit()")
report("at exit")
gdb.execute(f"break *{base() + FUNCTION:#x}")
gdb.execute("continue")
if exited():
    print("PROBE reset function not reached after exit began")
else:
    report("at reset function")
    gdb.execute("bt 16")
    gdb.execute("delete")
    gdb.execute("continue")
