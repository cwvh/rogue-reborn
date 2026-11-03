# Multiplayer

What we want from network play, what the original does, what we have, and the plan.
Decisions still open are marked **Open**.

## Who plays

- Most games: 4 to 8 players on a LAN.
- Team games of 3v3, 4v4 and 5v5. These are what we tune for.
- Up to 16 players, as the original allowed. Larger games must work, if not as smoothly.
- Now and then, one or two players joining over the internet. The server runs at someone's
  home, so their snapshots go out over that home's upload, and they need a forwarded port.

So the usual game mixes connections: most players at about 1 ms, one or two at 30 to 150 ms.
That is the hardest case to keep fair.

There are no human playtests for now. Bots stand in for players until there are.

## What we have

- The server runs the game and the clients predict their own player (`net::server`,
  `net::client`). Snapshots go out 60 times a second, as the difference from one the client
  has, and others are shown 25 ms in the past (see "Tuning").
- Lag compensation: the server judges a shot against where the shooter saw the others, up to
  three quarters of a second back.
- A pretend network and clock (`net::link`): delay, jitter, loss and duplicates. Runs are
  deterministic.
- 17 tests of the server and clients together (`net::tests`).
- Bots (`net::bot`) that see only what a player would, and aim with human limits.
  `rrbot --fight` plays one over a real network.
- A lag meter (`net::meter`), and `--sim-latency`, `--sim-jitter` and `--sim-loss` on the
  client and bot.
- `rrsrv`, the dedicated server.
- Single-player recordings, with a checksum every second (`recording`).

The game is everyone for themselves, with a respawn after 3.7 s. There are no sides, rounds,
time limit or scores. Players start at the mission's spawn points. `MAX_PLAYERS` is 16.

### Fitting a snapshot in a datagram

A datagram holds 1200 bytes. A snapshot is sent as what changed since one the client has. With
16 players all on the move and 16 characters, that is about 800 bytes, with room for events.

- **Other players are sent less.** Only a client's own player carries what is on their screen:
  fall speed, ammo, grenades left, throw charge and messages. Another player is about 55 bytes
  in full, against 73. Their trigger is sent only while their gun fires. So no client is told
  an enemy's ammo.
- **A grenade is sent as where it is and how far it has turned**, 17 bytes. Only the server
  moves grenades and sets them off.
- **A snapshot too big for a datagram goes in parts**, up to 16 (`proto::split`, `Parts`). This
  is for a snapshot in full, when a client joins or falls too far behind, and for a crowd of
  grenades. A part lost loses the whole snapshot, and the server sends the next as usual.

## The test suite and the lab

The tests check that things work. They do not yet measure whether the game feels fair, which
is what people notice.

### The lab

A program that runs many in-process games over a grid of conditions and prints a table. The
main setup is 8 LAN players and 2 internet players, as bots on two teams. It measures:

- **Hit agreement:** when the shooter's screen shows the crosshair on the target, how often the
  server agrees.
- **Shots behind cover:** how far past a corner the victim was, on their own screen, when the
  shot landed. Lag compensation favors the shooter, so a remote player can hit a LAN player who
  has already stepped out of sight. Measured at 50, 100 and 150 ms.
- **Peeker's advantage:** how many milliseconds someone coming round a corner gets before the
  other player can see them.
- **Prediction fixes:** how often, and how far, the server moves a player from where they
  predicted.
- **Delay to see a result:** from the trigger to the hit showing, and from the shot to the victim
  seeing their own death.
- **Bandwidth:** bytes a second for each client, at 2, 4, 8, 10 and 16 players. This is what the
  host's upload must carry.

The lab is for tuning: the snapshot rate, the delay before others are shown, the rewind limit
and the input queue. It is deterministic, so two runs differ only in what we changed.

Its numbers are only as good as the bots. They need reaction times and aim like a person's,
and they need to know their side.

How it is done (`net::lab`, `rrlab`; the tests use the same harness):

- **Fights:** bots in the box room, everyone for themselves for now, some on the LAN and some
  over the internet. A connection's ping is its round trip; jitter is a tenth of it, and 1% of
  datagrams are lost.
- **Peeking:** the box room has no cover, so the lab draws a line across it and calls it the
  edge of a wall. One player holds still across the room and fires whenever their screen shows
  the other past the line. The other steps out past it and back, again and again. The timing is
  what matters, and it is the same as at a real corner.
- **Hit agreement:** the server keeps each player's shot as it judged it (`Sim::fired`): the
  eye, the direction, where it found the others, and which input fired it. The lab tries the
  same ray against where the shooter's screen showed the others as they sent that input. Both
  take bodies as plain cylinders, so only the netcode differs.
- **The gap:** for each hit, how far the victim was on their own screen from where the shot
  found them. Past the line, that is a hit behind cover.
- With lag compensation off, agreement falls to about 30%, which shows the measure sees
  disagreement when there is some.

First findings (60 seconds a row, before tuning, at 30 snapshots a second):

- The server agrees with the shooter's screen on 99 to 100% of shots, at every ping tried.
- Peeker's advantage is about 95 ms even on the LAN, about 160 ms against a peeker at 100 ms,
  and about 195 ms at 150 ms. Most of the LAN figure is the client showing others two snapshots
  in the past (`VIEW_DELAY`), which is where to look first for tuning.
- A peeker at 100 ms is hit behind the corner, on their own screen, on about a fifth of the
  holder's hits. Both at 100 ms, about a third.
- The gap is about 40 units on the LAN, 60 to 70 for a player at 100 to 150 ms.
- Being told of one's own shot takes about 43 ms on the LAN, 160 ms at 100 ms; of one's death,
  about 26 ms and 76 ms.
- The server moves a player from where they predicted almost never. (Counted over whole
  games, there seemed to be ten a minute; all were players dying, whose own copy ran on a tick
  before the server's word came. A death is no longer counted as a fix.)
- 16 players take about 92 KB a second of the host's upload, about 0.75 Mbit/s, and each client
  about 5 KB a second.

### Fairness tests

Tests that hold the lab's numbers once we know what is good. For example: "at 100 ms each way,
the server agrees with at least 95% of hits."

How it is done (`net::fairness`): each limit is what the lab measured, with room to spare, and
its comment gives the figure measured. When tuning makes a figure better, its limit is
tightened. They hold:

- agreement: 97% on the LAN, 95% at 100 ms;
- the gap: 35 units on the LAN, 65 at 100 ms;
- being told of one's shot and death: 45 and 25 ms on the LAN, 180 and 85 ms at 100 ms;
- fixes: half a fix a minute for each player, none over 50 units;
- 16 players: 230 KB a second of the host's upload, 19 KB a second for any client;
- peeker's advantage: 65 ms on the LAN, 135 and 170 ms for a peeker at 100 and 150 ms, 120 ms
  for a holder at 100 ms;
- hits behind cover: 20% for a peeker at 100 ms, 21% for a holder at 100 ms.

Other tests hold the rest: Wi-Fi (`players_on_wifi_keep_playing...`), drift, freezes, the size
of snapshots and recordings. Showing others twice as far in the past fails four of the five
(tried before tuning).

### Tuning

`rrlab --tune` tries settings against one another, and the lab can play with any of them
(`lab::Tuning`). How soon others are shown trades against how often they stand still for want
of a snapshot (`Client::starved`). Measured over 90 s a row:

| Snapshots a second, intervals behind | Behind | Peeker's advantage, LAN / 100 ms | Standing still, 100 ms / Wi-Fi |
| --- | --- | --- | --- |
| 30, 2 (before) | 67 ms | 97 / 161 ms | 0% / 5% |
| 30, 1.5 | 50 ms | 81 / 145 ms | 0% / 6% |
| 60, 2 | 33 ms | 65 / 129 ms | 0% / 5% |
| 60, 1.5 (chosen) | 25 ms | 49 / 113 ms | 1% / 6% |

On the LAN nothing stands still at any of these. Wi-Fi's bursts last longer than any short
delay can cover. For comparison, from memory: Quake 3 sent 20 snapshots a second and showed
others about one interval behind (50 ms), with no lag compensation; Quake Live sends 40, about
25 ms behind; Counter-Strike shows others two ticks behind, about 31 ms at 64 ticks.

The cost of 60 a second: each client downloads about twice as much, 15 KB a second for a
player far away; the host's upload for 16 players is about 184 KB a second, most of it to the
LAN; a player's recording is about 13 KB a second.

How far back shots are judged:

| Back | Wi-Fi shooter's agreement | 300 ms shooter's agreement | Wi-Fi players in a fight |
| --- | --- | --- | --- |
| 0.25 s | 98% | 35% | 94% |
| 0.5 s (before) | 99% | 100% | 95% |
| 0.75 s (chosen) | 99% | 100% | 98% |
| 1 s | 99% | 100% | 98% |

Below half a second fails players far away. Three quarters carries Wi-Fi through most stalls,
and costs the players on the LAN nothing the lab can see. A shooter at 300 ms hits a peeker on
the LAN behind the corner on about half of their hits at any limit that lets them play; that
is the price of looking back. The server now keeps 64 ticks of where players were (it kept
40).

After tuning, against before: peeker's advantage 46 ms on the LAN (92), 106 at 100 ms (151);
hits behind cover 14% for a peeker at 100 ms (20%); the gap 23 units on the LAN (40); told of
one's shot in 33 ms on the LAN (43); Wi-Fi shooters agree 94% in a fight (91%).

The lag meter now counts a snapshot that came late, behind a newer one, as come rather than
lost. At 60 a second, jitter puts many out of order.

Tried by hand on 2026-11-02, against a bot: in the box room on one computer, and on
citystreet_large with 50 ms each way added to the player's connection. Both felt fine: others
moved smoothly and hits landed where the crosshair was. The bot's walk skates, which is for the
animation work, not the network.

More realistic conditions for them and the lab:

- Different conditions for each player in one game.
- Loss in bursts, not spread evenly.
- Latency spikes, and a client that freezes for 2 s.
- Clocks that drift apart.

How it is done, and what it found:

- **Bursts and stalls** (`Conditions`): now and then a link loses everything for a while, or
  holds everything up and then lets it through in a rush. They start at random, each link on
  its own. `Conditions::wifi()` is a fair connection with a burst of a fifth of a second every
  five seconds or so, and a stall of a third of a second every fifteen.
- **A freeze** (`Peer::frozen`): the computer does nothing for a while. It found that the
  server held a frozen player's last input until it gave up on them at 5 s: they ran on and
  fired by themselves, and were moved 270 units when they came back. Now the server holds a
  missing input a quarter of a second, which bridges a burst, and then lets go: no moving or
  firing, the same look.
- **Drift** (`Peer::clock`): each computer's clock can run fast or slow. It found that a clock
  only 0.01% fast fills the server's queue of inputs in about eight minutes, and the player
  then waits 30 ms longer for everything they do, for the rest of the game. Now the server
  skips an input when a client has had two to spare all through the last second, keeping what
  that input pressed.
- **Held triggers**: a shot fired from a held input, while the client's own were lost, was
  judged against where the others are now. Now it is judged as the client would have shown
  them by then. Players on Wi-Fi went from 88% to 91% agreement.
- What is left for Wi-Fi: a stall longer than the server looks back (then half a second), after
  which the shooter fires at a picture too old to honor; and the shooter's own place
  mispredicted after a burst. Wi-Fi in the peek holds 99%.

### Recording network games

- **On the server:** every player's inputs each tick, and the joins and leaves. The server is
  deterministic, so this replays the whole match.
- **On the client:** the snapshots it got, with when they came, and its own inputs. This replays
  what one player saw, through the client code.
- A key to mark a moment, so it can be found later.

A remote tester's recording is how we will see what they saw. A recording of a bug becomes a
test.

How it is done (`net::record`):

- `rrsrv --record match.rrs` keeps how the match was set up, who joined and left between which
  ticks, each player's input every tick, and how far back each shot was judged. It is written
  out every 30 seconds and when the server stops. Four players come to about 1 KB a second.
- `rr --connect HOST:PORT --record seen.rrc` keeps every datagram the server sent, with when it
  came, every input, and the moment of every frame. M marks a moment. It comes to about 13 KB
  a second at 60 snapshots a second.
- Both keep a checksum of the game every second. `rrsrv --check FILE` plays either back and
  says whether it comes out the same, or the tick where it first does not.
- A player's recording plays back through the client code, so a replay shows the frames they
  were shown. It also keeps the checksum of their game as it was set up, which catches a client
  that set it up differently from `build_sim`.

Not done yet: a way to watch a recording, from any player's eyes.

### For playtests

- A hit debug view: where the server judged each shot, against where the shooter saw the target.
- The lag meter on screen as a small graph.
- A short checklist: what to try, and what to note when something feels wrong.

### Order

1. Raise `MAX_PLAYERS` to 16, the original's limit, and check that a snapshot of 16 busy
   players fits a datagram. Give each player their own connection conditions. Done.
2. Record network games, on the server and on the client. Done (`net::record`).
3. The lab. Done, with bots fighting everyone for themselves until they know their side.
4. Burst loss and latency spikes. Done, with freezes and drifting clocks.
5. Tests that hold the lab's numbers. Done (`net::fairness`).
6. Tuning with the lab. Done: 60 snapshots a second, others shown 1.5 intervals behind, shots
   judged up to 0.75 s back.

## The original's match flow

From the game types (`data/mission/*.mgt`), their victory conditions (`*VicCond*.txt`,
`TerroristHunt.txt`, `DisarmBomb.txt`), the multiplayer missions (`*_MUL.mis`, `*_MULT.mis`) and
the game's text (`data/text/interface/english/r6_string_table.txt`).

### Rounds

Every game type plays "until time expires or only one side is left alive." There is no
respawn. A dead player watches through a teammate's eyes ("Observing through ...").

1. **The session screen.** Each player picks a side (Blue, Gold or observer), a name and a kit,
   and says they are ready. The host can lock the teams and eject players.
2. **A countdown.** "Starting mission in 5 seconds," then "Away we go."
3. **The round**, with a time limit or none. A player who joins now waits ("Game in progress").
4. **The results.** Who won: Blue, Gold, a player, or "Everybody Lost." Each player's kills, hit
   percentage, rounds fired and rounds taken. Each player presses continue for the next game.

The host's options: max players, Blue and Gold team sizes, time limit, show enemies, auto
targeting, random maps, random teams, random insertions, kit restrictions and a password.

### Where players start

Each `_MUL` mission has an adversarial section:

- 17 to 42 spawn points, for the scattered types. We read these already.
- 1 to 5 base pairs, for the types that start in bases. Each pair has a name, then a Blue base
  and a Gold base. A base is a box between two corners, with a name of its own.

The `_MULT` missions add the terrorists for the hunts. A `.mgt` names the missions it may be
played on, or allows all of them.

### Game types

A `.mgt` gives the name and description, a victory-condition file or none, whether there are
sides, whether players start in bases or scattered, and whether hostages are made (from a
`.hst` actor and a kit).

| Type | Sides | Start | How a side wins | Status |
| --- | --- | --- | --- | --- |
| Survival | none | scattered | last one alive | planned |
| Team Survival | 2 | bases | last side alive | planned, first |
| Scattered Teams | 2 | scattered | last side alive | later |
| Terrorist Hunt | 2 | bases | kills more than half the terrorists, or the most | planned |
| Scatter Hunt | 2 | scattered | as Terrorist Hunt | later |
| Stronghold | 2 | bases | Blue gets one player into Gold's base | later |
| Double Stronghold | 2 | bases | gets one player into the other's base | later |
| Assassination | 2 | bases | kills the other side's leader, a hostage | later |
| Scatter Assassination | 2 | scattered | as Assassination | later |
| Double Bluff | 2 | bases | brings both hostages to its base; killing one loses | later |
| Save Your Base | 2 | bases | disarms the bomb wired to its base | later |

We play Survival, Team Survival and Terrorist Hunt first. The rest come later. Later does not
mean never: a choice that would get in the way of one of them is raised first, with the reason,
and decided then.

### The session screen

From a screenshot of a 3v3 Team Survival lobby (Black Thorn, with the later Voobly service).

- **Players**, top left: one row each, on a blue or gold bar by side. Each row has signal bars
  and a dot, green when that player is ready. The host has a computer icon in place of the dot.
  So the host plays too: the game ran on one player's machine, not a separate server. Under
  the list, BLUE and GOLD buttons pick a side. Clicking a row selects it. The host kicks a
  player by selecting them and pressing the kick button at the bottom, which others see dimmed.
  The row above the players is a header of icons, which cannot be selected; we leave it out.
- **Player options**, top right: name, specialty, then two columns for the primary and the
  secondary: the firearm, its ammo, and a kit item each (both Frag Grenade in the screenshot).
  Each is a dropdown, so a kit is put together from parts rather than picked whole.
- **The session**, middle: type (Adversarial), rules (Team Survival), map, insertions (Random),
  length (3:00), and each side's uniform ("Blue Medium", "Street2 Medium").
- **Session options**: show enemies, allow auto targeting, random maps, random teams, random
  insertions, use kit restrictions.
- **Chat**, lower middle: a log and a line to type in. Players picked sides there.
- **Ready**: a button at the bottom left, red until a player clicks it green.

"Insertions" names the base pair the sides start in. Its names can be long. Checking "random
insertions" picks a pair at random, and the field then reads "Random".

Within a base, players start at fixed places, by some simple rule. In a 2v2, a side's two
players start at nearly opposite ends of the box. A guess that fits: they are spread evenly
along the line between the box's two corners, so two players take the two corners. The exe
should say.

"Specialty" (Assault, Demolitions, Electronics and others) made a player quicker at their
specialty's tasks: placing door charges, or bugs. Players kept to Assault. We leave the choice
out, and everyone plays as Assault.

"Random teams" shuffled the players onto sides every round. Players could not pick a side
while it was on, and did not see their side until the round began.

"Use kit restrictions" let the host rule out items for everyone. It was used mostly to ban C4,
and even that fell away over time. We leave it out for now.

"Show enemies" put the enemy on the minimap. "Allow auto targeting" let players use the
original's aim assist, which was never used online. We have no minimap or aim assist, so we
leave both out.

Only the host can change the session: its fields and its options. Ready only shows who is
ready: the host can start the countdown whenever they like. The countdown is usually 5 seconds, and each second is announced
in the chat ("Starting mission in 5 seconds", then "Away we go"). "Start suspended" suggests the
host can stop it.

Ours keeps the server a separate process, `rrsrv`, even when a player hosts. Hosting from `rr`
starts `rrsrv` with a secret token, and `rr` joins it on localhost with that token, which makes
it the host in the lobby. So there is one way to run a server, the host is a client like any
other (about 1 ms away), and a slow frame on the host's screen does not hold up the game. The
same token can let a player run a dedicated server's lobby. `rr` passes the token through the
environment, not the command line, and ends `rrsrv` when it quits. `rrsrv` also shuts down if
the host's client is gone for too long, so a crash leaves nothing running.

When the host leaves, the game ends for everyone and they go back to the menu, as in the
original. `rrsrv` tells the clients why before it goes. A dedicated server with no host carries
on. This is a first version; see "Better than the original" below.

After the results, each player presses continue and goes back to the session screen, where
they can change side and kit before the next round.

The lights along the top are decoration, and most buttons along the bottom can wait. We need
not copy the look.

## Our match flow

The plan, to be settled.

There is no warm-up in the level. Players wait in the lobby, a screen of its own as in the
original. The server runs the session in phases: lobby, countdown, loading, live, results, then
the lobby again.

- **The lobby.** Players pick their side and kit and say they are ready. The host sets the
  session and starts the countdown whenever they like. Until the lobby screen exists, these come
  from flags on the server and client, with a line of text for the phase and countdown.
- **The countdown**, 5 seconds, announced in the chat each second. The session is locked from
  its start: the host can stop the countdown, but cannot change the session while it runs.
  Players may have picked their kit for the map and settings they saw, so a change slipped in
  at the last moment would be unfair to them. Players' sides are locked too. Players can still
  change their kit until the countdown ends.
- **Loading.** Each client builds the round's game and tells the server when it is ready. A
  player who is ready sees through their own eyes at their starting place, but cannot move,
  look around or change weapons. A player not ready within 30 seconds is dropped. Once the rest
  are ready, a 2-second wait, then everyone is let go and the round's clock starts.
- **A round** gives each player one life. No one comes back until the round is over.
- **A new round** is a new setup with a new seed, which every client builds its game from. We
  already build the game this way on joining, so doors, terrorists and bodies reset alike
  everywhere.
- **The end of a round.** A round ends when no more than one side has anyone alive, or the time
  runs out. A grenade still in the air or on the ground is waited for first: a dead player's
  last throw counts. If no one is left alive, nobody wins ("Everybody Lost"), as when the time
  runs out.
- **A round called off.** The end of a round is not checked until players are let go. If a side
  then has no one, because its players were dropped while loading, the round is called off:
  no result, no change to the scores, and everyone goes back to the lobby. The chat says why.
  The server keeps what happened: that the round was called off, why, and each side's players,
  those still there and those dropped. Then a win by abandonment can be judged later if need be.
- **The results**, then back to the lobby, where players can change side and kit.
- **The snapshot** carries the phase, the time left, the scores and each player's side. Round
  start and round end are events.
- **Friendly fire** is a server option, on by default. Competitive play kept it on, and it was
  rarely turned off.
- **A dead player** watches their own side's players, through their eyes or from behind them
  in the third person. There is no free camera, so the dead cannot scout for their side.
- **Chat** is one channel, with no side or team chat. In the lobby and the results, everyone
  sees everything. During a round, the living see only what the living say, and the dead see
  what both say. So the dead cannot call out where the enemy is. The original also had team
  and all chat in a round; most players now talk over voice, so we leave those out.
- **A late joiner** waits in the lobby until the next round.
- **A player who drops** mid-round counts as dead.
- **Kits.** Each player picks their own. How is not settled; see below.
- **Bots fill empty slots**, so uneven teams still play. They are also the lab's players.
- **Respawning** soon after dying stays as a server option, for tests and trying things out.

Team Survival first. Survival comes almost free once rounds work. Terrorist Hunt follows,
since the AI already runs the `_MULT` missions' terrorists.

Tests, in-process like the rest:

- A round ends when one side is dead.
- The time limit ends a round in a draw.
- The last of each side killed by one grenade is a draw.
- A grenade thrown by the last of a side, who is then shot, still kills the last of the other
  side: a draw, not a win.
- The host cannot change the session during the countdown, only stop it. Players cannot change
  side, but can change kit.
- No one moves or looks around, and the clock does not start, until everyone has loaded and 2
  more seconds have passed.
- A player who takes too long to load is dropped, and the round goes on without them.
- A side whose only player is dropped while loading calls the round off, and the record names
  the players on each side, present and dropped.
- A late joiner waits for the next round.
- A player who drops counts as dead.
- Everyone sees the same winner.
- A new round gives every client the same game.

### Open

- **Picking a kit.** In the lobby, and by a flag until then: a primary and a secondary, ammo for
  each, and two items. A mission can rule kits out (the `.mis`'s "Filtered Kits"); what that
  means in multiplayer is not read yet.
- **The lobby's design.** To be worked out from the original's session screen.

## Better than the original

We follow the original where it plays well, and do better where we can. Ideas to come back to:

- **One insertions field.** The original splits the choice between a field and a checkbox.
  Ours could be one list, with a choice like "A random pair each round" set apart from the
  named pairs, so no one takes it for a pair named Random.
- **Random teams with three settings** (decided): Off, players pick. Shuffle, the server puts
  players on sides and shows them. Random, the server puts players on sides and shows everyone
  in grey until the round begins, as the original does. Under Shuffle and Random, players cannot
  pick a side.
- **A look at the map from the lobby**, so players new to a level can learn it before the
  round: a picture, the original's plan map, or a camera that flies through it.
- **Back as a watcher.** A player dropped for loading too slowly, or who drops mid-round,
  could rejoin and watch the side they were on until the round ends. The server would know them
  by the token their client joined with, kept for the session. For now they are dropped, and
  rejoin in the lobby.
- **Send only what a client could see or hear.** A client would be told only of the players
  and characters it could see or hear. That saves the most, and makes a wallhack useless, since
  the client does not know where hidden enemies are. It touches sounds (footsteps, gunfire),
  the dead player's view of their side, and the bots, which see through the client.
- **Characters sent by their changes.** A character that changes at all is sent whole, 23
  bytes. A Terrorist Hunt has 20 to 30 terrorists, so many moving at once would send many
  snapshots in parts. Sending only what changed, as for players, or only the characters a
  client could see, would fix that.
- **A new host when the host leaves.** The server is its own process, so it can go on without
  the host's client and hand the host's rights to another player. It still runs on the old
  host's machine and upload, so their leaving the game is not the same as their wanting it
  closed. Choices: keep running until the round ends, or until the machine's owner stops it,
  or ask the host when they leave.
