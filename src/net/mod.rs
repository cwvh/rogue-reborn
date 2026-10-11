//! Playing over a network: a server that owns the game, and clients that send it what their
//! players do and draw what it says happened.

pub mod animation;
pub mod client;
pub mod link;
pub mod proto;
pub mod server;
pub mod setup;

#[cfg(test)]
mod tests;
