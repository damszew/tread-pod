use std::process::Command;

fn main() {
    embuild::espidf::sysenv::output();

    // Firmware revision shown by the watch (Device Information): the crate version plus the
    // commit, e.g. `0.1.0 (318e493)`, with `-dirty` for uncommitted changes.
    let commit = Command::new("git")
        .args([
            "describe",
            "--always",
            "--dirty",
            "--abbrev=7",
            "--exclude=*",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let version = env!("CARGO_PKG_VERSION");
    let revision = match commit {
        Some(commit) => format!("{version} ({commit})"),
        None => version.to_owned(),
    };
    println!("cargo:rustc-env=TREAD_POD_FIRMWARE_REVISION={revision}");
    // Rebuild when HEAD moves (new commit or branch switch).
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
}
