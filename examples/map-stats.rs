use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::time::{Instant, Duration};

use anyhow::Result;
use rogue_reborn::map;
use get_size::GetSize;

fn main() -> Result<()> {
    // TODO: take CLI path to RSB file(s)

    let mut total = 0;
    let mut total_bytes = 0;
    let mut elapsed = Duration::ZERO;

    let opts = glob::MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    for path in glob::glob_with("data/**/city*.map", opts)? {
        let filename = path?;

        let mut file = File::open(&filename)?;
        let bytes = file.seek(SeekFrom::End(0))?;
        total_bytes += bytes;
        if filename.to_str().unwrap_or("").ends_with("obstacketest.map") {
            println!("skipping '{}' because it is not parseable",
                filename.display());
            continue;
        }
        print!("{} {} ", filename.display(), pretty(bytes));

        let now = Instant::now();
        let Ok(map) = map::read(&filename) else {
            println!("FAILED");
            continue;
        };

        let stack = map::Map::get_stack_size() as _;
        let heap = map.get_heap_size() as _;
        let mem_total = map.get_size() as _;
        println!("created {}", created(map.header.timestamp));
        println!("on-disk size         : {}", pretty(bytes));
        println!("in-memory stack size : {}", pretty(stack));
        println!("in-memory heap size  : {}", pretty(heap));
        println!("in-memory total size : {}", pretty(mem_total));
        println!("bloat                : {:.2}%",
            100.0 * (mem_total - bytes) as f64 / bytes as f64);
        println!("{:#?}", map.geometries.objects.len());

        elapsed += now.elapsed();
        total += 1;

    }
    println!("{:-<50}", "");
    println!("Read {total} MAP files in {:?} ({} bytes)\n",
        elapsed, pretty(total_bytes));

    Ok(())
}

fn pretty(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    const KB: f64  = 1024.0;

    if bytes < KB as _ {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= KB && unit < UNITS.len() - 1 {
        value /= KB;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn created(timestamp: u32) -> String {
    let timestamp = timestamp as i64;

    const SECS_PER_DAY: i64 = 86_400;

    let days = timestamp.div_euclid(SECS_PER_DAY);
    let seconds_of_day = timestamp.rem_euclid(SECS_PER_DAY);

    let (year, month, day) = civil_from_days(days);

    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02} UTC")
}

// Howard Hinnant `date.h` (public domain) for calculating Unix to UTC
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 14096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365*yoe + yoe/4 - yoe/100);
    let mp = (5*doy + 2) / 153;
    let d = doy - (153*mp + 2)/5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + (m <= 2) as i64;
    (year, m, d)
}
