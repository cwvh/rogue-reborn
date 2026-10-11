use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The commit being built, which debug dumps record
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Some(hash) = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    {
        println!("cargo:rustc-env=GIT_HASH={hash}");
    }
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "gnu" {
        provide_libgcc_eh();
    }
}

// Rust links the windows-gnu target with `-lgcc_eh`, but newer MinGW GCCs (such as
// w64devkit's GCC 15) fold that library into libgcc and no longer ship it, so the
// link fails with "cannot find -lgcc_eh". When the linker's GCC has none, give it an
// empty archive of that name: the unwinder symbols come from libgcc as before.
fn provide_libgcc_eh() {
    let linker = std::env::var("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER")
        .unwrap_or_else(|_| "x86_64-w64-mingw32-gcc".to_string());
    // Asks GCC where the library is: it prints just the bare name when it cannot find it
    let found = [linker.as_str(), "gcc"].iter().find_map(|gcc| {
        let out = Command::new(gcc).arg("-print-file-name=libgcc_eh.a").output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    });
    let Some(path) = found else { return };
    if path != "libgcc_eh.a" {
        return; // the real one exists
    }

    let dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    // The smallest valid archive: just the magic header
    std::fs::write(dir.join("libgcc_eh.a"), b"!<arch>\n").expect("write the libgcc_eh stand-in");
    println!("cargo:rustc-link-search=native={}", dir.display());
}
