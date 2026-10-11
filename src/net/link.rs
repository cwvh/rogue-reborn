//! Moving datagrams: the `Link` the server and client talk through, a real UDP link, a
//! pretend network for tests, and a wrapper that makes any link behave like a bad connection.
//!
//! Everything here is driven by the time the caller gives it, never a clock of its own, so a
//! test can run minutes of a game in a moment and get the same result every time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

/// Who a datagram is to or from. What it means is up to the link: it is the same for as long as
/// the same peer is being talked to.
pub type Addr = u64;

/// The most bytes a datagram may hold, so that it crosses the internet in one piece
pub const MAX_PACKET: usize = 1200;

/// A way to send datagrams to peers and get the ones sent to us. Datagrams may be lost,
/// repeated or arrive out of order; the protocol on top deals with that.
pub trait Link {
    fn send(&mut self, to: Addr, bytes: &[u8]);
    fn recv(&mut self) -> Option<(Addr, Vec<u8>)>;
}

/// How a connection misbehaves
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    /// Seconds a datagram takes, one way
    pub latency: f64,
    /// Up to this many seconds more, at random, which can put datagrams out of order
    pub jitter: f64,
    /// The chance of a datagram being lost, 0 to 1
    pub loss: f32,
    /// The chance of one arriving twice
    pub duplicate: f32,
}

impl Conditions {
    /// A perfect connection
    pub const NONE: Conditions = Conditions { latency: 0.0, jitter: 0.0, loss: 0.0, duplicate: 0.0 };

    /// Typical of a decent home connection to a nearby server
    pub fn broadband() -> Conditions {
        Conditions { latency: 0.03, jitter: 0.01, loss: 0.01, duplicate: 0.0 }
    }

    /// Typical of a poor connection
    pub fn poor() -> Conditions {
        Conditions { latency: 0.12, jitter: 0.06, loss: 0.08, duplicate: 0.01 }
    }
}

/// A datagram held back until its time comes
struct Held {
    at: f64,
    peer: Addr,
    bytes: Vec<u8>,
}

/// Decides what happens to each datagram: whether it is lost, repeated, and when it arrives
struct Impairment {
    conditions: Conditions,
    rng: u64,
    held: Vec<Held>,
}

impl Impairment {
    fn new(conditions: Conditions, seed: u64) -> Impairment {
        Impairment { conditions, rng: seed | 1, held: Vec::new() }
    }

    /// A random number from 0 up to 1
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Put a datagram in the pipe at time `now`
    fn put(&mut self, now: f64, peer: Addr, bytes: &[u8]) {
        let c = self.conditions;
        if self.random() < c.loss as f64 {
            return;
        }
        let copies = if self.random() < c.duplicate as f64 { 2 } else { 1 };
        for _ in 0..copies {
            let at = now + c.latency + c.jitter * self.random();
            self.held.push(Held { at, peer, bytes: bytes.to_vec() });
        }
    }

    /// The datagram that arrives first by time `now`, if one has. `only` picks whose, if given.
    fn take(&mut self, now: f64, only: Option<Addr>) -> Option<(Addr, Vec<u8>)> {
        let i = self.held.iter().enumerate()
            .filter(|(_, h)| h.at <= now && only.is_none_or(|a| h.peer == a))
            .min_by(|a, b| a.1.at.total_cmp(&b.1.at))
            .map(|(i, _)| i)?;
        let h = self.held.remove(i);
        Some((h.peer, h.bytes))
    }
}

/// A pretend network for tests: any number of endpoints, with datagrams held for a while,
/// lost or repeated according to the conditions, all on a clock the test sets.
#[derive(Clone)]
pub struct FakeNet {
    inner: Rc<RefCell<Net>>,
}

struct Net {
    now: f64,
    /// One pipe for each endpoint, holding what is on its way to it
    pipes: Vec<Impairment>,
    seed: u64,
    conditions: Conditions,
    /// Conditions for datagrams from one endpoint to another, in place of the usual ones
    between: HashMap<(Addr, Addr), Conditions>,
}

impl FakeNet {
    pub fn new(seed: u64, conditions: Conditions) -> FakeNet {
        FakeNet { inner: Rc::new(RefCell::new(Net { now: 0.0, pipes: Vec::new(), seed, conditions, between: HashMap::new() })) }
    }

    /// Join the network
    pub fn endpoint(&self) -> FakeLink {
        let mut net = self.inner.borrow_mut();
        let addr = net.pipes.len() as Addr + 1;
        let seed = net.seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(addr);
        let conditions = net.conditions;
        net.pipes.push(Impairment::new(conditions, seed));
        FakeLink { net: self.clone(), addr }
    }

    /// Set the time. Datagrams arrive once it reaches theirs.
    pub fn set_time(&self, now: f64) {
        self.inner.borrow_mut().now = now;
    }

    pub fn time(&self) -> f64 {
        self.inner.borrow().now
    }

    /// Change how every connection behaves from now on
    pub fn set_conditions(&self, conditions: Conditions) {
        let mut net = self.inner.borrow_mut();
        net.conditions = conditions;
        for p in net.pipes.iter_mut() {
            p.conditions = conditions;
        }
    }

    /// Change how datagrams from one endpoint to another behave, which is not the same as the
    /// way back
    pub fn set_conditions_between(&self, from: Addr, to: Addr, conditions: Conditions) {
        self.inner.borrow_mut().between.insert((from, to), conditions);
    }

    /// Datagrams still on their way
    pub fn in_flight(&self) -> usize {
        self.inner.borrow().pipes.iter().map(|p| p.held.len()).sum()
    }
}

/// An endpoint of a `FakeNet`
pub struct FakeLink {
    net: FakeNet,
    addr: Addr,
}

impl FakeLink {
    /// This endpoint's address, which the others send to
    pub fn addr(&self) -> Addr {
        self.addr
    }
}

impl Link for FakeLink {
    fn send(&mut self, to: Addr, bytes: &[u8]) {
        assert!(bytes.len() <= MAX_PACKET, "a datagram of {} bytes is too big", bytes.len());
        let mut net = self.net.inner.borrow_mut();
        let now = net.now;
        let Some(pipe) = net.pipes.get_mut(to as usize - 1) else { return };
        let saved = pipe.conditions;
        if let Some(c) = net.between.get(&(self.addr, to)).copied() {
            net.pipes[to as usize - 1].conditions = c;
        }
        net.pipes[to as usize - 1].put(now, self.addr, bytes);
        net.pipes[to as usize - 1].conditions = saved;
    }

    fn recv(&mut self) -> Option<(Addr, Vec<u8>)> {
        let mut net = self.net.inner.borrow_mut();
        let now = net.now;
        net.pipes[self.addr as usize - 1].take(now, None)
    }
}

/// A real UDP socket
pub struct UdpLink {
    socket: UdpSocket,
    peers: Vec<SocketAddr>,
}

impl UdpLink {
    /// Listen on an address such as "0.0.0.0:27960", or "127.0.0.1:0" for any free port
    pub fn bind(addr: &str) -> io::Result<UdpLink> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(UdpLink { socket, peers: Vec::new() })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The `Addr` that stands for a socket address
    pub fn addr_of(&mut self, peer: SocketAddr) -> Addr {
        match self.peers.iter().position(|&p| p == peer) {
            Some(i) => i as Addr + 1,
            None => {
                self.peers.push(peer);
                self.peers.len() as Addr
            }
        }
    }

    /// The socket address an `Addr` stands for
    pub fn peer(&self, addr: Addr) -> Option<SocketAddr> {
        self.peers.get((addr as usize).checked_sub(1)?).copied()
    }
}

impl Link for UdpLink {
    fn send(&mut self, to: Addr, bytes: &[u8]) {
        if let Some(peer) = self.peer(to) {
            // A datagram that cannot be sent is a datagram lost, which is nothing new
            let _ = self.socket.send_to(bytes, peer);
        }
    }

    fn recv(&mut self) -> Option<(Addr, Vec<u8>)> {
        let mut buf = [0u8; 2048];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => return Some((self.addr_of(from), buf[..n].to_vec())),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                // Some systems report an earlier datagram's failure on the next receive; go on
                Err(_) => continue,
            }
        }
    }
}

/// Makes the link inside it behave like a bad connection, in both directions. The owner says
/// what time it is with `set_time` before using it.
pub struct Impaired<L: Link> {
    inner: L,
    now: f64,
    outgoing: Impairment,
    incoming: Impairment,
}

impl<L: Link> Impaired<L> {
    pub fn new(inner: L, conditions: Conditions, seed: u64) -> Impaired<L> {
        Impaired { inner, now: 0.0, outgoing: Impairment::new(conditions, seed), incoming: Impairment::new(conditions, seed ^ 0x5DEECE66D) }
    }

    pub fn set_time(&mut self, now: f64) {
        self.now = now;
    }

    pub fn inner(&self) -> &L {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut L {
        &mut self.inner
    }

    /// Send what has been held long enough, and take in what has arrived
    fn pump(&mut self) {
        while let Some((to, bytes)) = self.outgoing.take(self.now, None) {
            self.inner.send(to, &bytes);
        }
        while let Some((from, bytes)) = self.inner.recv() {
            self.incoming.put(self.now, from, &bytes);
        }
    }
}

impl<L: Link> Link for Impaired<L> {
    fn send(&mut self, to: Addr, bytes: &[u8]) {
        self.outgoing.put(self.now, to, bytes);
        self.pump();
    }

    fn recv(&mut self) -> Option<(Addr, Vec<u8>)> {
        self.pump();
        self.incoming.take(self.now, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(conditions: Conditions) -> (FakeNet, FakeLink, FakeLink) {
        let net = FakeNet::new(1, conditions);
        let (a, b) = (net.endpoint(), net.endpoint());
        (net, a, b)
    }

    #[test]
    fn a_datagram_arrives_after_its_latency_and_not_before() {
        let (net, mut a, mut b) = pair(Conditions { latency: 0.05, ..Conditions::NONE });
        a.send(b.addr(), b"hello");
        assert_eq!(b.recv(), None);
        net.set_time(0.049);
        assert_eq!(b.recv(), None);
        net.set_time(0.051);
        assert_eq!(b.recv(), Some((a.addr(), b"hello".to_vec())));
        assert_eq!(b.recv(), None);
    }

    #[test]
    fn about_the_right_share_is_lost_and_the_same_ones_every_time() {
        let run = || {
            let (net, mut a, mut b) = pair(Conditions { loss: 0.25, ..Conditions::NONE });
            for i in 0..2000u32 {
                a.send(b.addr(), &i.to_le_bytes());
            }
            net.set_time(1.0);
            std::iter::from_fn(|| b.recv()).map(|(_, p)| u32::from_le_bytes(p.try_into().unwrap())).collect::<Vec<_>>()
        };
        let got = run();
        assert!((1350..1650).contains(&got.len()), "{} of 2000 arrived", got.len());
        assert_eq!(got, run());
    }

    #[test]
    fn jitter_puts_datagrams_out_of_order_and_duplicates_repeat_them() {
        let (net, mut a, mut b) = pair(Conditions { latency: 0.02, jitter: 0.05, loss: 0.0, duplicate: 0.5 });
        for i in 0..200u32 {
            net.set_time(i as f64 * 0.001);
            a.send(b.addr(), &i.to_le_bytes());
        }
        net.set_time(1.0);
        let got: Vec<u32> = std::iter::from_fn(|| b.recv()).map(|(_, p)| u32::from_le_bytes(p.try_into().unwrap())).collect();
        assert!(got.len() > 250, "{} arrived", got.len());
        assert!(got.windows(2).any(|w| w[1] < w[0]), "nothing arrived out of order");
        for i in 0..200 {
            assert!(got.contains(&i));
        }
    }

    #[test]
    fn each_direction_can_behave_differently() {
        let (net, mut a, mut b) = pair(Conditions::NONE);
        net.set_conditions_between(a.addr(), b.addr(), Conditions { loss: 1.0, ..Conditions::NONE });
        a.send(b.addr(), b"lost");
        b.send(a.addr(), b"kept");
        assert_eq!(b.recv(), None);
        assert_eq!(a.recv(), Some((b.addr(), b"kept".to_vec())));
    }

    #[test]
    fn real_udp_sockets_exchange_datagrams() {
        let mut server = UdpLink::bind("127.0.0.1:0").unwrap();
        let mut client = UdpLink::bind("127.0.0.1:0").unwrap();
        let to_server = client.addr_of(server.local_addr().unwrap());
        client.send(to_server, b"ping");
        let mut got = None;
        for _ in 0..200 {
            got = server.recv();
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (from, bytes) = got.expect("the datagram never arrived");
        assert_eq!(bytes, b"ping");
        server.send(from, b"pong");
        let mut reply = None;
        for _ in 0..200 {
            reply = client.recv();
            if reply.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(reply, Some((to_server, b"pong".to_vec())));
    }

    #[test]
    fn a_wrapped_link_holds_back_and_loses_what_it_sends() {
        let net = FakeNet::new(3, Conditions::NONE);
        let (mut plain, inner) = (net.endpoint(), net.endpoint());
        let mut bad = Impaired::new(inner, Conditions { latency: 0.1, loss: 0.5, ..Conditions::NONE }, 9);
        let to = plain.addr();
        for i in 0..1000u32 {
            bad.send(to, &i.to_le_bytes());
        }
        // Nothing has left yet: it is all being held for a tenth of a second
        assert_eq!(plain.recv(), None);
        bad.set_time(0.2);
        net.set_time(0.2);
        bad.recv();
        let arrived = std::iter::from_fn(|| plain.recv()).count();
        assert!((400..600).contains(&arrived), "{arrived} of 1000 arrived");
    }

    #[test]
    fn a_wrapped_link_holds_back_what_it_receives() {
        let net = FakeNet::new(3, Conditions::NONE);
        let (mut plain, inner) = (net.endpoint(), net.endpoint());
        let mut slow = Impaired::new(inner, Conditions { latency: 0.1, ..Conditions::NONE }, 9);
        plain.send(slow.inner().addr(), b"back");
        assert_eq!(slow.recv(), None);
        slow.set_time(0.05);
        assert_eq!(slow.recv(), None);
        slow.set_time(0.11);
        assert_eq!(slow.recv(), Some((plain.addr(), b"back".to_vec())));
    }
}
