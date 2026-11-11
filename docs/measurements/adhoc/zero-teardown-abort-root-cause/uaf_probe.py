"""Run inside gdb (gdb -batch -x uaf_probe.py --args <command>); probe.py does this.

Reads the control block of the CUDA provider's static kernel registry at two points: when the
process enters exit(), and when the function that resets the registry is entered. If the reset
function is entered after exit() began, the registry's static destructor has already freed the
control block, and the lines printed at that point show what the freed memory holds.

Every line this script prints starts with "PROBE". probe.py reads these lines to judge the run:

- "PROBE at exit: registry alive" means gdb stopped at the entry of exit(), the provider library
  was loaded, and the control block held one strong and one weak reference.
- "PROBE reset function reached after exit began" or "PROBE reset function not reached".
- "PROBE exit code N" or "PROBE signal NAME" says how the process ended.

PROBE_FUNCTION and PROBE_STATIC are the virtual addresses, inside
libonnxruntime_providers_cuda.so, that find_registry_reset.py prints.
"""

import os
import struct

import gdb

FUNCTION = int(os.environ["PROBE_FUNCTION"], 16)
STATIC = int(os.environ["PROBE_STATIC"], 16)
ALIVE = 0x0000000100000001  # use_count == 1 and weak_count == 1


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


def control_block(label: str) -> int | None:
    """Print the static and its control block; return the two reference counts as one word."""
    pointer, control = read(base() + STATIC, 2)
    print(f"PROBE {label}: static ptr={pointer:#x} ctrl={control:#x}")
    if not control:
        return None
    vptr, counts = read(control, 2)
    print(
        f"PROBE {label}: control block vptr={vptr:#x} "
        f"use={counts & 0xFFFFFFFF} weak={counts >> 32} raw={counts:#x}"
    )
    return counts


def exited() -> bool:
    return gdb.selected_inferior().pid == 0


def report_end() -> None:
    code = gdb.parse_and_eval("$_exitcode")
    if code.type.code != gdb.TYPE_CODE_VOID:
        print(f"PROBE exit code {int(code)}")
    else:
        print("PROBE ended without an exit code")


def stopped_by_signal() -> bool:
    """Report a stop that is not a breakpoint (a fatal signal) and end the process."""
    if int(gdb.parse_and_eval("$pc")) == base() + FUNCTION:
        return False
    signal = gdb.parse_and_eval("$_siginfo.si_signo")
    print(f"PROBE signal {int(signal)}")
    gdb.execute("bt 16")
    gdb.execute("kill")
    return True


gdb.execute("set pagination off")
gdb.execute("set breakpoint pending on")
gdb.execute("set print thread-events off")
gdb.execute("break exit")
gdb.execute("run")
if exited():
    raise RuntimeError("the process ended without calling exit()")
if gdb.selected_frame().name() not in ("exit", "__GI_exit"):
    raise RuntimeError(f"stopped in {gdb.selected_frame().name()}, not at the entry of exit()")
if control_block("at exit") == ALIVE:
    print("PROBE at exit: registry alive")
gdb.execute("delete")
gdb.execute(f"break *{base() + FUNCTION:#x}")
gdb.execute("continue")
if exited():
    print("PROBE reset function not reached")
    report_end()
elif not stopped_by_signal():
    print("PROBE reset function reached after exit began")
    control_block("at reset function")
    gdb.execute("bt 16")
    gdb.execute("delete")
    gdb.execute("continue")
    if exited():
        report_end()
    else:
        stopped_by_signal()
