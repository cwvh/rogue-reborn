First-Person Walking and Ground Snap

This document explains the first‑person (FPS) walking and ground snapping used by examples/opencode.rs so you can understand and re‑implement it yourself. The system is deliberately simple and relies on the actual level geometry (triangles) rather than the auxiliary collisions list.


1. Coordinates and Z-Handedness

- The MAP geometry is authored in a coordinate system that is effectively the opposite handedness on the Z axis from how we place the camera in OpenGL.
- For rendering, we flip handedness once with: glScalef(1, 1, -1).
- For all physics and queries, we work in MAP space. That means when we take camera positions (which are in the post-flip camera space), we convert Z to MAP space by flipping the sign:
  - z_tri = -z_cam
- Do this consistently whenever you query triangles (ground find, wall collision). After calculating results in MAP space, convert back to camera space if you need to update the camera position on Z.


2. Extracting Triangles From Geometry

- Iterate over level.geometries.objects and then each ObjectData in object.object_datas.
- For each triangle index (i0, i1, i2), gather p0, p1, p2 from object.vertices.
- The MAP supplies a FaceNormal per face with components (nx, ny, nz) and a distance value distance_origin_to_face. Normalize the plane:
  1) len = sqrt(nx^2 + ny^2 + nz^2)
  2) n = (nx/len, ny/len, nz/len)
  3) d = distance_origin_to_face / len
  The plane is then n · p + d = 0. This ensures distances are in world units.
- Store each tri as: { p0, p1, p2, n, d }.


3. Walkable Slope Test

- Define a maximum walkable slope angle, e.g., max_slope_deg = 50.
- Compute ny_thresh = cos(max_slope_deg in radians).
- A face is considered walkable if abs(n.y) >= ny_thresh.


4. Ground Solve (Find Floor Under Player)

Given camera X,Z in camera space, convert to MAP space: zq = -z_cam.

For each triangle tri:
1) Skip if not walkable: abs(n.y) < ny_thresh.
2) Solve the plane for Y at (x, zq):
   y = (-d - n.x * x - n.z * zq) / n.y
   Skip if |n.y| is too small (near vertical).
3) Test that the point p = (x, y, zq) lies inside the triangle on its plane using a barycentric test:
   - Compute v0 = c - a, v1 = b - a, v2 = p - a
   - Compute the dot products and then u,v barycentric coordinates.
   - Inside if u >= -eps, v >= -eps, and u + v <= 1 + eps.
4) Track best candidates:
   - best_below: the highest Y that is <= current_y
   - best_above: the lowest Y that is > current_y but within a small step height (step_up)

Pick best_below if it exists, otherwise best_above. If neither exists directly underfoot, do a small radial search around (x, zq) and repeat the process until you find a candidate. The snap target ground_y is the selected Y.

Finally, place the camera’s Y to ground_y + eye_height (we use 25.0 for eye_height).

Constants used:
- eye_height = 25.0
- max_slope_deg = 50.0 → ny_thresh = cos(50°)
- step_up ≈ 45.0 (allow stepping up small height differences)


5. Wall Collision Resolution

Walls are triangles that are near-vertical (|n.y| <= ny_wall_max). Use a small ny_wall_max like 0.3.

Algorithm (in MAP space):
1) Convert Z to MAP space for both old and new camera positions:
   - old_zq = -old_pos.z, new_zq = -new_pos.z
2) For each near-vertical tri, compute signed distances to its plane at a reference height y_ref (e.g., mid-body height = camera_y - eye_height * 0.5):
   - s_old = n.x * old_x + n.y * y_ref + n.z * old_zq + d
   - s_new = n.x * new_x + n.y * y_ref + n.z * new_zq + d
3) If we moved from back side (s_old <= -radius) to front side (s_new > -radius), we crossed into the wall. Compute the intersection parameter t along the horizontal move:
   - denom = n.x * dx + n.z * dzq
   - t = (-radius - s_old) / denom (guard denom != 0 and clamp t to a reasonable range)
4) Check the intersection point’s XZ (hx, hzq) is inside the triangle (use a 2D XZ barycentric test).
5) Push the camera back along the horizontal part of the normal by the penetration amount:
   - penetration = s_new + radius
   - horizontal normal is (n.x, n.z). Normalize its length and subtract penetration from (x, zq).
6) After all contacts, convert back to camera Z: new_pos.z = -new_zq.

Constants used:
- player_radius ≈ 30.0
- ny_wall_max ≈ 0.3


6. Barycentric Point-In-Triangle Tests

3D-on-plane test (for ground):
- Use the v0/v1 basis on the triangle’s plane and compute barycentric coordinates (u, v) relative to the triangle. Include a tiny epsilon so points on edges are accepted.

2D XZ test (for walls):
- Project (x, zq) and each tri vertex (x, zq) into 2D and repeat the same barycentric logic.


7. Putting It Together Per Frame

1) Compute intended movement from inputs (WASD), keep Y unchanged for now.
2) Resolve walls: apply horizontal correction based on near-vertical triangles in MAP space.
3) Solve ground: find ground_y under the corrected XZ (with radial fallback), then set camera_y = ground_y + eye_height.
4) Render with a single glScalef(1, 1, -1) so visuals are right-handed for OpenGL.


8. Debugging Aids

- Print logs when FPS toggles on and whenever the supporting ground triangle changes (index, normal, position).
- Draw the supporting ground triangle as a colored overlay and show a vertical line from eye to ground.
- If no ground tri is found under player, print it once until a ground tri is found again.


9. Common Pitfalls

- Forgetting the Z flip in queries: You’ll pick the wrong triangles or miss them entirely. Always flip z_cam -> z_tri = -z_cam for physics queries.
- Not normalizing plane normals and d: Plane distances won’t match world units and pushes/snaps will be wrong. Normalize (n, d) per face once when building triangles.
- Too strict slope or epsilon: You can miss valid walkable surfaces. Use small epsilons for barycentric checks and a practical slope limit (e.g., 50°).


10. Future Optimizations and Improvements (Hints)

- Spatial index: Partition triangles into a uniform grid or BVH to reduce the per-frame tri checks from O(N) to something near O(1) average.
- Stickiness and hysteresis: Prefer the previous ground tri when possible to avoid flicker at seams; introduce a small y tolerance.
- Gravity: Add a slight downward velocity and only snap upward within step_up. Helps walking off edges and settling smoothly.
- Wider radial fallback: Expand the search radii and sample count or do a short downward ray cast if your content has holes.
- Material/room filters: Once confident, filter candidates by room or material if that’s semantically meaningful and reliable for your content.
