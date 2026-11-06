#!/usr/bin/env python3
"""Exact occupancy-index table sizes for a 12x12 board; no magic search.

Each mask omits the final square of each ray because blocking on that square
has the same attack set as no blocker. Each square has its own packed table.
All entry counts include one entry when the relevant mask is empty.
No offset, mask, magic, alignment, or square-mapping metadata is counted.
"""
from collections import Counter
from math import prod
import json

N = 12
DIRECTIONS = [(1,0),(-1,0),(0,1),(0,-1),(1,1),(-1,-1),(-1,1),(1,-1)]
GROUPS = {
    'rook_four_rays': [[0,1,2,3]],
    'bishop_four_rays': [[4,5,6,7]],
    'rook_two_axes': [[0,1],[2,3]],
    'bishop_two_axes': [[4,5],[6,7]],
    'rook_four_separate_rays': [[i] for i in range(4)],
    'bishop_four_separate_rays': [[i] for i in range(4,8)],
}

def ray(f,r,df,dr):
    squares = []
    while 0 <= f+df < N and 0 <= r+dr < N:
        f += df
        r += dr
        squares.append((f,r))
    return squares

def summary(groups):
    counts = Counter()
    entries = unique = max_unique = max_attack_squares = 0
    for f in range(N):
        for r in range(N):
            rays = [ray(f,r,df,dr) for df,dr in DIRECTIONS]
            for group in groups:
                lengths = [len(rays[i]) for i in group]
                relevant_bits = sum(max(length-1,0) for length in lengths)
                counts[relevant_bits] += 1
                entries += 1 << relevant_bits
                distinct = prod(max(length,1) for length in lengths)
                unique += distinct
                max_unique = max(max_unique, distinct)
                max_attack_squares = max(max_attack_squares,sum(lengths))
    return {
        'entries':entries,
        'bitboard_bytes':24*entries,
        'bitboard_MiB':24*entries / (1 << 20),
        'u16_bytes':2*entries,
        'u16_KiB':2*entries / (1 << 10),
        'u8_bytes':entries,
        'unique_attack_entries': unique,
        'max_unique_attacks_per_square_group': max_unique,
        'max_attack_squares':max_attack_squares,
        'u16_id_plus_unique_bitboards_bytes':2*entries+24*unique,
        'u16_id_plus_unique_bitboards_MiB':(2*entries+24*unique)/(1<<20),
        'relevant_bit_histogram':dict(sorted(counts.items())),
    }

out = {name:summary(groups) for name,groups in GROUPS.items()}
for name, groups in [('all_four_axes',GROUPS['rook_two_axes']+GROUPS['bishop_two_axes']),
                     ('all_eight_rays',GROUPS['rook_four_separate_rays']+GROUPS['bishop_four_separate_rays']),
                     ('rook_and_bishop',GROUPS['rook_four_rays']+GROUPS['bishop_four_rays'])]:
    out[name] = summary(groups)

# Folding words by OR is injective on a mask iff all selected raw%64 values
# differ. On diagonals f differs at every square, so this holds. On vertical
# lines squares separated by four ranks collide (same f and rank%4).
fold = {}
for name, groups in GROUPS.items():
    colliding_groups = 0
    maximum_collisions = 0
    first_collision = None
    for f in range(N):
        for r in range(N):
            rays = [ray(f,r,df,dr) for df,dr in DIRECTIONS]
            for group in groups:
                mask_squares = sum((rays[i][:-1] for i in group),[])
                bit_positions = [(16*r1+f1)%64 for f1,r1 in mask_squares]
                collisions = len(bit_positions) - len(set(bit_positions))
                if collisions:
                    colliding_groups += 1
                    maximum_collisions = max(maximum_collisions,collisions)
                    if first_collision is None:
                        first_collision = {'from':[f,r], 'directions':group, 'mask_squares':mask_squares}
    fold[name] = {'groups_with_fold_collision':colliding_groups,
                  'max_lost_distinct_bit_positions':maximum_collisions,
                  'first_collision':first_collision}
out['masked_word_or_analysis'] = fold
out['shared_twelve_square_line_table'] = {
    'source_coordinate_count':12,
    'occupancy_bits':10,
    'entries':12*(1<<10),
    'u16_bytes':12*(1<<10)*2,
    'u16_KiB':24,
    'entries_if_source_bit_omitted':2*(1<<10)+10*(1<<9),
    'u16_KiB_if_source_bit_omitted':14,
    'note':'Normalization and reconstruction metadata are excluded.'
}
assert out['rook_four_rays']['entries'] == (2*2**10+10*2**9)**2
assert out['rook_two_axes']['entries'] == 2*12*(2*2**10+10*2**9)
assert out['rook_four_separate_rays']['entries'] == 4*12*2**11
assert out['bishop_four_separate_rays']['entries'] == 4*(2*(2**11-1)+2**11)
print(json.dumps(out,indent=2))
