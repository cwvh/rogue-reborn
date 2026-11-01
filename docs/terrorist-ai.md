# The original's terrorists

What the game's data and RogueSpear.exe say about how terrorists behave, read so far, and what a
first version of ours could be. Addresses are in RogueSpear.exe (see the Ghidra notes); "data" is
the game's data folder.

## What a mission gives them

Each mission's difficulty files (`data/mission/RM01Easy.mdf`, `Med`, `Hard`, 54 in all) list the
teams. A team of terrorists has:

- an actor file (`data/actor/TER_*.ter`): health, and skills out of 100: Aggression, Leadership,
  Self-control, Stamina, Sniper, Demolitions, Electronics, Assault, Grenades, Stealth;
- a kit (`data/kit/TER_*.kit`): a primary and a secondary weapon (`tw_ak47.wpn`, `sw_de357.wpn`)
  and how many magazines of each, the model they wear (a directory of `data/character` and the
  model in three levels of detail, `arab_cat1_a`..`c`), and armor;
- an initial alertness: Bored (775 teams), Ready (225) or Alert (54);
- an initial stance: Stand (951), Lean (47), Squat (23), Slouch (6), Kneel (5) (hostages Stand,
  Kneel and SitInChair);
- a radio: None, CellPhone or Earpiece;
- starting locations (x y z and a heading), of which one is picked;
- a plan, and trigger plans that take over from it.

A plan is a list of steps. Counted over every mission:

| Step | Uses | Example |
| --- | --- | --- |
| MoveTo | 7195 | `MoveTo x y z 0` (the last number is 0 or 1) |
| ROE | 3048 | two of them: `ROE Advance` / `Investigate` / `Defend` / `Cautious` / `Hostage...`, and `ROE Normal` / `Blitz` |
| GuardSpot | 343 | `GuardSpot plus 10000 1` |
| AddZone, PatrolZones, ClearZones | 232, 171, 43 | patrol the zones added |
| Cover | 124 | `Cover x y z  x y z -1`: a spot, and the spot to cover from it |
| SearchAndDestroy | 121 | `SearchAndDestroy "French Female VIP"` (a hostage, to kill) |
| PlanTrack ... EndTrack, PlanLoop ... EndLoop | 105, 97 | a patrol route, once or round and round |
| EngageHostagesOn, EscortHostage, TargetActor | 84, 33, 9 | |
| UseRadio, RadioCheckIn, PlaySound | 48, 6, 38 | |
| Snipe, Defend, ManipulateObject, KillSelf | 25, 20, 25, 33 | `Snipe x y z  x y z -1`, `Defend x y z -1` |

A trigger plan has a name, a priority (0 to 3) and the triggers that start it: DeadlyThreat (339),
ImmediateThreat (274), GloballyAware (265), ThreatInZone (212), DistantThreat (80),
GloballySuspicious (75), Sound (62), SuspiciousThreat (10). A typical guard: `ROE Advance`,
`ROE Normal`, `GuardSpot`; on ImmediateThreat or DeadlyThreat, say "Intruders", run to a spot and
`Cover` a doorway.

The exe knows more steps than the missions use, which its AI makes for itself: EngageEnemy,
FindCover, EliminateThreat, InvestigateDisturbance, HoldGround, ScanForThreats, FollowActor,
FindPath, OpenDoor, ClimbLadderUp/Down, Panic, Surrender, Berserk, Hesitate, RetreatToInsertion,
GrenadeRoom, ClearRoom, BreachDoor and others (the step names at 0x801e08..0x802130, read by the
plan reader 0x5f26b0; triggers by 0x58d1f0, ROE by 0x5f7c80, alertness by 0x613020). Sounds come in
kinds: Speech, Gunfire, Footstep, Explosion, Combat, BulletHit, Alarm, Environment (0x7fcfb8..).

Each mission's `.mis` also sets a `Max LOS Distance`: 4000 on 74 of them, 2500 to 5000 on the rest.
The levels have AI path points and cover points (the debug map draws them; not found in our map
reader yet).

## What a terrorist knows

The AI keeps several states apart, as the game's debug map colors them (0x54d250): alertness,
awareness, threat awareness, psychological state, behavior, combat state, ROE, and panicked.

## Waiting in a stance

The stances are numbered Kneel, Cower, Sit, Squat, Stand, Lean, SitInChair, Slouch (0x7ff550); an
unknown one stands. Until something happens, 0x5ceb20 plays the stance's rest, a short loop, and
now and then a fidget, picked by chance from the stance's own; after it the character rests in the
pose the fidget ends in, its `r` motion (0x5d1280: `tib_4a` then `tib_4ar`).

| Stance | Rest | Fidgets, with the chance of each and those before it | Rest between |
| --- | --- | --- | --- |
| Stand | `tib_1r` | `tib_3` .15, `tib_2` .45 (it is in the table twice), `tib_4a` .6, `tib_5`..`tib_9` .05 each, `tib_10` the rest (0x76db5c) | 190-395 |
| Lean | `til_1r` | `til_1` .3, `til_2` .6, `til_3` (0x76db38) | 190-395 |
| Squat | `tic_1r` | `tic_1` .33, `tic_2` .66, `tic_3` (0x76db24) | 240-520 |
| Kneel | `hik_1r` | `hik_1` .33, `hik_2` .66, `hik_3` (0x76db18) | 240-520 |
| Slouch | `tis_1r` | `tis_1` | |

The rest between is counted only while a rest plays (0x5c6c18), in updates of the character's
motions. A standing character whose team has some flag set (char+0xe0, +0x180) picks `tib_c1`..`c6`
instead, by the table at 0x76db44. Standing and leaning also smoke (`tib_cig1` lights up, `cig2`
puffs 5 to 8 times, `cig3` throws it away, with a `cigarette_rob` model in the hand), drink from a
`flask_sob` (`tib_11`, `til_4`) and talk in pairs (`tib_t1`..`t4`), at chances that depend on
whether they are alone. Squatting and kneeling also play `rmrc_base` and `hst_kneel_cyc` (as a base
under the rest, it seems), and a squatter without a rifle or subgun plays `tmic_*` instead.

## Seeing

Every frame (the simulation's line-of-sight stage, 0x53d140), each pair of characters is tested
both ways (0x53d360). For A seeing B:

- **Close by**, within a radius (its square is at 0x8169a0, set at run time, not read), each sees
  the other whichever way it faces.
- **Further**, B must be in A's field of view: the angle from A's facing to B within a cone whose
  cosine is at A+0xba8.
- **Range**: B is seen in full within A's sight range (A+0xbac) times how visible B is, and seen
  only partly (a glimpse) out to 1.75 times that.
- **Line of sight**: a ray from A's eye must reach B (0x561950 with flag 8, the faces sight passes),
  B being in A's room or in one A can see into (0x5639c0).
- A seeing B is noted (A's method +0x6c, 0x5ab250): who, where, and whether it was only a glimpse,
  up to 50 a frame. The brains stage decides what to do about it.

How visible B is (0x53d990): 1 standing, 0.7 crouched (posture below 4), 0.5 for posture 2, 0.4
for whatever B's method +0x80 says no to; 1.2 times that while moving; and in the dark, the light
on B (0.299 R + 0.587 G + 0.114 B) below 0.3 makes it that fraction of 0.3 as visible, unless A
sees in the dark (night vision, by the look of it).

The field of view and range (0x5aea40), from A's view angle `fov` (data+0xa8, the full angle; a
scope narrows it) and alertness:

- `k` = 0.6 Bored, 0.8 Ready, 1 Alert;
- field of view: the cone's half-angle is `k * fov / 2`;
- range: `D / tan(k * fov / 2)`, times 0.8 for some characters (data+0x50 1 or 2), 1.2 crouched,
  0.7 running or strafing at a run; where `D` is `Max LOS Distance * tan(F / 2)`, `F` the normal view
  angle (0x8169f8, set at run time, not read).

So with `fov = F`, an alert terrorist sees out to `Max LOS Distance` in a cone as wide as the view;
a bored one sees a narrower cone (60% of the angle) but further (about twice as far for a 90 degree
view), and a glimpse goes 1.75 times as far again.

## Aiming and firing

Players and terrorists aim the same way, through a spread that closes over time, as the
original's reticle does. Ours does it for players (sim/aim.rs), with the MP5K's numbers and a
skill of 90, and `v` the yaw turned in the tick over its length (0x59eb30: radians a second, times the view angle over the normal one); not yet the stun, or other guns.

The weapon file (read by 0x5dabf0) gives, after the ammo's name, the "Targetting" multipliers
for each way of moving, then three more. `tw_ak47.wpn`:

| Field | AK47 | |
| --- | --- | --- |
| +0x164 | 0.36 | Stopped |
| +0x168 | 1.01 | Shuffle (a slow walk, backing up, a side step) |
| +0x16c | 5.55 | Walk |
| +0x170 | 8.80 | Walk fast |
| +0x174 | 16.58 | Run |
| +0x178 | 0.70 | Crouched (times the one for moving) |
| +0x17c | 1.84 | recoil: the spread's growth at the first shot |
| +0x180 | 2.84 | twice the time, in seconds, the spread takes to close |
| +0x194 | 0.40 | (unread) |

The spread's size, `m` (0x5a9160, at char+0x980), with `s` the shooter's skill (char method +0xd4)
over 100:

- `m = (6 - 5s)` times the weapon's multiplier for how they move (char+0x4c8) and Crouched if
  crouched; times 8 while a flashbang stuns them (data+0x50 1 or 2, counting down, 0x5a9aa0) and
  3 when wounded (data+0x4c 1).
- The spread angle (char+0x970, 0x5a97d0) goes from `0.02 m` radians down to `0.00083 m`
  (`0.0276 m` stunned) as `(1 - t/T)^2`, `t` the time since it began to close and `T` half the
  weapon's +0x180 (0.5 s without a gun); times `1 + v^2`, `v` at char+0x550 how fast they turn,
  in radians a second. Never wider than 15 degrees.
- It starts again from the widest when how they move or their stance changes, or a stun.
- Each shot widens it by the recoil: the first by +0x17c, and each after by a growth that comes
  down toward 1 (`g = (g - 1) * 0.7 + 1`); then it closes again.
- A shot leaves within the spread (0x5a54e0): `r = (1 - u1 u2) * spread` off the aim, at a random
  angle round it, `u1`, `u2` uniform in 0..1. So most shots go out toward its edge (on average at
  3/4 of it), not into its middle.

Before it fires, a terrorist takes aim (0x5a83e0). Each frame it takes as its target the nearest
player it sees in full (not a glimpse) within a fifth of its view angle either side of where its
body faces, and its aim point (char+0x950) swings toward them from where it was, starting from
where the body faces. The swing is at most `t / T * (1.3 - s) * 10 pi` radians a second, `s` its
Assault skill, `t` how long it has been swinging at this target, up to `T`, half the weapon's
+0x180 (0.5 s without one). Once the aim is on them (char+0x555) the trigger rule below may fire.
The sight stage tests every pair every frame (0x53d140; 0x54bcf0 only leaves out the same side),
and what is seen one frame can be the target that frame: nothing in the code read so far holds a
terrorist back. Yet in the game there is a clear pause before one facing you fires: raising its
weapon from waiting. The original blends into its weapon's hold by a length set at run time
(0x816b14); from leaning and slouching it has motions for standing up with the rifle (`rmt_llup`,
36 frames, `rmt_slup`, 51). Ours plays those, or eases into aiming over half a second (a guess),
and holds fire until it is done.

The gun's skill (char method +0xd4, 0x5af310) is Assault, or toward Sniper as a scope zooms in.
The trigger's `s` (char+0x304) is Self-control. The actor's skills sit in the character from
char+0x2fc in the file's order, Aggression first: +0x308 is Stamina, +0x318 Assault, +0x31c
Grenades (a grenade's spread uses it).

A terrorist (0x5a93e0) aims at its target's head (char+0x4f4) if its skill is over 74 and
char+0x25e is at most 0.2 (unread), otherwise at the middle of the body (char+0x500, a fixed height
over the feet). It pulls the trigger (0x601610) when the spread, as wide as it is at the target
(the angle times the distance, char+0x994), is at most `15 - 7s` units, or the spread has nearly
closed (`(1 - t/T)^2` at most `0.15 - 0.14s`); and never while it is wider than 180 units there,
or while it is switching weapons (0x600c20's flags). `s` there is char+0x304, its Self-control.

For an AK in the hands of a terrorist with an Assault of 40, standing still: the spread closes
from 1.7 degrees to 0.07 in 1.4 s, and it fires once that is 12 units wide at the target (at
10 m, 1000 units, after about half a second), or nearly closed. Running: 15 degrees down to 3.2.

## What a bullet does

Whoever fires it, a bullet's effect comes from the gunfire combat model, `data/kit/gfcm.txt`
(read by 0x5822c0; worked out by 0x5833e0, applied by 0x5b2ce0):

- Each kind of ammunition (a weapon file names it after the muzzle) has a kill energy, how far it
  can kill, and how it falls off: over `d` meters it loses `d * a + d^2 * b`. A 9mm round leaves
  with 550 and is spent at 240 m.
- The energy that reaches the target picks a row of the kill chart, ten bands from 1 to 1000. For
  the part of the body struck (head, chest, abdomen, legs, arms) a roll from 1 to 100 picks what it
  does: no effect, wounded, incapacitated or dead, the higher rolls doing worse.
- A character's state only gets worse. A wound adds to their wounds, and when those reach their
  limit they are incapacitated: one more than 1 for a Stamina up to 30, 2 up to 60, 3 up to 80,
  4 up to 90, 5 above (0x5ae470, the Stamina at char+0x308; the actor's skills sit from char+0x2fc
  in the file's order, Aggression first). Three wounds to the legs incapacitate too.
- Also there, not used yet: the stun energy and its chart (dazed, stunned), armor (`TR_ARM1.eq`
  and the like), which turns a share of the kill energy into stun, a first number per ammunition
  not yet known (0.8 for 9mm), and a roll doubled for some head shots (when, depends on arguments
  not pinned down).

Ours (gfcm.rs): health is still the 100 shown, and a wound takes its share (a third for a
terrorist with a Stamina of 50, a fifth for a player, whose Stamina is taken as 85); being
incapacitated is dying.

## Hearing

The original's sounds have kinds (the table at 0x7fcf94): Environment, Alarm, BulletHit, Combat,
Door, Explosion, Footstep, Gunfire, Speech. Mission plans react to them by triggers (`Trigger
Sound Alarm`, 55 of them, and `Trigger Sound Combat`, read by 0x58d1f0), beside the threat
triggers (GloballySuspicious, GloballyAware, SuspiciousThreat, DistantThreat, ImmediateThreat,
DeadlyThreat, ThreatInZone). How far a sound carries to a character, and what it does to one
without a trigger, is not read yet. `soundvol.txt` gives each sound how far it is heard: an
unsuppressed gun 10000 to 12500, a suppressed one 1000, footsteps 500 to 1500 by pace and gear
(`F_TW`/`F_TR`, walking and running; `INT`, `LEQ`, `HEQ`, no, light and heavy equipment).

Ours (sim::terrorist): every sound the game makes at a place, the players' shots and their
footsteps are heard by the terrorists within that reach, and through walls within half of it (a
guess). Footsteps, four a second, are heard half as far as they sound for light gear (walking
400, running 500), never through walls, and creeping not at all: heard to the sounds' full reach,
every step made the terrorists far too sharp. One that hears something becomes alert and turns toward it with
its weapon up, for ten seconds (a guess), so it is quicker to fire on whoever comes.

What it does then is its team's plan's (mission.rs reads them: the plan's `ROE` lines, its steps,
and its trigger plans). With no one to fight, Investigate and Advance go to see, at a walk or, with
Blitz, at a run; Cautious creeps there; Defend stays put. Each terrorist grades how sure it is of
a threat as the triggers do: Suspicious for a quiet sound, Distant for gunfire or a blast (a sound
that carries 5000 or more), Immediate when it sees an enemy, Deadly when it is shot or one of its
team is killed. A team's grade is its members' highest; GloballySuspicious and GloballyAware are
anyone's Suspicious and Immediate. When one of a trigger plan's conditions holds, the team turns
to it, once, the lowest Priority first (and `RemoveSamePriorityTriggers` rules out the others of
its priority); `Sound Combat` is having heard gunfire, and `ThreatInZone N` a threat known to be in
zone N: a player a terrorist is fighting, or something one has just heard, in one of its rooms.
Alarms are not there yet.

A plan's steps are its way (`Plan::route`): `MoveTo` places and `PatrolPath` points to go to in
turn, `PlanLoop`/`EndLoop` round and round, `PlanTrack`/`EndTrack` once, `ChangeSpeed` units a
second (under 350 a creep, under 650 a walk, else a run) and `GuardSpot`, where it stops and stays.
A terrorist with nothing else to do walks its plan's way from doorway to doorway, at its plan's
speed (a walk, or with Blitz a run); going to see what it heard takes it off it, and it walks back
to the step it left once it has stopped listening; turning to a trigger plan starts that plan's
way from the top. `MoveTo`'s last number (0 or 1, 1 mostly where the way climbs) is not known.
A mission's zones are sets of its rooms (the `.mis`'s `ZoneRooms`, a room and its zone a line).
`AddZone` and `ClearZones` set the zones a team patrols, and `PatrolZones` sends it from room to
room of them, to the middle of one picked at random, staying three seconds (a guess) in each, on
and on. `Cover` and the rest are not taken yet.

## Walking

The original's path points are not found in our reading of the level files yet. Ours finds its way
through the rooms' doorways (`Visibility::path`): the shortest way from portal to portal standing
upright, not shut by a closed door, walking straight across each room, at a player's walking speed
(514 units a second), on its feet, keeping to the floor and up stairs. It does not yet keep clear
of furniture, open doors, or play a walk of its own weapon's.

## Not read yet

- How a sighting becomes a threat: what makes it Suspicious, Distant, Immediate or Deadly, and
  how awareness rises and falls (the brains stage, 0x53d030 and on).
- How fast a terrorist turns to face a target, what holds back its trigger, how it fires
  (bursts, the rate of fire) and reloads, and which skill it aims with (Assault, or Sniper with a
  sniper rifle).
- Hearing: how far each kind of sound carries to a character, and what it starts without a trigger.
- Moving: the path points, doors, and the ROE's speeds (Normal, Blitz).
- The two values set at run time: the close radius and the normal view angle.

## A first version

Small enough to try out on the maps we play, and grow from:

1. Terrorists from the mission files, where the mission puts them: the team's actor, kit, stance,
   alertness and one of its starting spots. Done (mission.rs, kit.rs, characters::place_terrorists,
   sim::idle): a team at one of its places, facing its heading (256ths of a turn from +z, which
   examples/mission-headings.rs settles: that way 12 of 574 face a wall within 100 units, against
   33 to 90 any other way), its members side by side, each in the model their kit names with their
   actor's textures, the kit's weapon (the primary, or else the secondary) in the right hand,
   waiting in their stance with its rests and fidgets. Still to do: smoking, drinking and
   talking, dropping the weapon on dying, and hit reactions that start from their stance.
2. Seeing, as above: field of view and range from their alertness and the mission's Max LOS
   Distance, the player's visibility (crouched, moving) and line of sight. Done (sim::terrorist),
   but for glimpses (a player they are already after they keep seeing out to the glimpse range)
   and the light. Guessed: a normal view of 90 degrees and a close radius of 200 units. Seeing a
   player makes them alert, and so does being shot by one, who becomes their target.
3. Reacting to a player in view: turn toward them and fire. Done: they turn at 180 degrees a second
   (a guess), aim through the spread above with their weapon's numbers and their Assault skill,
   and pull the trigger by the rule above once they face the player, at their head if they shoot
   well and else their middle. The spread opening with each shot makes bursts of the automatics.
   Shots are as fast as the kind of weapon (guesses: 600 a minute for automatics, a pistol 0.3 s,
   a sniper's rifle 0.6 s, a shotgun 0.8 s) and hurt as players' bullets do, with the weapon's own
   sound. They reload when the magazine is empty, from the magazines the kit gives them.
   They stand, or kneel when squatting or kneeling, with the gun up as a player fires.
   Still to do: the delay the original may add before reacting, a reload and the weapons' own
   holds in their bodies, and shotguns' pellets. Bullets do what the gunfire combat model says
   (above).
4. Then hearing (gunfire, footsteps), the guard and patrol plans (GuardSpot, MoveTo loops),
   triggers, and moving to cover.
