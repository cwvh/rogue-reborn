//! reader-bench MAP...: times reading each map with the Rust reader, printing one line per
//! measurement as `experimental/map/mapbench` does for the C one:
//!   rust STEP MAP BYTES RUNS MIN_NS MEDIAN_NS
//! where STEP is "read" (map::read from the path, not counting dropping the map), "drop"
//! (dropping it) or "file" (only reading the file).

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use rogue_reborn::map;

/// Runs `once` twice to warm up, then until it has taken 300 ms and run 20 times
fn bench(name: &str, path: &str, bytes: usize, mut once: impl FnMut() -> Duration) {
    once();
    once();
    let mut times = Vec::new();
    let mut total = Duration::ZERO;
    while total < Duration::from_millis(300) || times.len() < 20 {
        let t = once();
        total += t;
        times.push(t);
    }
    times.sort();
    println!("rust {name} {path} {bytes} {} {} {}", times.len(), times[0].as_nanos(), times[times.len() / 2].as_nanos());
}

fn main() {
    for path in std::env::args().skip(1) {
        let p = Path::new(&path);
        let bytes = std::fs::read(p).expect("the map").len();
        bench("read", &path, bytes, || {
            let t0 = Instant::now();
            let m = map::read(p).expect("the map reads");
            let t = t0.elapsed();
            drop(black_box(m));
            t
        });
        bench("drop", &path, bytes, || {
            let m = map::read(p).expect("the map reads");
            let t0 = Instant::now();
            drop(black_box(m));
            t0.elapsed()
        });
        bench("file", &path, bytes, || {
            let t0 = Instant::now();
            let b = black_box(std::fs::read(p).expect("the map"));
            let t = t0.elapsed();
            drop(b);
            t
        });
    }
}
