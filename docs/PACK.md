# Packing a level

A pack is a zip holding only the files needed to play one level. A player unpacks it beside
`rr.exe` and gets a `data/` directory that plays that level and nothing else, without the rest of
the game's data. This walks through packing `citystreet_large`.

You need the full game `data/` in the repo root, and Git Bash (the pack script is a shell
script). Run everything from the repo root.

## 1. Build

```bash
cargo build --release --target x86_64-pc-windows-gnu --bin rr --example level-manifest
```

## 2. Make the manifest

The manifest lists every file the level needs, one path per line. `level-manifest` finds each
file the way the game does: the map and its textures, ambience, door sounds, mission spawn points,
the character model with its textures and motions, one face, the gun, the grenade, the sprites
and the sounds.

```bash
mkdir -p manifests
target/x86_64-pc-windows-gnu/release/examples/level-manifest.exe \
    data/map/m00/citystreet_large.map > manifests/citystreet_large.txt
echo $?
```

An exit status of 0 means every file was found. Anything missing is printed as a `missing:` line,
and the status is 1. `note:` lines point out sounds whose name differs in case from the file on
disk. Windows doesn't care, but Linux and macOS won't find them.

The character model and face can be chosen too:

```bash
level-manifest.exe MAP --model data/character/team_hrt/blue_M_a.CRP --actor Chavez
```

Those two are the defaults, and so is citystreet_large for the map. Write the manifest from Git
Bash: PowerShell's `>` adds a byte-order mark or writes UTF-16, and the pack script can't read
either.

## 3. Pack

```bash
scripts/pack-level.sh manifests/citystreet_large.txt citystreet_large.zip
```

This copies the listed files into a staging folder and zips it, so the zip unpacks to `data/`. It
stops without packing if any listed file is missing. The zip name is optional and defaults to the
manifest's name. Zips in the repo root and the `manifests/` folder are git-ignored. citystreet_large
packs to about 1.6 MB.

## 4. Check the pack

Unpack it into an empty folder, as a player would:

```bash
mkdir /c/tmp/player && cd /c/tmp/player
/c/Windows/System32/tar.exe -xf /c/dev/src/rogue-reborn/citystreet_large.zip
```

Run the manifest tool from there. It looks only at the unpacked `data/`, so an exit status of 0
means the pack is complete:

```bash
/c/dev/src/rogue-reborn/target/x86_64-pc-windows-gnu/release/examples/level-manifest.exe > /dev/null; echo $?
```

Then play it:

```bash
/c/dev/src/rogue-reborn/target/x86_64-pc-windows-gnu/release/rr.exe --first-person --textured
```

To check that files loaded, watch the console for `missing`, `could not load`, `no motion` or
`no texture`. Some sounds load only when they first play, so fire, reload, throw a grenade and
hit a character. The dedicated server runs from the same folder with
`rrsrv.exe --level citystreet_large`.

## Shipping it

Give players the zip, `rr.exe` (and `rrsrv.exe` for hosting), and any MinGW runtime DLLs the
build needs. They unpack the zip next to the executables and run `rr.exe` from that folder.
