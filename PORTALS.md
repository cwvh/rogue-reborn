# Portals and room drawing

Where the room and portal drawing stands, what we have learned from rm02, rm05, rm08 and rm11
and from the original game, and what to try next. Written 2026-10-14, after two drawing modes
and many dumps.

In short: the drawing is now the original engine's, as far as we can tell (option C below,
without its one invented rule). Each room in view is drawn whole within the screen rectangle
of the portals it is seen through, sorted out by depth; inside the openings of the player's
own doorways the room beyond wins. It gets every known view right, including the original's
own flaw (rm11's wall in 120's doorway, BUGS.txt). The older drawing, which masks each room to
the exact opening it is seen through, once per path of portals, is kept behind
`--portal-paths` for now.

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

Added 2026-10-14, from photos of the original (`debug/original-engine-rm11-stairs-*.png`) and
the user's dumps:

| Where | The original | Tells us |
|---|---|---|
| rm11, anywhere in 121 (the room with the stairs; 119 is the paneled corridor behind the door) | the stairs never show through the door, at any angle | no distance rule: the player's room always loses inside its own doorways |
| rm11, at the door in 121, then one step into 119 | 121's wall in 120's doorway shows, then is gone | it switches on the player's room: in 119, the hall is only drawn within the door's rectangle |
| rm02 deck, in rooms 100 and 605 at once (no portal between them) | crates, railings and the deckhouse's lower walls always show | both rooms are drawn as the player's rooms |

Our model of the original: draw from the player's rooms (every room with a floor under the
feet at the same height; usually one). Draw them whole, and each other visible room whole
within the screen rectangle of the portals it is seen through, sorted out by depth. Cut at a
portal's plane near the doorway the player stands in (in its opening, within 40 units of its
plane). Inside the opening of each of the player's room's own doorways, the room beyond is
drawn in front of the player's room where that room overlaps it. This is the default drawing.

The map format agrees: a portal is a polygon and two room numbers, with no flags, no sides
and no visibility lists; each room's geometry is one object (rm11's 121 is one 1037-face
mesh); the `RoomFlags` in the mission files are weather (256 snow, 64 rain, checked in the
game). That is the data a 1999 engine needs for screen-rectangle portal culling over a depth
buffer, on hardware with no stencil to mask with.

## The two drawing modes

### `--portal-paths`: one path at a time (`src/visibility.rs` `plan`, `src/draw.rs` `draw_plan`)

The default until 2026-10-14.

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

### Default: rectangles (`rect_plan`, `touched_plan`, `wipe_opening`)

Was the `--portal-rects` and `--original-rooms` experiment.

Each room in view planned once, breadth first. Its rectangle for seeing further is its
portals' rectangles cut to the rooms in front; the rectangle it is drawn in (a scissor) is its
portals' whole rectangles. Each room is drawn whole, nearest first, sorted by depth. The
player's room and the doorways they stand in are cut as in the default drawing
(`touched_plan`). In the opening of each of the player's room's own doorways, the depth is
wiped where the surface already drawn lies inside the floor plan of the room beyond there
(`bounds_at`), so that room shows in front: this is what hides 121's stairs from the hall, as
in the original. The wipe is tested with a box drawn into the stencil (`wipe_opening`), and
is done only for an opening seen from the room's far side. (`--portal-rects` also wiped the
depth in deeper openings, up to 200 units past them, `OVERLAP_DEPTH`, to hide the wall the
original shows in 120's doorway; that was the one rule not taken from the original, and it
was removed on 2026-10-14.)

Right: every known view, and about twice as fast as the default on rm08 (room 203: 1.6 ms a
whole frame against 3.6).

Wrong, or not sure:
1. An overlap deeper than one doorway shows, as it does in the original (the wall in 120's
   doorway).
2. The doorway wipe cuts 15 pixels of a post at the corner of a roof in rm17 room 24.
3. A quarter-unit slack (`BOX_SLACK`) and the far-side test are guesses fitted to rm08.

Fixed in both modes on 2026-10-14:
- rm02's deck is in rooms 100 and 605, each with a floor at the same height and no portal
  between them. Drawing started from whichever floor came first and lost the other room's
  railings, crates and walls; now it starts from both (`CollisionWorld::floor_rooms`).
- On the top landing of rm02's stairwell (room 5), the feet were within 40 units of the top
  edge of the doorway into 404 a floor below, so the player counted as standing in it, and
  room 5's walls over 404 were cut away. Now the player stands in a doorway only inside its
  opening (`stands_in`).

## Known views

Each with the command for the default drawing; add `--portal-paths` or `--no-cull` to
compare. `--pos` takes z negated. Status: OK, or what goes wrong.

| # | View | `--portal-paths` | Default (rectangles) | Original game |
|---|---|---|---|---|
| 1 | rm08 201 by the alley | alley missing (budget) | OK | |
| 2 | rm08 203's landing | black band (portal from behind) | OK | |
| 3 | rm08 203, crane over the shed | crane cut | OK | crane whole |
| 4 | rm08 202, the shed's eave | eave cut (plane) | OK | |
| 5 | rm08 by the canal, alley facade | black band (portal from behind) | OK (golden s) | |
| 6 | rm08 stairwell landing, railing | railing OK (BOX_SLACK), stairwell floor black | OK (golden t) | |
| 7 | rm08 201, crane over the shed | crane cut | OK (golden v) | |
| 8 | rm05 outside at the door | OK | OK | clean |
| 9 | rm05 boarding stairs, top of the fuselage | OK (characters by room) | OK (golden u) | |
| 10 | rm11 hall doorway into 119, 120's doorway | wall hidden (golden q) | wall shows, as the original (golden p) | wall shows |
| 11 | rm11 hall, 77 units from the doorway into 119 | stairs hidden, wall hidden | as the original (golden w) | stairs hidden, wall in 120's doorway |
| 12 | rm11 courtyard spire | cut, as the original | cut | cut |
| 13 | rm11 119 doorway, ceiling | hairline of 121's stairs (TODO) | same | |
| 14 | rm11, one step past the hall's door into 119 | OK | OK (golden aa) | wall gone |
| 15 | rm02 deck by the stairs to the bridge deck | OK (fixed) | OK (golden y) | crates, railings |
| 16 | rm02 stairwell's top landing, at the generators' door | OK (fixed) | OK (golden z) | |

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
14 cargo rr --map data/map/rm11/rm11.map --doors-open 1 --pos 29495.1,32932.0,-39551.7 --yaw -51.2 --pitch -2.9 --first-person --textured
15 cargo rr --map data/map/rm02/rm02.map --pos 39365.4,32932.0,-30775.0 --yaw -58.0 --pitch 2.5 --first-person --textured
16 cargo rr --map data/map/rm02/rm02.map --doors-open 1 --pos 28527.1,34372.0,-31874.0 --yaw -691.6 --pitch -10.3 --first-person --textured
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
   Most overlaps the audit finds are between rooms several portals apart (only 22 of
   Rogue Spear's 148 pairs are joined by a portal), and the rectangles seldom draw the two
   together where the crossing is: the level designers tested against that drawing.
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

## Overlap audit (`examples/overlaps.rs`)

Drawing every room at once would be right but for the overlaps, so how many are there? The
audit finds the triangles of two different rooms that cut through each other, each reaching
more than 8 units past the other's plane along more than 16 units. It leaves out
alpha-tested (.tga) triangles, since trees are crossing planes. It then asks whether anyone can
see the crossing: an eye stands on each room's floors (on a 96-unit grid over its volumes,
on flat ground under both feet, with headroom), and looks at the four arms of the crossing.
Where two surfaces merely meet (a wall running into the ground), one arm of each is buried in
the other. Where three or four arms are seen, one surface pokes through the other. For each
pair of rooms it prints a `cargo rr ... --no-cull` command for a view of it.

```
cargo run --release --target x86_64-pc-windows-gnu --example overlaps -- rm11 [--all]
```

Results over all 129 maps of several RS engine games (`debug/overlaps-all-games.txt`; a level
takes a second, the crossings alone a tenth of that):

- 63 levels have no overlap anyone can see; rm08 is one of them.
- 56 levels have 255 pairs of rooms that overlap where seen, between different materials,
  along 100 units or more. Same-material pairs are mostly two walls overshooting at a
  corner, or the ground of two outdoor cells crossing.
- It finds every overlap we knew of (rm05's hull with 17 cabin rooms; rm11's 121 with 119
  and 120) and new ones. Checked by eye: rm11's courtyard ground (room 1) runs through the
  cellar (110); tm01's 400 stands in 101; tm02's 900 cuts a corridor; sp02's ceiling boards
  cut a room. In each, the default drawing hides the overlap. False alarms seen: rm17's eave
  (1 and 9), rm11's snow-on-snow between outdoor cells, and eyes on roofs out of play (EW01,
  bt08).
- BT03, bt05 and bt06 do not parse yet (the map reader also asks for 1.8 GB on BT03, which
  can abort the process).

So overlaps are common, and mostly between an outdoor room (1, 100, 500, 900) and the
buildings in it. Drawing everything is right only on about half the levels.

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

**B'. Draw everything, and mask only the pairs of rooms that overlap.** When the eye is in
one room of an overlapping pair (from the audit), the other is drawn only through portals,
in front of the eye's room inside their openings. This is right for point 2: from the hall,
119 is drawn in its doorway in front of the stairs, which is the doorway wipe both modes
already have. No search of the portal graph on levels without overlaps (rm08 and half the
others). A false alarm costs nothing but masking a room that did not need it. Its risk is
that the masking is the same per-portal drawing whose bugs started all this, now limited to
the overlapping rooms (mostly an outdoor room against the buildings in it, whose portals are
doors and windows).

**C. The rectangles, modeled on the original (chosen, 2026-10-14).** Fast, right in every
known view, and each rule is something the original was seen to do. The 200-unit wipe that
fixed the original's own flaw (point 2) was dropped: we match the original instead. An
overlap deeper than one doorway that we want fixed is a case for D.

**D. Annotate each map by hand** (which room's triangles to hide from where). Exact, and
there are only so many maps, but it is work per map and does not explain anything.

## Next steps

1. Turn the scratch script that rendered every view in every mode into `scripts/views.sh`
   (one image per view and mode, and a count of the pixels that differ), so every change is
   checked against all of the table above. Until then the views live in golden.sh (p to aa).
2. Play the other levels (rm01 to rm19, citystreet), and dump what looks wrong. Where in
   doubt, check the same spot in the original game, and add the view to the table.
3. Rerun the overlap audit with the rectangles instead of `--no-cull`: count only crossings
   where the eye reaches the other room through portals and the crossing is inside that
   room's rectangle. That is the list of overlaps the drawing can still show.
4. If nothing new turns up, remove the per-path drawing (`--portal-paths`: `plan`, most of
   `draw_plan`).
5. Display lists per room (the frame time on big views).
5. Characters, decals and glass inside the rectangle of their room (characters show only when
   their room is drawn, but are not limited to its rectangle).

## Where things are

- `src/visibility.rs`: `rect_plan` (default; `RectRoom::doorways` says which openings are
  wiped) and `touched_plan`, `plan` (`--portal-paths`), `stands_in` (the doorways the player
  stands in), `bounds_at` (doorway cut boxes), `project` (portal rectangles), `portal_plane`.
- `src/collision.rs`: `floor_rooms` (the rooms drawing starts from).
- `src/draw.rs`: `draw_plan` (`BOX_SLACK`, `PLANE_SLACK`), `wipe_opening`.
- `src/main.rs`: chooses the plan and draws it; characters show only when their room is drawn
  (`shown`), in both modes.
- Flags: `--no-cull`, `--portal-paths`, `--portals` (outlines), `--rooms` (room colors),
  `--hide-rooms`, F2 or `--dump`.
- Tests: `the_halls_wall_stands_inside_the_room_past_119`,
  `the_room_past_the_next_doorway_is_left_to_the_depth_test`,
  `every_room_in_view_is_planned_once`, `standing_in_a_portal_means_in_its_opening`
  (`src/visibility.rs`), `two_rooms_can_share_a_floor` (`src/collision.rs`); golden
  scenarios p to aa (`scripts/golden.sh`).
- BUGS.txt: the spire and the hall's wall. TODO: the stairs hairline, characters through
  walls in the per-path drawing, and the [perf] entries.
- Local and git-ignored: `debug/perf-instrumentation.patch` (draw counters, prints the plan
  tree), `debug/rm08-portal-debug.patch` (`portal-sides` example, `RR_SKIP_PORTALS`), and the
  photos of the original in `debug/original-engine-rm11-stairs-*.png`.
