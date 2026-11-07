//! Playing over a network: a server that owns the game, and clients that send it what their
//! players do and draw what it says happened.

pub mod animation;
pub mod bot;
pub mod client;
pub mod host;
pub mod lab;
pub mod link;
pub mod meter;
pub mod proto;
pub mod record;
pub mod round;
pub mod server;
pub mod setup;
pub mod watch;

#[cfg(test)]
mod fairness;
#[cfg(test)]
mod lobby_tests;
#[cfg(test)]
mod results_tests;
#[cfg(test)]
mod round_tests;
#[cfg(test)]
mod tests;
