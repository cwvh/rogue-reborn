Direct3D vs OpenGL coordinate and texture-space differences causing the “flipped/mirrored” map

Symptom observed
- The entire map looks flipped or mirrored “on the up–down plane.”
- Textures previously appeared upside down and/or reversed.

Root cause summary
1) Handedness mismatch (primary cause of whole-scene mirror)
   - Classic Direct3D content is authored in a left‑handed (LH) coordinate system: X to the right, Y up, Z forward (into the screen).
   - OpenGL’s fixed pipeline assumes a right‑handed (RH) system: X to the right, Y up, Z out of the screen; the camera looks down −Z.
   - Rendering LH vertices/normals directly in RH space without conversion produces a mirrored world (effectively reflecting across a plane, typically the Z axis).

   Practical implication:
   - If you negate Z for all positions (and normals) once, or apply a model transform that flips Z (e.g., scale(1,1,−1)), the world’s handedness matches OpenGL and the “global mirror” goes away.

2) Triangle winding/front-face definition (secondary effect once you flip axes)
   - D3D content often treats clockwise (CW) as front faces (depending on cull mode), while OpenGL’s default front face is counter‑clockwise (CCW).
   - If you flip Z (mirror) without adjusting winding/front-face state, your triangles’ facing flips (fronts become backs).
   - With culling disabled (as in your viewer), this won’t hide geometry, but lighting and “which side is front” semantics can appear inverted.
   - If you do a Z flip via scaling, you typically must also switch the front-face (e.g., to GL_CW) or reorder indices to keep fronts consistent.

3) Texture V axis origin difference (explains upside-down/reversed textures)
   - D3D UV origin is traditionally at the top‑left; OpenGL’s is bottom‑left.
   - Correct conversion is v' = 1 − v (flip V).
   - Flipping U instead (u' = 1 − u) causes a left–right mirror, not an upside‑down fix.
   - If textures still look horizontally mirrored, it’s likely because U is being flipped instead of (or in addition to) V.

How this maps to your current code and data
- Geometry: MAP vertex positions and FaceNormal planes are likely in D3D LH space. Rendering them directly in OpenGL RH space makes the world look mirrored.
- Normals/planes: When converting LH→RH, normals must also be converted (e.g., negate Z), and plane constant signs follow that transform.
- UVs: Your recent change to use (1.0 − u, v) flips U, which mirrors horizontally. The D3D→OpenGL fix usually needs V flipped, i.e., (u, 1.0 − v).

High-level fixes (conceptual; not code)
- Apply a single LH→RH conversion:
  - Positions: z = −z (once, globally).
  - Normals: nz = −nz (once, globally).
  - Planes: update distances consistently with the transformed normal.
- After applying a Z flip, restore correct front-face:
  - Either switch front-face to CW, or reorder triangle indices to preserve CCW under RH.
- Flip texture V only (v' = 1 − v). Avoid flipping U unless the asset naming/layout specifically requires it.

Net: The mirrored world look stems from feeding left‑handed Direct3D map data into a right‑handed OpenGL pipeline without converting handedness and V texture origin.
