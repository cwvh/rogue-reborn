FPS collision implementation plan (rooms, levels, geometries)

Overview
- Goal: robust first-person movement that respects the correct floor for the player’s current “planning level” and prevents passing through walls.
- Data to use:
  - planning_levels.levels[L].room_names and floor_height
  - rooms.rooms[r].level_heights (per-room level reference heights) and sherman_levels.tm_with_aabb (AABB culling volumes)
  - geometries.objects[o] render faces (Faces + vertices) and collisions (FaceNormal planes)

Active sets per frame (filtered by planning level)
- Current planning level index: L
- Active rooms: R = { r | rooms.rooms[r].section_name ∈ planning_levels.levels[L].room_names }
- Active geometry objects: O = { o | geometries.objects[o].name ∈ R.section_name }
- Use only collisions and faces from O for physics (walls/floors); other rooms/levels are ignored so lower/upper floors do not incorrectly block movement.

Mapping planning level height to per-room heights
- planning_levels.levels[L].floor_height is a global hint; rooms.rooms[r].level_heights is room-local.
- For each active room r, pick a target room floor height H_r(L):
  - Option A (direct index): If |rooms.rooms[r].level_heights| equals total planning_levels, use the same index L if valid.
  - Option B (nearest): Choose h in rooms.rooms[r].level_heights with minimal |h.height − floor_height| to get H_r(L).
- Use H_r(L) as the room-local reference band for choosing the correct “floor” planes when stacking levels exist.

Preprocessing (startup)
1) Build maps for fast lookup:
   - room_name → room_id
   - object_name → object_id
   - room_id → {object_ids} where object.name == room.section_name
2) For each room r:
   - Keep its sherman_levels.tm_with_aabb AABBs; these are used to cull queries to local regions.
3) Optional acceleration:
   - For each (room r), build two plane lists from collisions.faces:
     - Floors: FaceNormal with ny >= cos(max_slope) (e.g., 50° → ~0.64–0.66)
     - Walls: FaceNormal with |ny| <= ny_wall_max (e.g., 0.3)
   - For each plane, compute a loose XY extent proxy:
     - Use the room’s AABB bands from sherman_levels and/or the object’s vertex AABB to limit the effective area of infinite planes.
   - Store per-room BVH or uniform grid over these proxies (optional first step: simple linear scan constrained by AABBs).

Runtime per frame
A) Determine current active sets
- With current L, set R and O as above.
- Determine the player’s current room r by testing which room AABB(s) (from sherman_levels.tm_with_aabb) contain the player XY; on ties, choose the one whose H_r(L) is closest to player Y.

B) Grounding: find floor under the player
- Reference band:
  - target = H_r(L) for the current room r.
  - Band window: e.g., ±200 units to filter candidate floors to the right level.
- Candidate planes:
  - From collisions.faces of O in the current room r.
  - Up-facing (ny >= cos(max_slope)).
  - Culling: only those whose AABB (room AABB and/or object AABB) contains player XY.
- Solve y = (−d − nx·x − nz·z)/ny for each candidate.
  - Prefer solutions within the target band (|y − target| ≤ band).
  - Choose the nearest y ≤ player.y; if none, choose the smallest y > player.y (for mid-air snap).
- Set player.y = y + eye_height.

C) Lateral collision (walls) with capsule
- Player is a vertical capsule:
  - radius R (e.g., 25–30 units)
  - eye_height H_eye (current code) and body_height H_body (e.g., 180 units)
  - Foot point Y_foot = player.y − eye_height; capsule spans [Y_foot, Y_foot + H_body].
- Movement attempt: Δ = input vector in XY plane based on WASD and yaw.
- Candidate walls:
  - From collisions.faces of O in the current room r.
  - Near-vertical planes: |ny| ≤ ny_wall_max (e.g., 0.3).
  - Culling: same AABB tests as floors + optionally project capsule center onto plane AABB XY.
- Plane test (approximate sweep):
  - Signed distance s = n·p + d where p uses (x, y_ref, z) with y_ref ~ Y_foot + H_body/2 (capsule mid-height).
  - If s ≥ −R after applying Δ (i.e., penetration or crossing), project Δ to slide along the wall:
    - Let n_h = normalize([nx, 0, nz]); remove the component of Δ along n_h:
      Δ' = Δ − dot(Δ, n_h)·n_h
    - Update player.xz with Δ'.
  - Iterate 1–2 times to handle corners.

D) Vertical motion nuances (optional step)
- Step up: If a small obstacle (height ≤ step_height) is in the way, allow stepping up by moving Y_foot up to the obstacle top; then snap to ground again.
- Gravity/jumps: If you later add gravity, run a vertical sweep test against floors (downward).

E) Room transitions
- Portal-based: If segment from old to new XY intersects a portal polygon, update current room to the opposite_room.
- AABB-based fallback: If player XY exits all sherman_level AABBs of current room, choose the nearest room whose AABB contains the XY; then recompute H_r(L) and snap to that room’s floor.

Geometry restrictions (finite planes)
- Collisions.faces are infinite planes; avoid over-restricting:
  - Restrict plane influence using the union of:
    - The room’s sherman_levels.tm_with_aabb AABBs for the current level.
    - The object’s geometry AABB for objects in O.
  - Optional: derive convex polygon extents by projecting the associated render triangles (Faces.face_indices + vertices) that belong to the same object and level, and keep only planes whose XY point lies within those polygons. This is a later refinement.

LH→RH consistency
- Rendering applies glScalef(1,1,−1). Ensure physics queries are consistent:
  - Either negate Z on all player/query points before evaluating n·p + d,
  - Or pre-convert plane normals (nz = −nz) and adjust d once at load time.
  - Pick one approach and stick with it to avoid double flips.

Incremental implementation steps
1) Active set filter: limit rendering and physics to planning_levels[L].room_names and their objects.
2) Ground snap with banded floors:
   - Implement find_ground_y(x,z,y_current,target=H_r(L)) using collisions.faces + AABB culling.
3) Capsule wall slide:
   - Adopt simple plane-based projection of Δ against near-vertical planes using radius R.
4) Room transitions:
   - Start with AABB-based room assignment; add portal crossing later.
5) Extents and acceleration:
   - Use room and object AABBs to bound planes; add a simple uniform grid or coarse BVH for queries.
6) Refinements:
   - Stepping logic; portal-aware culling; polygon-clipped planes from render geometry; gravity/jumps.

Tunable parameters (initial suggestions)
- max_slope_deg = 50.0 (floor threshold → ny ≥ cos(50°))
- ny_wall_max = 0.3 (|ny| ≤ 0.3 → walls)
- band window around H_r(L): ±200.0
- capsule radius R: 25.0–30.0
- body_height: ~180.0
- step_height: ~30.0–40.0

This plan ensures the player snaps to the correct floor for the current planning level and collides only with relevant rooms’ geometry. Floors below/above won’t block unless they physically intersect the player’s path, and wall collisions produce natural sliding using a capsule approximation.
