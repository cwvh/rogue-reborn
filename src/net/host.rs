//! A server started for its host: `rr --host` runs rrsrv as a child, with a key that makes the
//! player who joins with it the host (`RR_HOST_KEY`), and ends it on quitting. rrsrv ends the
//! game by itself if the host leaves, goes quiet or never comes (`--hosted`), so a host whose
//! game crashes leaves nothing running for long.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// How long the server has to load its level and listen
pub const STARTUP: Duration = Duration::from_secs(30);

/// How many of the server's last lines are kept, to show if it fails
const LAST_LINES: usize = 12;

/// A secret for the host: `std`'s hasher keys come from the OS's random source. Never 0, which
/// is no key.
pub fn new_key() -> u64 {
    loop {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        let key = h.finish();
        if key != 0 {
            return key;
        }
    }
}

/// rrsrv, running for this player
pub struct HostedServer {
    child: Child,
    /// The server's last lines, in case it fails
    last: Arc<Mutex<VecDeque<String>>>,
}

impl HostedServer {
    /// Start `exe` with `args` and the host `key`, and wait up to `wait` for it to listen. Its
    /// output goes on to ours, each line marked as the server's.
    pub fn start(exe: &Path, args: &[String], key: u64, wait: Duration) -> Result<HostedServer> {
        let mut command = Command::new(exe);
        command.args(args).arg("--hosted").env("RR_HOST_KEY", key.to_string()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: no console of its own
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().with_context(|| format!("could not start {}", exe.display()))?;
        let last = Arc::new(Mutex::new(VecDeque::new()));
        let (listening, heard) = mpsc::channel();
        let out: Box<dyn Read + Send> = Box::new(child.stdout.take().expect("piped"));
        let err: Box<dyn Read + Send> = Box::new(child.stderr.take().expect("piped"));
        // Both are read to the end, so that neither pipe fills and stops the server
        for pipe in [out, err] {
            let (last, listening) = (last.clone(), listening.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    println!("server: {line}");
                    if line.contains("listening on port") {
                        let _ = listening.send(());
                    }
                    let mut last = last.lock().unwrap();
                    last.push_back(line);
                    if last.len() > LAST_LINES {
                        last.pop_front();
                    }
                }
            });
        }
        let mut server = HostedServer { child, last };
        let start = Instant::now();
        loop {
            if heard.recv_timeout(Duration::from_millis(50)).is_ok() {
                return Ok(server);
            }
            if let Some(status) = server.child.try_wait()? {
                // What it said before it stopped
                std::thread::sleep(Duration::from_millis(100));
                bail!("the server stopped ({status}): {}", server.last_lines());
            }
            if start.elapsed() > wait {
                bail!("the server did not start within {} seconds: {}", wait.as_secs(), server.last_lines());
            }
        }
    }

    fn last_lines(&self) -> String {
        self.last.lock().unwrap().iter().cloned().collect::<Vec<_>>().join(" / ")
    }

    /// Whether it is still running
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// End it, and wait for it to be gone, as dropping it does
    pub fn stop(self) {}
}

impl Drop for HostedServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_never_zero_and_never_the_same() {
        let keys: Vec<u64> = (0..100).map(|_| new_key()).collect();
        assert!(keys.iter().all(|&k| k != 0));
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
    }

    #[test]
    fn a_server_that_is_not_there_is_reported() {
        let Err(e) = HostedServer::start(Path::new("no/such/rrsrv"), &[], 1, Duration::from_secs(1)) else { panic!("it started") };
        assert!(format!("{e:#}").contains("could not start"), "{e:#}");
    }
}
