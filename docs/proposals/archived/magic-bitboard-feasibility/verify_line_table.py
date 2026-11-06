#!/usr/bin/env python3
"""Verify the proposed shared 24-KiB line table, without using minase code.

Behavior comes from RULES.md article 7.4-5: a slider stops at the first
occupied square and includes that square (color filtering occurs later).
The finite board bounds stop the walk. The source square is not attacked.

Coverage: line lengths 1..12, all contiguous placements in 12 coordinates,
every source in that line, and every occupancy of its length squares.
This includes occupied/empty sources and endpoints, adjacent blockers,
multiple blockers, and empty/full lines. Table output is clipped to the
actual line, which is necessary for diagonals shorter than 12 squares.

The table uses the interior ten coordinates as occupancy index. It retains
the irrelevant source bit when the source is interior, allowing all sources
to share a fixed 1024-entry stride. Endpoints do not affect the attack set.
"""
import json

WIDTH = 12
INTERIOR_KEY_MASK = (1 << (WIDTH - 2)) - 1


def rule_walk(lo, length, source, occupied):
    """Independent rule oracle: visit squares, include blocker, then stop."""
    attacked = 0
    for step in (-1, 1):
        square = source + step
        while lo <= square < lo + length:
            attacked |= 1 << square
            if occupied & (1 << square):
                break
            square += step
    return attacked


def table_candidate(source, key):
    """Build candidate via nearest set bits and an integer interval mask."""
    interior_occupied = key << 1
    lower = interior_occupied & ((1 << source) - 1)
    upper = interior_occupied >> (source + 1)
    left_edge = lower.bit_length() - 1 if lower else 0
    right_edge = source + (upper & -upper).bit_length() if upper else WIDTH - 1
    interval = ((1 << (right_edge + 1)) - 1) ^ ((1 << left_edge) - 1)
    return interval & ~(1 << source)


table = [
    [table_candidate(source, key) for key in range(1 << (WIDTH - 2))]
    for source in range(WIDTH)
]

cases = 0
by_length = {}
mutation_counterexamples = {}
for length in range(1, WIDTH + 1):
    by_length[length] = 0
    for lo in range(WIDTH - length + 1):
        valid_line = ((1 << length) - 1) << lo
        for source in range(lo, lo + length):
            for local_occupied in range(1 << length):
                occupied = local_occupied << lo
                key = (occupied >> 1) & INTERIOR_KEY_MASK
                actual = table[source][key] & valid_line
                expected = rule_walk(lo, length, source, occupied)
                assert actual == expected, {
                    'length':length, 'lo':lo, 'source':source,
                    'occupied':occupied, 'actual':actual, 'expected':expected,
                }
                mutants = {
                    'drop_the_first_blocker':actual & ~occupied,
                    'include_the_source':actual | (1 << source),
                    'omit_short_line_clipping':table[source][key],
                    'ignore_all_blockers':table[source][0] & valid_line,
                }
                for name, mutant in mutants.items():
                    if mutant != expected and name not in mutation_counterexamples:
                        mutation_counterexamples[name] = {
                            'length':length, 'lo':lo, 'source':source,
                            'occupied':occupied,
                        }
                cases += 1
                by_length[length] += 1

assert len(mutation_counterexamples) == 4
print(json.dumps({
    'verified_cases':cases,
    'cases_by_line_length':by_length,
    'entries':WIDTH*(1 << (WIDTH-2)),
    'u16_payload_bytes':2*WIDTH*(1 << (WIDTH-2)),
    'rule_oracle':'RULES.md article 7.4-5, bounded square-by-square walk',
    'semantic_mutations_rejected':mutation_counterexamples,
    'SPEC_UNCLEAR':[],
    'remaining_implementation_coupled_tests':[],
    'limitations':'Mathematical line-table candidate only; no Rust/BMI2 integration or timing.',
},indent=2))
