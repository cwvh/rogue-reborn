//! `rr --host`'s server, run for real: rrsrv as a child on the built-in box room, which needs
//! none of the game's data

use std::net::{ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant};

use rogue_reborn::net::client::{Client, Status};
use rogue_reborn::net::host::{HostedServer, new_key};
use rogue_reborn::net::link::UdpLink;

fn rrsrv() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_rrsrv"))
}

fn args(port: u16, more: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = ["--demo", "--no-models", "--teams", "--rounds", "--quiet", "--port", &port.to_string()].iter().map(|s| s.to_string()).collect();
    a.extend(more.iter().map(|s| s.to_string()));
    a
}

/// Whether nothing holds the port any more
fn free(port: u16) -> bool {
    UdpSocket::bind(("0.0.0.0", port)).is_ok()
}

/// Run until `done`, for at most `seconds`
fn wait_for(seconds: f64, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

/// A player who joins with `key`
fn join(port: u16, key: u64) -> (Client, UdpLink) {
    let mut link = UdpLink::bind("0.0.0.0:0").unwrap();
    let to = link.addr_of(("127.0.0.1", port).to_socket_addrs().unwrap().next().unwrap());
    let mut client = Client::new(to, 77, option_env!("GIT_HASH").unwrap_or("unknown"));
    client.set_key(key);
    let start = Instant::now();
    while client.connect(start.elapsed().as_secs_f64(), &mut link).is_none() {
        assert!(start.elapsed().as_secs_f64() < 10.0, "no answer: {:?}", client.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    (client, link)
}

#[test]
fn a_hosted_server_starts_and_stopping_it_leaves_nothing_running() {
    let port = 27991;
    let server = HostedServer::start(rrsrv(), &args(port, &[]), new_key(), Duration::from_secs(30)).unwrap();
    assert!(!free(port));
    server.stop();
    assert!(wait_for(2.0, || free(port)));
}

#[test]
fn a_port_in_use_is_reported_and_not_joined() {
    let port = 27992;
    let _taken = UdpSocket::bind(("0.0.0.0", port)).unwrap();
    let Err(e) = HostedServer::start(rrsrv(), &args(port, &[]), new_key(), Duration::from_secs(30)) else { panic!("it started on a port in use") };
    assert!(format!("{e:#}").contains("cannot listen"), "{e:#}");
}

#[test]
fn a_hosted_server_ends_when_no_host_comes() {
    let port = 27993;
    let mut server = HostedServer::start(rrsrv(), &args(port, &["--host-wait", "1"]), new_key(), Duration::from_secs(30)).unwrap();
    assert!(wait_for(5.0, || !server.running()), "it is still running with no host");
}

#[test]
fn a_hosted_server_ends_when_its_host_leaves() {
    let port = 27994;
    let key = new_key();
    let mut server = HostedServer::start(rrsrv(), &args(port, &[]), key, Duration::from_secs(30)).unwrap();
    let (mut client, mut link) = join(port, key);
    assert_eq!(client.status, Status::Connected);
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.running());
    client.leave(&mut link);
    assert!(wait_for(5.0, || !server.running()), "it is still running without its host");
}
