# rogue-reborn

A reimplementation of Tom Clancy's Rainbow Six: Rogue Spear. It reads the original
game's data files (maps, textures, characters, animations, sounds) directly, so you need your
own copy of the game.

What works today:

- **The levels**, drawn with the original's textures, rooms and portals. You walk them in first
  person, with collision, doors that open and glass that breaks.
- **The MP5K and frag grenades**, with the original's spread, damage and wound reactions.
- **The missions' terrorists**, placed as the original places them. They see and hear you, turn
  on you and fire, and come to look when they hear something.
- **Network play**: a server, players and bots, free for all or in two teams, with rounds of
  Team Survival or Survival.
- **Recordings** that play back exactly, alone or on a server.

The design notes are in [docs/](docs/): [multiplayer.md](docs/multiplayer.md) for network play
and the match flow, [terrorist-ai.md](docs/terrorist-ai.md) for the terrorists, and
[walking.md](docs/walking.md) for how players and characters move.

.MAP and .RSB parsing credit to [RSE-file-formats](https://github.com/AlexKimov/RSE-file-formats)
by Alexander Evdokimov. I adapted his 010 templates to Rust for initial map loading and
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
| `data/kit/`, `data/model/` | the weapon definition and model, and what each terrorist wears |
| `data/mission/` | spawn points, and where the terrorists start |
| `data/sound/` | all sound effects and `soundvol.txt` |

The repository itself only carries two maps and two face textures. That is **not enough to
see anything**: without a level's textures the level is not drawn at all (only the glass
shows), and without the character, animation and sound folders you get no characters, no
weapon animation and no sound. The viewer prints what it could not load and carries on.

### One level on its own

From a full `data/`, `cargo run --example level-manifest` lists every file the viewer and the
server read to play a level (with one character model and one face), and
`scripts/pack-level.sh` copies the files a list names into a zip that unpacks to a `data/` that
plays that level and nothing else. Run from an unpacked copy, `level-manifest` reports anything
missing. [docs/PACK.md](docs/PACK.md) walks through packing citystreet_large.

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
cargo rr --level rm11 --first-person --textured   # any level, any options
cargo rr --list-levels                            # what is in data/map
cargo wtest                                       # run the tests
```

On other platforms, or without the aliases:
`cargo run --release --bin rr -- --first-person --textured --spawn 1`.

### Controls

| Input | Action |
| --- | --- |
| W A S D, mouse | move, look |
| Left mouse | fire (hold for full auto). Shots go within the crosshair, which opens when you start to move or change stance and then closes, wider on the run and narrower crouched, and while you turn; each shot opens it more, and it stays open while you hold the trigger |
| Right mouse (hold) | walk instead of run |
| Left Ctrl | crouch, or stand up again (where there is room overhead). Crouched you are lower and slower, and harder to hit |
| Q / E (hold) | peek left / right round a corner, as the original's Peek Left and Right: the view leans out with the head, and you stand still until you lean back |
| Space or Backspace | open or close the door you are looking at (within about 2 m) |
| R | reload |
| 1 / 3 | the gun / a frag grenade (3 of them). With a grenade out, hold left mouse to wind up and let go to throw: the longer the hold, the harder the throw. It bounces, goes off after 3 s, kills characters within about 3 m (hurts them out to about 6 m, walls shield them), breaks glass, and shakes and deafens you if close |
| F | toggle third-person view |
| F1 | toggle between first person and free-fly (collision off, Shift = faster, scroll = zoom) |
| T | textured / wireframe |
| P, C | show portals, show collision points |
| B | show what bullets hit of each player, outlined in yellow (your own in third person or free-fly): their posed body, arms and all, or without the players' model, cylinders for the legs, belly, chest and head |
| N | characters blink or not |
| H | cycle how soft sprites (shadows, light pools) are drawn, to track down a flicker: writes depth and is pulled toward the camera (the default), writes depth, no depth writes. F2 and F4 record the mode and the depth buffer's bits, and `--sprite-mode N` starts in one |
| L | on a server: show or hide the lag meter (`--lag-meter` starts with it shown) |
| V | toggle room culling (on by default; `--no-cull` starts with it off) |
| G | color each room's geometry (the room you stand in is white) and list your rooms |
| F2 | debug dump: writes `debug/dump-<time>.png` and `.txt` with your position, rooms, what you are looking at and a command to come back to the spot |
| F4 | clip: a dump (screenshot and log) of every frame for 2 seconds, in `debug/clip-<time>/` as `0001.png`, `0001.txt`..., for finding a flicker you see only in motion |
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
| `--difficulty easy\|medium\|hard` | which of the mission's difficulty files places its terrorists (medium by default). On the 18 RM levels the terrorists stand where the mission starts them, a team at one of its places picked at random (`--seed` fixes it), each in the model their kit names, holding its weapon and waiting in their stance. A terrorist who sees you turns and fires, and one who hears you comes to see; elsewhere, characters stand at the spawn points |
| `--max-fps N` | cap the frame rate (turns vsync off); the default is vsync |
| `--realistic-decals` | per-surface bullet holes with splinters, sparks and dust, instead of the game's sprites |
| `--no-sound`, `--volume 0.0-1.0` | sound off, master volume |
| `--doors-open 0..1` | start with every door that far open (they start shut) |
| `--god` | take no harm, but flinch at hits as usual (playing alone or a recording, not on a server) |
| `--viewmodel` | draw the gun in front of the camera (the original didn't) |
| `--record FILE` | record the game (a byte a tick when the mouse is still, a few when it moves) to FILE when you quit |
| `--play FILE` | play a recording back; it starts as the game did, and checks the game against the recording every second |
| `--screenshot FILE` | render a few frames, save a PNG and exit |

The client has many more, listed in `src/args.rs` (animation, camera and weapon tweaks).

## Demos

`--record FILE` saves a game and `--play FILE` plays it back. A recording is how the game began
plus what the player did each tick, stored as what changed since the tick before, so it is small
(a ten-minute game is around 100 kilobytes) and, because the sim is deterministic, playing it back reproduces the game exactly. It
also holds a checksum of the whole game state for every second, and a replay reports the tick
where it stops matching, which means a rule, a data file or the build has changed since it was
recorded. In a replay the keys and the mouse do nothing, but you can still change how it is
drawn (T, F, G and the rest). A replay with `--screenshot FILE` runs as fast as it can, saves a
picture of the last frame, and exits with status 1 if the game stopped matching, so a recording
can be kept as a test.

## Playing over a network

A server owns the game and runs it for everyone. Players send it what they do, and it sends back
what happened.

### Hosting and joining

```
rrsrv --demo --max-players 4                  # the box room, or --level rm11 for a level
rr --connect 127.0.0.1:27960 --first-person --textured
rr --connect 127.0.0.1:27960 --first-person --textured
```

`rrsrv` listens on UDP port 27960 (`--port` changes it) for up to 16 players (`--max-players`).
Build `rr` and `rrsrv` together, from the same commit: the server turns away a client built from
another, because the two must move a player the same way.

The server places a level's mission terrorists (medium difficulty) where it has a mission, or
`--npcs` characters in the box room. Players can hurt each other (`--no-pvp` turns it off): a
head shot kills, a body shot takes a fifth, a grenade by how close it is. Without rounds, a player
who dies comes back after 3.7 seconds at the spawn point farthest from the others.

### Team games and rounds

```
rr --host --level citystreet_large --first-person --textured
rr --connect HOST:27960 --side blue --first-person --textured
```

`rr --host` starts a Team Survival server and joins it as the host. Between rounds everyone
waits in the lobby, laid out after the original's session screen. Players pick a side and say
ready there, and the host sets the round length, friendly fire and the bases, and starts each
round. Closing the host's window ends the game. A dedicated server, `rrsrv --level
citystreet_large --teams --rounds`, starts each round by itself instead.

- `--teams` puts players on two sides, Blue and Gold, starting from a pair of the level's bases
  (`--base-pair N`, or one the seed picks). `--side blue|gold` on `rr` and `rrbot` asks for a
  side; a player goes to the smaller side if theirs has more. Blue wears Blue Medium and Gold
  Desert 1 Medium, as ladder play had it.
- Friendly fire is on, as in competitive play. `--no-friendly-fire` turns it off.
- `--rounds` plays in rounds with one life each: Team Survival with `--teams`, last one standing
  without. A round starts with a 5 second countdown, from the host or once both sides have
  someone. Everyone is put back in their base and held for 2 seconds, then let go. The round
  lasts 3 minutes (`--round-time SECONDS`, 0 for no limit). It ends when one side is left, or in
  a draw when the time runs out or no one is left. Then 8 seconds of results, and the lobby.
- A team game has no terrorists.

### How it stays fair

Snapshots go out 60 times a second, as only what changed since the last one the client has.
Other players are drawn about 25 ms in the past, moving smoothly between snapshots. Your own
movement happens at once on your screen; the server checks it, and rarely moves you back by a
little. Everything else is the server's word: shots, hits, grenades, doors, glass, kills.

A shot is judged against where the others were when you saw them, up to 0.75 seconds back, so
there is no need to lead a target by your ping. `rrsrv --no-lag-compensation` judges against
where they are now instead.

To see how the game copes with a poor connection, add `--sim-latency 80 --sim-jitter 30
--sim-loss 5` to `rrsrv` or `rr` (milliseconds each way, milliseconds of jitter, percent lost).

### The lag meter

L shows the lag meter at the top left, over the last 4 seconds:

- **ping:** from sending an input until a snapshot says the server used it. This includes the
  inputs the server holds back against late ones, so it is the lag you feel.
- **jitter:** the spread of the snapshots' arrival times.
- **loss:** snapshots that never came.
- **view:** how far in the past the other players are shown.
- **fixes:** how often the server moved you from where your computer predicted, and by how much
  at most.
- **in, out:** bytes a second.

The graph's upper band is how much the view of the other players has in hand: blue, or yellow
when it runs dry and they stall. The lower band has a bar for each snapshot: green to its ping,
red for one lost, and yellow from the top for a fix.

### Recording a network game

`rrsrv --record match.rrs` records the match as the server ran it, and `rrsrv --check match.rrs`
plays it back and says whether it comes out the same. `rr --record FILE` on a server records
what that player saw, and M marks a moment in it to find later. `rrsrv --check` plays those back
too.

### Bots

To play alone against the computer, `scripts/mp.sh` starts a server, some bots and your own
window, and stops them all when you close it:

```
scripts/mp.sh                              # the box room, three bots
scripts/mp.sh rm11 --bots 5 --skill hard   # a level, five hard bots
scripts/mp.sh rm08 --net bad               # your connection 90 ms each way, 30 ms jitter, 4% lost
```

`--net` is `clean`, `good`, `typical`, `bad` or `awful`, and makes only your own connection worse.
`--skill` is `easy`, `normal` or `hard`, `--npcs N` and `--no-pvp` go to the server, and anything
after `--` goes to `rr`.

The bots are `rrbot --connect HOST:PORT --fight [--skill hard] [--side gold]`. They wander the
level, opening the doors they come to, and turn on whoever they see on the other side: they react
after a moment, turn at a limited rate, fire in bursts with an aim that drifts and settles,
sidestep and now and then throw a grenade. They see only what a player on the same computer
would, so their shots are judged the same way. They are poor at finding each other on a big
level. `--trace` prints where a bot is and whom it fights, once a second.

`rrbot` without `--fight` is a scripted client (`rrbot --connect HOST:PORT --keys WF --seconds
10`), and `scripts/net-test.sh` runs a server and two bots over real sockets and checks that the
game stayed in agreement.

### The lab

`rrlab` plays bots against each other over pretend connections, in moments, and prints how fair
and smooth it was: how often a shot's result on the shooter's screen matched the server's, how
far the victim was from where the shot found them, how much sooner a player stepping round a
corner sees the one waiting than they are seen, and the bytes sent. `rrlab --tune` tries the
snapshot rate, the view delay and how far back shots are judged against each other. Tests hold
its figures, so a change that makes play less fair fails them. See
[docs/multiplayer.md](docs/multiplayer.md) for the figures and how they were tuned.

## How the code is laid out

The game itself is `src/sim/`, part of the library: the player, weapon, grenades, characters,
doors and glass, and what a tick of time does to them. It reads no clock, window or sound
device, and takes its randomness from its own generator, so the same inputs always give the
same game (a test checks this). Each tick takes an `Input` and returns `Event`s.

The server, client, wire format, rounds, recordings and lab are `src/net/`, tested against a
pretend network with a pretend clock. The client, `src/main.rs` and its modules, is a window around that: it turns keys and the mouse
into `Input`, steps the sim in fixed 60 Hz ticks, and draws, plays sounds and prints the events.

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
- `scripts/golden.sh DIR` plays a few scripted runs (shooting, a grenade, a door) and records the
  log and a screenshot of each, to compare before and after a change to how the game works.

For debugging a level, `--rooms` (or `G`) colors each room, `--no-cull` (or `V`) draws every
room, `--hide-rooms 5,224` leaves rooms out, and F2 writes a screenshot and a log of where you
are into `debug/`. `--no-cull-backs` draws the faces turned away from the camera too (the
original leaves them out, and that hides most overlapping rooms). `--portal-paths` draws rooms the older way, each masked to the exact openings
it is seen through, instead of within their portals' screen rectangles (see PORTALS.md).
