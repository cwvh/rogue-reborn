# Development

Reference for working on the code. The README covers building and playing.

## Game data folders

The game's data is git-ignored. Copy `data/` from your game install into the directory you run
the commands from (the repository root).

| Folder | Used for |
| --- | --- |
| `data/map/` | levels (`rm01/rm01.map`...), their textures, and ambient sound (`.ass`) |
| `data/texture/` | shared textures, including bullet and glass sprites |
| `data/character/`, `data/actor/`, `data/motion/` | the characters, their faces and animations |
| `data/kit/`, `data/model/` | the weapon definition and model, and what each terrorist wears |
| `data/mission/` | spawn points, and where the terrorists start |
| `data/sound/` | all sound effects and `soundvol.txt` |

The repository carries only two maps and two face textures. Without a level's textures the
level is not drawn at all (only the glass shows). Without the character, animation and sound
folders you get no characters, no weapon animation and no sound. The viewer prints what it could
not load and carries on.

### One level on its own

From a full `data/`, `cargo run --example level-manifest` lists every file the viewer and the
server read to play a level (with one character model and one face). `scripts/pack-level.sh`
copies the files a list names into a zip that unpacks to a `data/` that plays that level and
nothing else. Run from an unpacked copy, `level-manifest` reports anything missing.
[PACK.md](PACK.md) walks through packing citystreet_large.

## Build and sound

On Windows, `build.rs` works around w64devkit's GCC 15 not shipping `libgcc_eh`.

Audio plays through the default output device. With none, sound is switched off with a message.

## Options

| Option | Effect |
| --- | --- |
| `--level NAME` | pick a level by name (`rm05`, `citystreet_large`...); `--map PATH` for a file |
| `--demo [--npcs N]` | a plain 10 m box with you and up to 8 characters (default 8) standing close by, for trying weapons and grenades |
| `--first-person` | start walking instead of in free-fly |
| `--textured` | textured instead of wireframe |
| `--spawn N` | start at the Nth mission spawn point (1-based) |
| `--difficulty easy\|medium\|hard` | which of the mission's difficulty files places its terrorists (medium by default). On the 18 RM levels the terrorists stand where the mission starts them, a team at one of its places picked at random (`--seed` fixes it), each in the model their kit names, holding its weapon and waiting in their stance. A terrorist who sees you turns and fires, and one who hears you comes to see. Elsewhere, characters stand at the spawn points |
| `--max-fps N` | cap the frame rate (turns vsync off); the default is vsync |
| `--realistic-decals` | per-surface bullet holes with splinters, sparks and dust, instead of the game's sprites |
| `--gamma N` | brighten the 3D world (1.0 changes nothing, 1.6 is a good start); the HUD and menus stay as they are |
| `--no-sound`, `--volume 0.0-1.0` | sound off, master volume |
| `--doors-open 0..1` | start with every door that far open (they start shut) |
| `--god` | take no harm and don't flinch (playing alone or a recording, not on a server) |
| `--viewmodel` | draw the gun in front of the camera (the original didn't) |
| `--record FILE` | record the game to FILE when you quit |
| `--play FILE` | play a recording back |
| `--screenshot FILE` | render a few frames, save a PNG and exit |

The client has many more, listed in `src/args.rs` (animation, camera and weapon tweaks). The
cargo aliases are in `.cargo/config.toml`: `cargo rr`, `cargo demo`, `cargo wtest` and
`cargo wcheck`.

## Controls in detail

- **Fire:** shots go within the crosshair. It opens when you start to move or change stance and
  then closes, wider on the run and narrower crouched, and while you turn. Each shot opens it
  more, and it stays open while you hold the trigger.
- **Crouch:** Left Ctrl crouches, or stands up again where there is room overhead. Crouched you
  are lower, slower and harder to hit.
- **Peek:** Q / E lean out with the head, as the original's Peek Left and Right. You stand still
  until you lean back.
- **Doors:** Space or Backspace works on the door you are looking at, within about 2 m.
- **Grenades:** you carry 3. The longer you hold left mouse, the harder the throw. A grenade
  bounces and goes off after 3 s. It kills characters within about 3 m and hurts them out to
  about 6 m. Walls shield them. It breaks glass, and shakes and deafens you if close.

## Debug keys

| Key | Effect |
| --- | --- |
| F1 | toggle first person and free-fly (collision off, Shift = faster, scroll = zoom) |
| T | textured / wireframe |
| P, C | show portals, show collision points |
| B | outline in yellow what bullets hit of each player (your own in third person or free-fly): their posed body, or without the players' model, cylinders for the legs, belly, chest and head |
| N | characters blink or not |
| H | cycle how soft sprites (shadows, light pools) are drawn, to track down a flicker: writes depth and is pulled toward the camera (the default), writes depth, no depth writes. `--sprite-mode N` starts in one |
| L | on a server: show or hide the lag meter (`--lag-meter` starts with it shown) |
| V | toggle room culling (on by default; `--no-cull` starts with it off) |
| G | color each room's geometry (the room you stand in is white) and list your rooms |
| F2 | debug dump: `debug/dump-<time>.png` and `.txt` with your position, rooms, what you look at and a command to come back to the spot. Also records the depth buffer's bits |
| F4 | clip: a dump of every frame for 2 seconds, in `debug/clip-<time>/` as `0001.png`, `0001.txt`..., for a flicker you see only in motion. Also records the sprite mode |
| F3 | reload: closes the window and starts again with the same options (a random spawn point is picked again unless you gave `--spawn` or `--seed`) |

## Recordings

`--record FILE` saves a game and `--play FILE` plays it back. A recording is how the game began
plus what the player did each tick, stored as what changed since the tick before. It is small (a
byte a tick when the mouse is still, a few when it moves, so a ten-minute game is around 100
kilobytes). Because the sim is deterministic, playing it back reproduces the game exactly.

It also holds a checksum of the whole game state for every second. A replay checks the game
against it and reports the tick where it stops matching, which means a rule, a data file or the
build has changed since it was recorded.

In a replay the keys and the mouse do nothing, but you can still change how it is drawn (T, F, G
and the rest). A replay with `--screenshot FILE` runs as fast as it can, saves a picture of the
last frame, and exits with status 1 if the game stopped matching, so a recording can be kept as a
test. Recordings work alone or on a server (see [multiplayer.md](multiplayer.md)).

## Debugging a level

- `--rooms` (or `G`) colors each room.
- `--no-cull` (or `V`) draws every room.
- `--hide-rooms 5,224` leaves rooms out.
- `--no-cull-backs` draws the faces turned away from the camera too. The original leaves them
  out, and that hides most overlapping rooms.
- `--portal-paths` draws rooms the older way, each masked to the exact openings it is seen
  through, instead of within their portals' screen rectangles (see
  [PORTALS.md](../PORTALS.md)).
- F2 writes a screenshot and a log of where you are into `debug/`.

## Tools

`examples/` has small inspectors for the data:

- `rsb-png`: texture contact sheets.
- `room-graph`, `portal-edges`: a level's rooms and portals.
- `obj-list`, `dyn-info` (doors and glass), `mat-list`, `room-info`, `motion-info`.
- `map-dump`, `map-stats`: need the `serde` and `get-size` features.
- `portal-list`: every portal's rooms, position and size.
- `room-boxes`: each room's level volumes, and which rooms hold a point.
- `floor-scan`, `floor-probe`: check that the floor stays solid around a spot once only the rooms
  near the player collide.
- `floor-walk`: walks a grid and finds steps from one room's floor onto the next where the floor
  would drop out.
- `level-manifest`: the files a level needs (see above).

`scripts/` holds checks for changes to the renderer and the game:

- `scripts/bench.sh LABEL` prints average frame times for a few fixed views.
- `scripts/shots.sh DIR` renders a fixed set of views. `python scripts/pngdiff.py OLD NEW`
  counts the pixels that differ between two such runs (see the comments in `shots.sh`).
- `scripts/golden.sh DIR` plays a few scripted runs (shooting, a grenade, a door) and records
  the log and a screenshot of each, to compare before and after a change to how the game works.

Network scripts (`mp.sh`, `net-test.sh`) are in [multiplayer.md](multiplayer.md).
