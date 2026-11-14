# Lighting

The original lights each level corner once, in software, when the level loads. We do the same. The lights come from a `.dmp` file beside the level's `.map`. `--flat-light` skips all of it and draws every texture at full brightness.

## The .dmp file

All numbers are little-endian. A string is a `u32` length and then that many bytes, with the trailing NUL counted.

Header:

| Bytes | Type | Meaning |
|---|---|---|
| 0 | u32 | version, 2 in every shipped file |
| 4 | f32 x4 | ambient color (r, g, b, a) |
| 20 | u32 | an id the editor keeps, not used |
| 24 | u32 | light count |

Each light, in this order:

1. room name, a string such as "100"
2. light name, a string such as "light100_4591"
3. kind, a `u32`: 1 directional, 2 or 3 point, 6 spot
4. direction, 3 x f32
5. position, 3 x f32, in the same space as the level's vertices
6. cone angle in degrees, f32, for spots
7. five f32 the lighting never reads
8. range, f32
9. light ambient color, 4 x f32, not used
10. diffuse color, 4 x f32, the light's color
11. specular color, 4 x f32, not used
12. orientation quaternion, 4 x f32, not used
13. one byte, not used

Every shipped file parses to exactly its length with this layout.

## The ambient light

- With no file the ambient is grey 0.5 and there are no lights.
- If r, g and b are all under 0.15 the brightest is scaled up to 0.15. If all three are near zero the ambient is grey 0.15.

## The level

An object belongs to the room its name starts with. A light lights only the objects of its own room.

Each face corner starts at the ambient color. Then each light of the room adds to it:

- Directional: `max(0, -dot(direction, normal))` times the diffuse color.
- Point: `1 - d^2 / range^2` times the diffuse color, when the corner is inside the range and faces the light. There is no N dot L.
- Spot: as a point light, but only when the surface faces against the beam and the corner is inside half the cone angle.

The sum is clamped to 0..1. The normal is the one the map gives that texture vertex. The texture is multiplied by the result. The map's own 0.5 vertex colors are ignored.

A point or spot light also lights doors named `*door*` in the next room when the door's center is within 100 units of the portal between the rooms.

Soft sprites (shadows, light pools) and materials with an emissive strength above zero are drawn unlit.

## Characters

A character is lit every frame by the level's ambient plus the two strongest lights of the room it stands in. The ambient is doubled when all its channels are under 0.16. The lights are weighed at the chest.

Each corner of the posed model takes its triangle's normal. A directional light adds `N dot L`. A point or spot light adds `N dot L` times `1 - d^2 / range^2`. Guns take one color made from the same ambient and lights, without normals.

## Gamma

The original runs the colors through a gamma table, which is the identity until the mission sets a gamma. We leave the colors as baked.
