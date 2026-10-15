# Portals and room drawing

Where the room and portal drawing stands, what we have learned from rm05, rm08 and rm11 and
from the original game, and what to try next. Written 2026-10-14, after two drawing modes and
many dumps.

In short: the `--portal-rects` experiment now gets every known view right. It draws whole
rooms within loose screen rectangles, as the original engine does. Its rules come from what
the original does, with one exception: it also hides a stray wall the original shows, which
`--original-rooms` keeps. The default drawing masks each room to the exact opening it is
seen through. It gets the overlaps right, but it cuts real geometry and gets lost in rm08's
portals. The proposal is to make the rectangles the way forward (option C below). The
per-path drawing stays the fallback, for overlaps the rectangles cannot decide.

## The problem

A level is a set of rooms. Each room has geometry (objects named after it), volumes (boxes,
mostly thin floor slabs, the "sherman levels") and portals to other rooms. The geometry of
rooms overlaps:

- **rm05**: the plane's cabin is bigger than its hull. From inside, the hull (part of the
  outside, room 100) crosses the cabin; from outside, the upper deck's walls (room 401) poke
  out of the fuselage.
- **rm11**: the hall (121) wraps around room 119. 121's stairs climb through 119's back wall
  and ceiling, and a wall of 121 stands 60 units inside room 120, past 119's far doorway.

So drawing every room at once (`--no-cull`) is wrong wherever rooms overlap. The original
engine drew the player's room and what is seen through its portals, and so do we.

rm08 adds a different kind of trouble. It is an outdoor dock block cut into cells:

- Ground cells (201 around a shed, 203 the alley by the canal, 202 and 200 north-south
  strips) are separated by portals that are walls of air, 800 units tall, up to roof height
  (y 33202).
- Room 501 is one volume of sky over the whole block, joined to the cells below by flat
  portals at y 33202. "pt" portals open only over a low roof (y 32748 up, over the shed).
- So the portal graph is dense and full of loops: from room 203 the default plan has 83
  paths through portals, and stops there only because of its budget.
- Room 201 (the shed's cell) has no portal into the sky.
- Room 203 is L-shaped: standing on its landing, the player is on room 521's side of portal
  pt203_521 (whose sides the planner cannot tell apart), and sees it from behind.
- The shed's roof overhangs its portal into 202.

## What the original engine does

Observed by playing it (see the memory note too):

| Where | The original | Tells us |
|---|---|---|
| rm08, alley 203 looking up over the shed at the crane | crane whole | no exact masking to portal openings, at least for the sky |
| rm05, outside at the boarding stairs, looking at the side door | fuselage clean | rooms are masked to their portals somehow |
| rm11, hall doorway into 119, looking through 119 into 120 | 121's wall stands in 120's doorway | rooms drawn whole within a rectangle and sorted by depth; nothing wipes the depth in an opening |
| rm11 hall, 77 units from the doorway into 119 (view 11) | 121's stairs hidden; 121's wall in 120's doorway still shows | the player's room is hidden inside the room just past one of its own doorways, and no deeper |
| rm11 courtyard, by portal pt05_06 | spire cut in half (BUGS.txt) | rooms are cut at a portal's plane near it |
| rm11 courtyard, deep in room 6 | spire hidden, back when turning left | the rectangle of a portal that wraps behind the camera is view-dependent; not replicated |
| rm05, third person | the camera snaps into the cabin as the player enters | the original keeps the camera in the player's room |

Our working model of the original: each visible room drawn whole within the screen
rectangle of the portals it is seen through, sorted out by depth, cut at a portal's plane
near the doorway the player stands in. Inside the opening of each of the player's room's own
doorways, the room beyond is drawn in front of the player's room where that room overlaps it.
`--original-rooms` draws that way.

## The two drawing modes

### Default: one path at a time (`src/visibility.rs` `plan`, `src/draw.rs` `draw_plan`)

From the player's room, a tree of every path through portals in view (depth first, 96 nodes
at most, 8 deep). For each node: mark the portal's opening in the stencil where it is
visible, wipe the depth there, draw the room beyond cut at the portal's plane and masked to
the opening, then seal the opening. Standing within 40 units of a portal, both rooms are
drawn whole and each is cut only where it overlaps the other (`cut_in_self` / `bounds_at`).

Right: every rm05 and rm11 view, the hall's stairs and walls included.

Wrong:
1. Runs out of its budget on rm08's dense graph before reaching the room next to the player
   (view 1 below: almost nothing drawn), and draws rooms many times (rm08: 84 room draws,
   28,200 triangles where the whole level is 9,677).
2. A portal seen from behind (203's landing, pt203_521) gets a node; the room beyond has
   nothing there, the opening is sealed, and the real room cannot draw into it (black band).
3. Masking to the exact opening cuts what is seen past a missing portal (the crane over the
   shed) and what overhangs a portal (the shed's eave).
4. The infinite plane cut cuts things far to the side of a portal (rm11 spire, BUGS.txt).
5. Characters show through walls where an opening's depth was wiped and the room beyond has
   nothing (rm08 by the canal).

### Experiment: rectangles (`--portal-rects`, `--original-rooms`; `rect_plan`, `wipe_opening`)

Each room in view planned once, breadth first. Its rectangle for seeing further is its
portals' rectangles cut to the rooms in front; the rectangle it is drawn in (a scissor) is its
portals' whole rectangles. Each room is drawn whole, nearest first, sorted by depth. The
player's room and the doorways they stand in are cut as in the default drawing
(`touched_plan`). In the opening of each of the player's room's own doorways, the depth is
wiped where the surface already drawn lies inside the floor plan of the room beyond there
(`bounds_at`), so that room shows in front: this is what hides 121's stairs from the hall, as
in the original. Both modes do that. `--portal-rects` also wipes the depth in deeper
openings, where the surface already drawn is within 200 units past the opening
(`OVERLAP_DEPTH`): this hides the wall the original shows in 120's doorway. Each wipe is
tested with a box drawn into the stencil (`wipe_opening`), and is done only for an opening
seen from the room's far side.

Right: every known view, and about twice as fast as the default on rm08 (room 203: 1.6 ms a
whole frame against 3.6).

Wrong, or not sure:
1. The 200-unit depth is the one rule not taken from the original. An overlap deeper than one
   doorway and farther than 200 units past an opening shows, as it does in the original.
2. The doorway wipe cuts 15 pixels of a post at the corner of a roof in rm17 room 24.
3. A quarter-unit slack (`BOX_SLACK`) and the far-side test are guesses fitted to rm08.

## Known views

Each with the command for the default drawing; add `--portal-rects`, `--original-rooms` or
`--no-cull` to compare. `--pos` takes z negated. Status: OK, or what goes wrong.

| # | View | Default | --portal-rects | Original game |
|---|---|---|---|---|
| 1 | rm08 201 by the alley | alley missing (budget) | OK | |
| 2 | rm08 203's landing | black band (portal from behind) | OK | |
| 3 | rm08 203, crane over the shed | crane cut | OK | crane whole |
| 4 | rm08 202, the shed's eave | eave cut (plane) | OK | |
| 5 | rm08 by the canal, alley facade | black band (portal from behind) | OK (golden s) | |
| 6 | rm08 stairwell landing, railing | railing OK (BOX_SLACK; golden t), stairwell floor black | OK | |
| 7 | rm08 201, crane over the shed | crane cut | OK (golden v) | |
| 8 | rm05 outside at the door | OK | OK | clean |
| 9 | rm05 boarding stairs, top of the fuselage | OK (characters by room; golden u) | OK | |
| 10 | rm11 hall doorway into 119, 120's doorway | OK (golden p) | OK (golden q) | wall shows (golden r, `--original-rooms`) |
| 11 | rm11 hall, 77 units from the doorway into 119 | OK | OK (golden w; x with `--original-rooms`) | stairs hidden, wall in 120's doorway |
| 12 | rm11 courtyard spire | cut, as the original | cut | cut |
| 13 | rm11 119 doorway, ceiling | hairline of 121's stairs (TODO) | same | |

```
1  cargo rr --map data/map/rm08/rm08.map --pos 34202.9,32567.0,-29624.9 --yaw -28.1 --pitch -1.9 --first-person --textured
2  cargo rr --map data/map/rm08/rm08.map --pos 34352.5,32567.0,-29911.0 --yaw -20.7 --pitch -1.2 --first-person --textured
3  cargo rr --map data/map/rm08/rm08.map --pos 34610.2,32567.0,-30290.5 --yaw 34.6 --pitch 22.6 --first-person --textured
4  cargo rr --map data/map/rm08/rm08.map --pos 34423.6,32567.0,-31061.1 --yaw 47.8 --pitch 40.6 --first-person --textured
5  cargo rr --map data/map/rm08/rm08.map --pos 34368.0,32567.0,-29610.3 --yaw 329.4 --pitch -0.8 --first-person --textured
6  cargo rr --map data/map/rm08/rm08.map --pos 34448.0,32567.0,-29726.5 --yaw 356.1 --pitch -10.9 --first-person --textured
7  cargo rr --map data/map/rm08/rm08.map --pos 33136.0,32567.0,-30433.2 --yaw 352.5 --pitch 39.0 --first-person --textured
8  cargo rr --map data/map/rm05/rm05.map --pos 36700.0,32783.2,-36300.0 --yaw -70.0 --pitch 0.0 --first-person --textured
9  cargo rr --map data/map/rm05/rm05.map --spawn 13 --pos 37229.5,32932.0,-36546.5 --yaw -100.6 --pitch 52.0 --first-person --textured
10 cargo rr --map data/map/rm11/rm11.map --doors-open 1 --pos 29467.9,32932.0,-39542.9 --yaw 312.8 --pitch -5.0 --first-person --textured
11 cargo rr --map data/map/rm11/rm11.map --doors-open 1 --pos 29400.5,32932.0,-39432.1 --yaw -35.8 --pitch 2.7 --first-person --textured
12 cargo rr --map data/map/rm11/rm11.map --pos 32639.9,32832.0,-38155.6 --yaw -170.8 --pitch 13.2 --first-person --textured
```

Also checked in both modes, all fine: rm05's side door from inside and the upper deck, rm11's
hall, room 119 and the stairs (four directions each), rm12 room 850, rm17 room 24 (but for the
15 pixels above).

## What we learned

1. **Overlap is the only reason to mask at all.** In every view above, `--no-cull` is right
   except where rooms overlap (rm05's cabin and hull, rm11's hall with 119 and 120). Nothing
   in our rm08 views overlaps.
2. **In an overlap, which room a point belongs to depends on the path to it.** 121's stairs
   are stairs seen from the hall, and junk seen through 119's doorway: the same triangles,
   one pixel apart. In general only a mask per path through portals can decide that per
   pixel. The original does not solve it in general either: it hides the player's room
   inside the room just past one of its own doorways (the stairs), and nothing deeper (the
   wall in 120's doorway). One doorway deep covers every overlap we have seen.
3. **Exact masks are too strict for this data.** Portals are missing (201's sky), roofs
   overhang their portals, rooms are L-shaped so their own portals are seen from behind, and
   some portals' sides cannot be worked out (pt203_521).
4. **Planning paths depth first with a node budget fails on dense graphs** (rm08), and
   drawing a room once per path costs a lot in immediate mode.
5. **Immediate mode costs about 0.5 us per glBegin**, whatever is in it. Display lists or
   vertex buffers per room would make drawing a room once per path cheap.
6. **Room volumes are floor slabs, not air.** They cannot tell overlap junk from real
   surfaces: most portals have some other room's triangles inside the room beyond (77 of
   rm11's 111, 82 of rm08's 104), and rm11's room 500 contains triangles of 21 rooms.

## Options

**A. The per-path drawing, repaired (the fallback).** It is right for overlaps by
construction (point 2), and its failures are each a separate, fixable thing:
- Plan breadth first, nearest first; give up on a path when its opening is a few pixels, not
  after a node count. One node for several portals between the same two rooms.
- No node for a portal seen from behind (the far-side test from `rect_plan`), so nothing is
  sealed by mistake.
- Cut at the portal's plane only inside the box where the two rooms overlap (the fix
  BUGS.txt proposes for the spire); elsewhere the stencil mask is enough. Fixes the eave.
- Draw outdoor rooms (the sky, 501) unmasked, or masked to their openings' rectangles, behind
  everything else: fixes the crane without a portal that is not there.
- Draw each character inside its room's path (in draw_plan), so none shows through walls.
- Display lists per room, so the extra draws cost little.

**B. Draw everything, hide overlaps by the camera's room.** Simple and fast, but point 2
rules it out: from the hall, the stairs must show and the same stairs must not show through
119's doorway.

**C. The rectangles, modeled on the original (proposed).** Fast, right in every known view,
and each rule is something the original was seen to do, except the 200-unit wipe that fixes
the original's own flaw (point 2). Its risk is an overlap deeper than one doorway that the
original also gets wrong and that we want fixed: then the per-path drawing (A) is the tool.

**D. Annotate each map by hand** (which room's triangles to hide from where). Exact, and
there are only so many maps, but it is work per map and does not explain anything.

## Next steps

1. Turn the scratch script that rendered every view in every mode into `scripts/views.sh`
   (one image per view and mode, and a count of the pixels that differ), so every change is
   checked against all of the table above.
2. Play the other levels (rm01 to rm19, citystreet) with `--portal-rects`, and dump what looks
   wrong. Where in doubt, check the same spot in the original game, and add the view to the
   table.
3. If nothing new turns up, make `--portal-rects` the default drawing: remove the per-path
   drawing (`plan`, most of `draw_plan`), keep `--original-rooms`, and clean up the names
   and comments marked EXPERIMENT.
4. Display lists per room (whichever drawing stays; the frame time on big views).
5. Characters, decals and glass inside the rectangle of their room (characters show only when
   their room is drawn, but are not limited to its rectangle).

## Where things are

- `src/visibility.rs`: `plan` (default), `rect_plan` (experiment; `RectRoom::doorways` and
  `portals` say which openings are wiped) and `touched_plan`,
  `bounds_at` (doorway cut boxes), `project` (portal rectangles), `portal_plane`.
- `src/draw.rs`: `draw_plan` (default; `BOX_SLACK`, `PLANE_SLACK`), `wipe_opening` and
  `WipeWithin` (`OVERLAP_DEPTH`).
- `src/main.rs`: chooses the plan and draws it; characters show only when their room is drawn
  (`shown`), in both modes.
- Flags: `--no-cull`, `--portal-rects`, `--original-rooms`, `--portals` (outlines), `--rooms`
  (room colors), `--hide-rooms`, F2 or `--dump`.
- Tests (`src/visibility.rs`): `the_halls_wall_stands_inside_the_room_past_119`,
  `the_room_past_a_doorway_is_drawn_in_front_inside_its_opening`,
  `every_room_in_view_is_planned_once`; golden scenarios p to x (`scripts/golden.sh`).
- BUGS.txt: the spire and the hall's wall. TODO: the stairs hairline, characters through
  walls in the default drawing, and the [perf] entries.
- Local and git-ignored: `debug/perf-instrumentation.patch` (draw counters, prints the plan
  tree), `debug/rm08-portal-debug.patch` (`portal-sides` example, `RR_SKIP_PORTALS`), and the
  images of every view in every mode in `debug/portal-rects`, `debug/rm08-rects`,
  `debug/rm08-compare`, `debug/rm05-compare` and `debug/new-bugs`.
- Not committed yet: the experiment, the characters-by-room fix and `BOX_SLACK` (all in the
  files above), and this note.
