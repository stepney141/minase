import json, math
from pathlib import Path
repo = Path("/home/stepney141/board-games/minase")
out = Path("/tmp/claude-1000/-home-stepney141-board-games-minase/6df15eae-20d7-4230-8efc-cdb9a3906246/scratchpad/astra-work")
directions = [(0,1),(1,1),(1,0),(1,-1),(0,-1),(-1,-1),(-1,0),(-1,1)]
knights = [(1,2),(2,1),(2,-1),(1,-2),(-1,-2),(-2,-1),(-2,1),(-1,2)]
def inside(x,y):
    return 0 <= x < 12 and 0 <= y < 12
def valid(plane, square, promotion_zone=False):
    x,y=square%12,square//12
    if plane < 176:
        direction,distance=divmod(plane%88,11)
        dx,dy=directions[direction]
        tx,ty=x+dx*(distance+1),y+dy*(distance+1)
        return inside(tx,ty) and (not promotion_zone or plane<88 or y>=8 or ty>=8)
    if plane < 184:
        dx,dy=knights[plane-176]
        return inside(x+dx,y+dy)
    if plane < 240:
        first,second=divmod(plane-184,7)
        dx,dy=directions[first]
        mx,my=x+dx,y+dy
        reverse=(first+4)%8
        second += second >= reverse
        sx,sy=directions[second]
        return inside(mx,my) and inside(mx+sx,my+sy)
    if plane < 248:
        dx,dy=directions[plane-240]
        return inside(x+dx,y+dy)
    assert plane==248
    return True
geometry_mask=[valid(p,s) for p in range(249) for s in range(144)]
mask=[valid(p,s,True) for p in range(249) for s in range(144)]
counts=[sum(mask[a*144:b*144]) for a,b in [(0,88),(88,176),(176,184),(184,240),(240,248),(248,249)]]
fixtures=[json.loads(s) for s in (repo/"tools/train/zero/fixtures/nn-positions.jsonl").read_text().splitlines()]
labels=[move[4] for f in fixtures for move in f["moves"]]
assert all(mask[label] for label in labels)
result={"on_board_geometry_count":sum(geometry_mask),"valid_including_promotion_zone":sum(mask),"always_impossible_count":len(mask)-sum(mask),"counts_by_family":counts,
        "fixture_positions":len(fixtures),"fixture_legal_moves":len(labels),"fixtures_not_masked":True,
        "equal_logits_total_loss":math.log(len(mask)),"equal_logits_geometry_loss":math.log(sum(mask)),
        "loss_difference":math.log(len(mask)/sum(mask)),
        "uniform_legal_mass_if_81":81/len(mask),"uniform_geometry_legal_mass_if_81":81/sum(mask)}
(out/"round1-policy-mask.json").write_text(json.dumps(result,indent=2)+"\n")
print(json.dumps(result))
