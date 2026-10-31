//! The spread a gun's shots leave in, as the original's reticle has it (RogueSpear.exe
//! 0x5a9160, 0x5a97d0, 0x5a54e0; docs/terrorist-ai.md). How wide it can be comes from the
//! shooter's skill, the gun, how they move and their stance. Whenever how they move or their
//! stance changes it opens as wide, and then closes over a time the gun sets. Each shot opens it
//! further: with the trigger held it stays open between shots and opens by less each time, and
//! let go it closes again. Turning widens it too (0x59eb30 keeps how fast).

/// What a gun's file says of how it aims (after its ammunition's name, `data/kit/*.wpn`)
#[derive(Clone, Copy, Debug)]
pub struct Gun {
    /// How wide the spread is for each way of moving: stopped, shuffling (a slow walk, backing
    /// up, a side step), walking, walking fast and running
    pub moving: [f32; 5],
    /// Times that when crouched
    pub crouched: f32,
    /// How much the first shot opens the spread
    pub recoil: f32,
    /// Twice the time, in seconds, the spread takes to close
    pub close: f32,
}

/// The MP5K (pw_mp5k.wpn)
pub const MP5K: Gun = Gun { moving: [1.26, 1.75, 5.16, 7.59, 13.44], crouched: 0.70, recoil: 1.51, close: 0.76 };

/// A player's skill with a gun, out of 1: the original's operatives' Assault is mostly 80 to 100
pub const PLAYER_SKILL: f32 = 0.9;

/// The widest a spread can be, in radians (15 degrees)
const WIDEST: f32 = 0.2617994;

/// How a shooter moves, as the gun's spread goes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pace {
    #[default]
    Stopped,
    Shuffle,
    Walk,
    WalkFast,
    Run,
}

/// A shooter's spread
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Aim {
    /// The angle shots leave within, in radians
    pub angle: f32,
    /// What it closes from and to, and how long it has been closing
    pub from: f32,
    pub to: f32,
    pub since: f32,
    /// How much the next shot opens it, while the trigger is held
    pub growth: f32,
    /// How the shooter moved and stood when it last opened
    pub pace: Pace,
    pub crouched: bool,
    pub started: bool,
    /// Which way they faced, as an `Input`'s yaw, last time: how fast they turn widens it
    pub look: u16,
}

impl Aim {
    /// The spread `dt` seconds on, facing `look` (an `Input`'s yaw): opened afresh if how the
    /// shooter moves or stands has changed, held open while the trigger is, and otherwise
    /// closing. Turning widens it, by `1 + v^2` for `v` radians a second (a hundredth of that,
    /// tick by tick, with the trigger held). `wounded` makes it three times as wide.
    pub fn step(&mut self, gun: &Gun, skill: f32, pace: Pace, crouched: bool, wounded: bool, trigger: bool, look: u16, dt: f32) {
        let turned = if self.started { look.wrapping_sub(self.look) as i16 as f32 * std::f32::consts::TAU / 65536.0 } else { 0.0 };
        let turn = turned / dt.max(1e-6);
        self.look = look;
        let size = (6.0 - 5.0 * skill) * gun.moving[pace as usize] * if crouched { gun.crouched } else { 1.0 } * if wounded { 3.0 } else { 1.0 };
        if !self.started || pace != self.pace || crouched != self.crouched {
            *self = Aim { from: (size * 0.02).max(self.angle).min(WIDEST), to: size * 0.00083, since: 0.0, growth: gun.recoil, pace, crouched, started: true, ..*self };
            self.angle = self.from;
            return;
        }
        self.to = size * 0.00083;
        if trigger {
            self.angle = (self.angle * (1.0 + 0.01 * turn * turn)).min(WIDEST);
            self.from = self.angle;
            return;
        }
        self.growth = gun.recoil;
        let time = gun.close * 0.5;
        self.since = (self.since + dt).min(time);
        let left = 1.0 - self.since / time;
        self.angle = (((self.from - self.to) * left * left + self.to) * (1.0 + turn * turn)).min(WIDEST);
    }

    /// A shot opens the spread, by less with each while the trigger stays down
    pub fn fired(&mut self) {
        self.angle = (self.angle * self.growth).min(WIDEST);
        self.growth = (self.growth - 1.0) * 0.7 + 1.0;
        self.from = self.angle;
        self.since = 0.0;
    }

    /// How far off the aim a shot goes, in radians, and which way round it, from three numbers
    /// each between 0 and 1. Most go out toward the edge: on average at three quarters of it.
    pub fn throw(&self, u: [f32; 3]) -> (f32, f32) {
        ((1.0 - u[0] * u[1]) * self.angle, u[2] * std::f32::consts::TAU)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn settled(pace: Pace, crouched: bool) -> Aim {
        let mut a = Aim::default();
        for _ in 0..120 {
            a.step(&MP5K, PLAYER_SKILL, pace, crouched, false, false, 0, DT);
        }
        a
    }

    #[test]
    fn the_spread_closes_standing_still_and_is_widest_running() {
        let still = settled(Pace::Stopped, false);
        assert!((still.angle - 1.5 * 1.26 * 0.00083).abs() < 1e-5, "{}", still.angle);
        let crouched = settled(Pace::Stopped, true);
        assert!((crouched.angle - still.angle * 0.7).abs() < 1e-5);
        assert!(settled(Pace::Run, false).angle > 10.0 * still.angle);
        // Starting to move opens it at once, as wide as the new way of moving makes it
        let mut a = still;
        a.step(&MP5K, PLAYER_SKILL, Pace::Run, false, false, false, 0, DT);
        assert_eq!(a.angle, WIDEST);
    }

    #[test]
    fn shots_open_the_spread_by_less_each_time_until_the_trigger_is_let_go() {
        let mut a = settled(Pace::Stopped, false);
        let start = a.angle;
        let mut widths = vec![a.angle];
        for _ in 0..4 {
            a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, true, 0, DT);
            a.fired();
            widths.push(a.angle);
        }
        let growths: Vec<f32> = widths.windows(2).map(|w| w[1] / w[0]).collect();
        assert!((growths[0] - MP5K.recoil).abs() < 1e-4 && growths.windows(2).all(|g| g[1] < g[0]), "{growths:?}");
        // Held, it stays open; let go, it closes again in the gun's time
        a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, true, 0, DT);
        assert_eq!(a.angle, *widths.last().unwrap());
        for _ in 0..30 {
            a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, false, 0, DT);
        }
        assert!((a.angle - start).abs() < 1e-5, "{} against {start}", a.angle);
    }

    #[test]
    fn turning_widens_the_spread_by_one_and_the_square_of_how_fast() {
        let still = settled(Pace::Stopped, false);
        // About a radian a second, which is twice as wide
        let mut a = still;
        let step = (65536.0 / std::f32::consts::TAU * DT) as u16;
        a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, false, step, DT);
        let turn = step as f32 * std::f32::consts::TAU / 65536.0 / DT;
        assert!((a.angle / still.angle - (1.0 + turn * turn)).abs() < 1e-3, "{} against {}", a.angle / still.angle, 1.0 + turn * turn);
        // Turning back counts the same, and stopping lets it close again
        a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, false, 0, DT);
        assert!(a.angle > 1.9 * still.angle);
        a.step(&MP5K, PLAYER_SKILL, Pace::Stopped, false, false, false, 0, DT);
        assert!((a.angle - still.angle).abs() < 1e-6);
    }

    #[test]
    fn shots_go_out_mostly_toward_the_edge() {
        let a = Aim { angle: 1.0, ..Aim::default() };
        let mut sum = 0.0;
        let n = 20;
        for i in 0..n {
            for j in 0..n {
                sum += a.throw([(i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32, 0.0]).0;
            }
        }
        assert!((sum / (n * n) as f32 - 0.75).abs() < 0.01);
    }
}
