# rogue-reborn

A first-person viewer and sandbox for the levels of Tom Clancy's Rainbow Six:
Rogue Spear.  It reads the original game's data files (maps, textures,
characters, animations, sounds) directly, so you need your own copy of the
game.

.MAP and .RSB parsing credit to [RSE-file-formats](https://github.com/AlexKimov/RSE-file-formats)
by Alex Kimov. I adapted his 010 templates to Rust for initial map loading and
wireframe rendering in 2023. As of 2026 frontier models have advanced enough
that figuring out the binary formats is no longer the headache it once was.

## Game data

The game's data is not included and is git-ignored. Copy the `data/` folder from your game
install into the directory you run the commands from (the repository root), so you have
`data/map/`, `data/texture/` and so on next to `Cargo.toml`.

| Folder | Used for |
| --- | --- |
| `data/map/` | levels (`rm01/rm01.map`...), their textures, and ambient sound (`.ass`) |
| `data/texture/` | shared textures, including bullet and glass sprites |
| `data/character/`, `data/actor/`, `data/motion/` | the characters, their faces and animations |
| `data/kit/`, `data/model/` | the weapon definition and model |
| `data/mission/` | spawn points |
| `data/sound/` | all sound effects and `soundvol.txt` |

The repository itself only carries two maps and two face textures. That is **not enough to
see anything**: without a level's textures the level is not drawn at all (only the glass
shows), and without the character, animation and sound folders you get no characters, no
weapon animation and no sound. The viewer prints what it could not load and carries on.

## Building

You need Rust (a recent stable, the crate uses edition 2024) and CMake, because GLFW is built
from source.

- **Windows:** the default MSVC target needs Visual Studio just to build GLFW, so this
  project uses the MinGW target. Install [w64devkit](https://github.com/skeeto/w64devkit)
  (put its `bin` on your `PATH`) and run `rustup target add x86_64-pc-windows-gnu`. The cargo
  aliases in `.cargo/config.toml` already pass the target. `build.rs` works around w64devkit's
  GCC 15 not shipping `libgcc_eh`.
- **Linux:** install CMake and the GLFW build dependencies (`libxrandr-dev libxinerama-dev
  libxcursor-dev libxi-dev libgl1-mesa-dev` on Debian/Ubuntu). Not tested recently.
- **Audio:** playback goes through the default output device. With none, sound is switched
  off with a message.

## Running

```
cargo demo
```

starts the first-person demo on the default level (citystreet_large) at spawn point 1 with
textures, realistic bullet holes and a 144 fps cap. Other shortcuts:

```
cargo wireframe --level rm11 --first-person --textured   # any level, any options
cargo wireframe --list-levels                            # what is in data/map
cargo wtest                                              # run the tests
```

On other platforms, or without the aliases:
`cargo run --release --example wireframe -- --first-person --textured --spawn 1`.

### Controls

| Input | Action |
| --- | --- |
| W A S D, mouse | move, look |
| Left mouse | fire (hold for full auto) |
| Right mouse (hold) | walk instead of run |
| Space or Backspace | open or close the door you are looking at (within about 2 m) |
| R | reload |
| 1 / 3 | the gun / a frag grenade (3 of them). With a grenade out, hold left mouse to wind up and let go to throw: the longer the hold, the harder the throw. It bounces, goes off after 3 s, kills characters within about 3 m (hurts them out to about 6 m, walls shield them), breaks glass, and shakes and deafens you if close |
| F | toggle third-person view |
| F1 | toggle between first person and free-fly (collision off, Shift = faster, scroll = zoom) |
| T | textured / wireframe |
| P, C | show portals, show collision points |
| V | toggle room culling (on by default; `--no-cull` starts with it off) |
| G | colour each room's geometry (the room you stand in is white) and list your rooms |
| F2 | debug dump: writes `debug/dump-<time>.png` and `.txt` with your position, rooms, what you are looking at and a command to come back to the spot |
| F3 | reload: closes the window and starts again with the same options, so a level is back as new (a random spawn point is picked again unless you gave `--spawn` or `--seed`) |
| Esc | quit |

### Useful options

| Option | Effect |
| --- | --- |
| `--level NAME` | pick a level by name (`rm05`, `citystreet_large`...); `--map PATH` for a file |
| `--demo [--npcs N]` | a plain 10 m box with you and up to 8 characters (default 8) standing close by, for trying weapons and grenades |
| `--first-person` | start walking instead of in free-fly |
| `--textured` | textured instead of wireframe |
| `--spawn N` | start at the Nth mission spawn point (1-based) |
| `--max-fps N` | cap the frame rate (turns vsync off); the default is vsync |
| `--realistic-decals` | per-surface bullet holes with splinters, sparks and dust, instead of the game's sprites |
| `--no-sound`, `--volume 0.0-1.0` | sound off, master volume |
| `--doors-open 0..1` | start with every door that far open (they start shut) |
| `--viewmodel` | draw the gun in front of the camera (the original didn't) |
| `--screenshot FILE` | render a few frames, save a PNG and exit |

The example `examples/wireframe.rs` has many more (animation, camera and weapon tweaks).

## Other tools

`examples/` has small inspectors for the data: `rsb-png` (texture contact sheets), `room-graph`
and `portal-edges` (a level's rooms and portals), `obj-list`, `dyn-info` (doors and glass),
`mat-list`, `room-info`, `motion-info`. `map-dump` and `map-stats` need the `serde` and
`get-size` features.

For rooms and portals: `portal-list` (every portal's rooms, position and size), `room-boxes`
(each room's level volumes, and which rooms hold a point) and `floor-scan` / `floor-probe`
(check that the floor stays solid around a spot once only the rooms near the player collide) and
`floor-walk` (walks a grid and finds steps from one room's floor onto the next where the floor
would drop out).

`scripts/` holds checks for changes to the renderer:

- `scripts/bench.sh LABEL` prints average frame times for a few fixed views.
- `scripts/shots.sh DIR` renders a fixed set of views, and `python scripts/pngdiff.py OLD NEW`
  counts the pixels that differ between two such runs (see the comments in `shots.sh`).

For debugging a level, `--rooms` (or `G`) colours each room, `--no-cull` (or `V`) draws every
room, `--hide-rooms 5,224` leaves rooms out, and F2 writes a screenshot and a log of where you
are into `debug/`.
