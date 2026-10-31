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
  and how many magazines of each, and armor;
- an initial alertness: Bored (775 teams), Ready (225) or Alert (54);
- an initial stance: Stand (951), Lean (47), Kneel (23), Squat (15), SitInChair (12), Slouch (6);
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

## Not read yet

- How a sighting becomes a threat: what makes it Suspicious, Distant, Immediate or Deadly, and
  how awareness rises and falls (the brains stage, 0x53d030 and on).
- How long a terrorist takes to react, how it turns and aims, how accurate it is (the skills, and
  the weapon's numbers: `tw_ak47.wpn` lists 0.36, 1.01, 5.55, 8.80, 16.58, 0.70, 1.84, 2.84, 0.40,
  likely its spread in each stance and pace), how it fires (bursts) and reloads.
- Hearing: how far each kind of sound carries and what it starts (the sound stage).
- Moving: the path points, doors, and the ROE's speeds (Normal, Blitz).
- The two values set at run time: the close radius and the normal view angle.

## A first version

Small enough to try out on the maps we play, and grow from:

1. Terrorists from the mission files, where the mission puts them: the team's actor, kit, stance,
   alertness and one of its starting spots (the characters we place now stand at spawn points).
2. Seeing, as above: field of view, range, glimpses, visibility and line of sight.
3. Reacting to a player in view: turn toward them and fire, after a delay and with a spread to be
   read from the exe (and the skills); reload; take wounds and die as the characters do now.
4. Then hearing (gunfire, footsteps), the guard and patrol plans (GuardSpot, MoveTo loops),
   triggers, and moving to cover.
