//! The lag meter: what a client can tell about its connection from what it sends and what comes
//! back, over the last few seconds.
//!
//! - Ping is the time from sending an input until a snapshot says the server has used it. That
//!   includes the wait in the server's input queue and up to a snapshot's interval before the
//!   next one goes out, so it is the lag the player feels, a little more than the network's.
//! - Snapshots are numbered by server tick, so one that never came shows as a gap.
//! - Jitter is the spread of the snapshots' arrival times, each against when it was due.
//! - A fix is the server moving the player from where the client predicted them.

use std::collections::VecDeque;

use crate::sim::TICK;

/// How far back the meter looks (seconds)
pub const WINDOW: f64 = 4.0;

/// The most inputs remembered to time a ping by: more than the server ever holds back
const SENT: usize = 300;

/// A fix smaller than this (units) is rounding, not a misprediction
pub const FIX: f32 = 1.0;

/// A snapshot, or one that never came
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    /// When it arrived (or the next one did, for a lost one)
    pub at: f64,
    pub tick: u32,
    /// Seconds from sending the input it confirms to its arrival
    pub ping: Option<f32>,
    pub lost: bool,
    /// How far (units) it moved the player from where they were predicted to be
    pub fix: f32,
}

/// One frame's view of the other players: how much is in hand beyond what is shown, and how far
/// behind the server's present it is (seconds)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub at: f64,
    pub ahead: f32,
    pub behind: f32,
}

/// The meter's figures over the window
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Figures {
    /// Seconds, the mean
    pub ping: f32,
    /// Seconds, the standard deviation of arrival times against when they were due
    pub jitter: f32,
    /// Lost snapshots, 0 to 1
    pub loss: f32,
    /// How far in the past other players are shown (seconds): the view's delay plus half the ping
    pub view: f32,
    pub fixes: u32,
    pub worst_fix: f32,
    /// Bytes a second
    pub bytes_in: f32,
    pub bytes_out: f32,
}

#[derive(Default)]
pub struct Meter {
    /// Inputs sent: their number and when
    sent: VecDeque<(u32, f64)>,
    pub columns: VecDeque<Column>,
    pub shown: VecDeque<Shown>,
    traffic_in: VecDeque<(f64, usize)>,
    traffic_out: VecDeque<(f64, usize)>,
    /// The newest snapshot's tick, and the smallest step between two newest ones: the server's
    /// snapshot interval in ticks
    newest: Option<u32>,
    interval: Option<u32>,
}

impl Meter {
    /// An input numbered `seq` went out at `now`, in a datagram of `bytes`
    pub fn sent(&mut self, now: f64, seq: u32, bytes: usize) {
        self.sent.push_back((seq, now));
        while self.sent.len() > SENT {
            self.sent.pop_front();
        }
        self.traffic_out.push_back((now, bytes));
        self.forget(now);
    }

    /// A datagram of `bytes` arrived
    pub fn received(&mut self, now: f64, bytes: usize) {
        self.traffic_in.push_back((now, bytes));
    }

    /// A snapshot for server tick `tick` arrived, saying the server has used inputs up to `ack_seq`
    pub fn snapshot(&mut self, now: f64, tick: u32, ack_seq: u32) {
        if self.newest.is_some_and(|n| tick <= n) {
            // Late, behind a newer one: it was counted as lost
            return;
        }
        if let Some(n) = self.newest {
            let step = tick - n;
            let interval = self.interval.map_or(step, |i| i.min(step));
            self.interval = Some(interval);
            // The ones that never came, spread over the time between the two that did
            let last = self.columns.back().map_or(now, |c| c.at);
            let missed = step / interval;
            for m in 1..missed {
                let at = last + (now - last) * m as f64 / missed as f64;
                self.columns.push_back(Column { at, tick: n + m * interval, ping: None, lost: true, fix: 0.0 });
            }
        }
        self.newest = Some(tick);
        let ping = self.sent.iter().find(|s| s.0 == ack_seq).map(|s| (now - s.1) as f32);
        self.columns.push_back(Column { at: now, tick, ping, lost: false, fix: 0.0 });
        self.forget(now);
    }

    /// The newest snapshot moved the player `by` units from where they were predicted to be
    pub fn fixed(&mut self, by: f32) {
        if let Some(c) = self.columns.back_mut() {
            c.fix = by;
        }
    }

    /// A frame showed the other players `ahead` ticks behind the newest snapshot, and `behind`
    /// ticks behind where the server is thought to be now
    pub fn frame(&mut self, now: f64, ahead: f64, behind: f64) {
        self.shown.push_back(Shown { at: now, ahead: (ahead * TICK as f64) as f32, behind: (behind * TICK as f64) as f32 });
        self.forget(now);
    }

    fn forget(&mut self, now: f64) {
        let old = now - WINDOW;
        while self.columns.front().is_some_and(|c| c.at < old) {
            self.columns.pop_front();
        }
        while self.shown.front().is_some_and(|s| s.at < old) {
            self.shown.pop_front();
        }
        for traffic in [&mut self.traffic_in, &mut self.traffic_out] {
            while traffic.front().is_some_and(|t| t.0 < old) {
                traffic.pop_front();
            }
        }
    }

    /// The server's snapshot interval (seconds), once two snapshots have shown it
    pub fn interval(&self) -> Option<f32> {
        self.interval.map(|i| i as f32 * TICK)
    }

    pub fn figures(&self) -> Figures {
        let pings: Vec<f32> = self.columns.iter().filter_map(|c| c.ping).collect();
        let ping = mean(&pings);
        // Each arrival against when it was due, taking the server's clock as a steady one
        let offsets: Vec<f32> = self.columns.iter().filter(|c| !c.lost).map(|c| (c.at - c.tick as f64 * TICK as f64) as f32).collect();
        let center = mean(&offsets);
        let jitter = mean(&offsets.iter().map(|o| (o - center).powi(2)).collect::<Vec<_>>()).sqrt();
        let lost = self.columns.iter().filter(|c| c.lost).count();
        let loss = if self.columns.is_empty() { 0.0 } else { lost as f32 / self.columns.len() as f32 };
        let behind = mean(&self.shown.iter().map(|s| s.behind).collect::<Vec<_>>());
        let fixes: Vec<f32> = self.columns.iter().map(|c| c.fix).filter(|&f| f >= FIX).collect();
        let rate = |traffic: &VecDeque<(f64, usize)>| traffic.iter().map(|t| t.1).sum::<usize>() as f32 / WINDOW as f32;
        Figures {
            ping, jitter, loss,
            view: behind + ping / 2.0,
            fixes: fixes.len() as u32,
            worst_fix: fixes.iter().copied().fold(0.0, f32::max),
            bytes_in: rate(&self.traffic_in),
            bytes_out: rate(&self.traffic_out),
        }
    }
}

fn mean(xs: &[f32]) -> f32 {
    if xs.is_empty() { 0.0 } else { xs.iter().sum::<f32>() / xs.len() as f32 }
}
