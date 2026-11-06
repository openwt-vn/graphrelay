use std::process::Command;

/// Embeds a build fingerprint into the binary: the git commit it was built
/// from, whether the working tree was dirty, and — if dirty — a short
/// checksum of the uncommitted diff, so two builds from different uncommitted
/// states are distinguishable. Exposed via `gr --version` and the
/// `/health` endpoint's `version` field.
///
/// Deliberately emits no `cargo:rerun-if-changed` directives: with none given,
/// cargo reruns this script on every build rather than caching its output,
/// which is required here — `.git/index` only changes on `git add`/`commit`,
/// not on a plain working-tree edit, so watching it would let the embedded
/// fingerprint go stale across incremental rebuilds of a dirty tree.
fn main() {
    let sha = git_output(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);

    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());

    let full_version = if dirty {
        let diff = Command::new("git")
            .args(["diff", "HEAD"])
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default();
        let checksum = xxhash_rust::xxh64::xxh64(&diff, 0);
        format!("{pkg_version} (commit {sha}, dirty, diff-checksum {checksum:016x})")
    } else {
        format!("{pkg_version} (commit {sha}, clean)")
    };

    println!("cargo:rustc-env=GRAPHRELAY_FULL_VERSION={full_version}");
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}
