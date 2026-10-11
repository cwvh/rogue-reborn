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
| R | reload |
| F | toggle third-person view |
| F1 | toggle between first person and free-fly (collision off, Shift = faster, scroll = zoom) |
| T | textured / wireframe |
| P, C | show portals, show collision points |
| Esc | quit |

### Useful options

| Option | Effect |
| --- | --- |
| `--level NAME` | pick a level by name (`rm05`, `citystreet_large`...); `--map PATH` for a file |
| `--first-person` | start walking instead of in free-fly |
| `--textured` | textured instead of wireframe |
| `--spawn N` | start at the Nth mission spawn point (1-based) |
| `--max-fps N` | cap the frame rate (turns vsync off); the default is vsync |
| `--realistic-decals` | per-surface bullet holes with splinters, sparks and dust, instead of the game's sprites |
| `--no-sound`, `--volume 0.0-1.0` | sound off, master volume |
| `--viewmodel` | draw the gun in front of the camera (the original didn't) |
| `--screenshot FILE` | render a few frames, save a PNG and exit |

The example `examples/wireframe.rs` has many more (animation, camera and weapon tweaks).

## Other tools

`examples/` has small inspectors for the data: `rsb-png` (texture contact sheets), `room-graph`
and `portal-edges` (a level's rooms and portals), `obj-list`, `dyn-info` (doors and glass),
`mat-list`, `room-info`, `motion-info`. `map-dump` and `map-stats` need the `serde` and
`get-size` features.
