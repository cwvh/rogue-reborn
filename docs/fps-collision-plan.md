First-person collisions and floor selection plan using MAP structures

Goals
- Accurate, stable first-person (FPS) movement:
  - Grounding: keep player’s feet on the correct floor for the current level.
  - Walls: prevent walking through walls, with natural sliding.
  - Multi-level: snap to and collide against the correct level’s geometry.
- Fast queries using precomputed indices/acceleration per room/level.

Key MAP relationships we can leverage (from NOTES.txt and src/map.rs)
- Room identity linkage:
  - geometries.objects[i].name matches rooms.rooms[j].section_name.
  - This lets us associate each geometric Object (and its collisions) with a Room.
- Levels per room:
  - rooms.rooms[k].sherman_levels: contains tm_with_aabb entries with AABBs that bound pieces of a level within the room (good for spatial culling).
  - rooms.rooms[k].level_heights: per-room reference heights; these can anchor clustering of walkable planes.
- Global level view:
  - planning_levels.levels[n]:
    - level_number and floor_height for that global level.
    - room_names: which rooms participate in that planning level.
  - This is a strong hint for which floors/walls to enable for a given global level.
- Collision geometry:
  - objects[].collisions: a simplified collision mesh intended for physics.
    - faces: planes (FaceNormal + distance_origin_to_face).
    - vertices: collision vertices (may be used in other contexts).
  - Prefer collisions over render meshes for walkability and walls.

Coordinate system note (LH → RH)
- Rendering now flips Z (glScalef(1,1,-1)) to match OpenGL RH.
- CPU-side physics and camera should consistently use one space. Two safe choices:
  1) Convert all inputs used by physics (player position, mission start, planes) to RH by negating Z once; or
  2) Keep physics in LH, convert only when rendering.
- Given you already negated the mission Z and use planes as-is, standardize to RH:
  - For every collision plane n=(nx,ny,nz), d: convert to RH with nz'=-nz and d' updated consistently with your plane convention (n·p + d = 0). If planes were derived in LH, a Z flip implies d' = d as long as p is also measured in RH after Z-negation. Validate numerically on a few points.
  - Alternatively, negate Z on all player/query points before evaluating planes, to keep plane coefficients untouched. Pick one and be consistent.

Preprocessing pipeline
1) Build room index
   - Map room_name → room_id for rooms.rooms.
   - Map object_name → object_id for geometries.objects.
   - For each room, collect object indices whose name matches the room’s section_name.

2) Per-room, per-level banding (walkable floors)
   - Gather candidate walkable planes from collisions.faces of the room’s objects where normal.y >= cos(max_slope).
   - Compute representative height per plane (e.g., evaluate plane at room AABB center XY, or at projected centroids from associated triangles if available).
   - Cluster these heights with a small epsilon (e.g., 0.1–0.2m) to form height bands.
   - Snap bands to the nearest known heights from:
     - rooms.rooms[].level_heights, and/or
     - planning_levels.floor_height for levels that include this room (planning_levels.room_names contains the room’s name).
   - Result: per-room, per-level FloorSet = {planes, optional merged patches/BVH}.

3) Spatial culling volumes
   - For each rooms.rooms[r].sherman_levels[l].tm_with_aabb entry, keep its AABB as a culling proxy for that room-level segment.
   - Use these to quickly select the relevant subset of planes for ground/wall queries (only those whose AABB XY contains the player or query ray).

4) Acceleration structures
   - Build per room-level:
     - A simple BVH or uniform grid over plane proxies (optionally use AABB XY slices).
     - If you reconstruct triangles for collisions (later), build a triangle BVH instead of planar sets.

Runtime: selecting active room and level
- Room selection:
  - Start with the room that contains the initial spawn XY in one of its ShermanLevel AABBs (or the closest).
  - As the player moves, if they cross a portal polygon (portals.portals entries connect room indices by names), transition to the opposite room. For a simpler first cut, if they leave all AABBs of the current room, find the nearest room AABB that contains the new XY.
- Level selection within a room:
  - Use current player Y to choose the nearest FloorSet band (within a tolerance) or the planning_levels.floor_height for that room.
  - Hysteresis: to avoid rapid toggling between adjacent levels, add a small dead zone or require a minimum delta before switching.

Grounding (floor/ramps/stairs)
- For current room and selected level:
  - Raycast down: using plane sets, compute y = (-d - nx*x - nz*z)/ny for each up-facing plane; pick the nearest below current Y. Optionally, if triangles are available, use triangle intersection tests to ensure XY is within the polygon, not just on an infinite plane.
  - Snap: player.y = ground_y + eye_height.
  - Slopes: Accept planes with ny >= cos(max_slope) so gentle ramps are walkable.
  - Stairs: Even if risers violate slope threshold, you’ll still get step-wise planes. To smooth motion, allow stepping up by a limited height if the forward sweep detects a near step (capsule step-up logic can come later).

Walls (lateral collision and sliding)
- Restrict candidate planes to near-vertical: |ny| <= ny_wall_max (e.g., 0.3).
- Also restrict to current room-level’s AABBs to avoid far planes.
- Do a capsule (or cylinder) sweep in the intended horizontal move:
  - Approximate first: evaluate signed distance s = n·p + d at the new XY (with player’s reference Y ~ feet/eye height as needed).
  - If s > -radius (penetrating or crossing), push out along the horizontal component of the normal by penetration depth.
  - Perform 1–2 iterations to resolve corners.
- Sliding: When pushing out, only remove the penetration along the wall normal (keep tangential component) to produce natural sliding.
- Portals/door frames: Include planes near active portal polygon when transitioning to neighbor rooms; otherwise you can clip against door jambs in either room.

Planning-level integration
- Planning levels give a macro organization:
  - Active rooms for a global level = planning_levels.levels[L].room_names.
  - floor_height = nominal floor for that global level.
- Uses:
  - On ‘F’ first-person toggle, pick the nearest planning level by comparing player Y to floor_height values; clamp to that level for initial grounding.
  - Restrict collision queries to rooms that belong to the active planning level to reduce false positives from upstairs/downstairs.
  - When the computed ground_y deviates significantly from the level’s floor_height (e.g., ramps, mezzanines), prefer the raycast result but keep level assignment unless the deviation exceeds a threshold.

Handling missing or coarse collision data
- If collisions.faces are only planes (no per-face polygons), infinite planes can over-constrain:
  - Mitigate by culling with room-level AABBs and, if available, approximate plane extents using nearby render triangles or by projecting collision vertices (if they correspond).
  - If a room/object has no collisions, temporarily fall back to render geometry faces for walls/floors in that area.

Proposed implementation steps (incremental)
1) Indexing
   - Build maps: room_name→room_id, object_name→object_id, room_id→object_ids.
   - Build planning level index: level_number→{floor_height, room_ids}.

2) Band floors per room
   - Extract up-facing planes from collisions in each room; cluster into height bands; snap to level_heights when close.

3) Build per room-level culling sets
   - For each room-level, gather its planes and associated ShermanLevel AABBs.
   - Optionally build a simple grid/BVH over XY.

4) FPS runtime
   - Maintain (current_room, current_level).
   - Ground query: downward plane evaluation within culling sets, snap to ground.
   - Lateral collision: plane-based push-out within culling sets.

5) Room transitions
   - Detect portal crossing by projecting movement segment onto portal polygons or bounding strips; swap to opposite room on crossing.
   - Alternatively, re-evaluate room by AABB containment when leaving current AABBs.

6) Validation & tuning
   - Tune ny thresholds (floors vs walls), clustering epsilons, level switch hysteresis, and capsule radius/eye height.
   - Confirm LH→RH consistency for physics (either convert planes once, or convert query points consistently).

Data to persist in a runtime “Nav/Collision” cache
- For each room:
  - object_ids
  - room AABBs (from sherman_levels.tm_with_aabb)
  - floor level bands: [{height, plane_ids, accel}]
  - wall planes (near-vertical): [plane_ids, accel]
- For each planning level:
  - floor_height
  - room_ids
- Global settings: thresholds, epsilons, player capsule size.

This plan keeps physics queries scoped to the correct room and level, uses the collision mesh as the authoritative source, and leverages planning_levels and room AABBs to robustly handle multi-level buildings.
