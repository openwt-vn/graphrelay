//! Memory-aware sizing for the full-index parallel parse pool.
//!
//! `Extractor::extract_many`'s rayon fan-out has no bound on concurrent
//! memory use by default — on a constrained environment (a small VPS, a
//! resource-limited container), parallelizing without a budget just moves
//! the failure mode from "sequential and slow" to "parallel and OOM-killed
//! faster." `sysinfo` gives a portable (Linux/macOS/Windows) read of
//! available memory in one call, in place of hand-rolling `/sys/fs/cgroup`
//! reads or shelling out to `vm_stat`.
//!
//! `sysinfo` reads host-level available memory, not a cgroup/container
//! memory limit — inside a memory-limited container (e.g. `docker run
//! --memory=2g`) it does not see that limit by default and would size the
//! pool as if the full host memory were available. On Linux, `available_bytes()`
//! now reads the container's real cgroup v2 (falling back to v1) limit
//! directly when one is set, closing that gap for the most common
//! containerized-deployment case; `sysinfo` is
//! still the fallback when no cgroup limit is found, and on macOS/Windows,
//! where cgroups don't exist, unchanged. `[watcher] memory_budget_mb` remains
//! the explicit override for cases cgroup detection can still get wrong (a
//! nested container, a non-standard cgroup mount point).

/// Read current system-available memory in bytes: a cgroup limit when one is
/// set, else a host-level `sysinfo` read.
pub fn available_bytes() -> u64 {
    if let Some(cgroup_bytes) = cgroup_available_bytes() {
        return cgroup_bytes;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    host_available(sys.available_memory(), sys.total_memory(), sys.used_memory())
}

/// `available_memory()` is not implemented on every platform: on macOS it reports 0,
/// which made `parse_pool_size` return 1 and forced every full index to parse on a
/// single core (10k-file keycloak: 248 s of parse that 8 cores do in a fraction).
/// A zero reading therefore means "unknown", not "no memory": fall back to
/// `total - used`, and to a quarter of physical memory if even that is unavailable.
fn host_available(available: u64, total: u64, used: u64) -> u64 {
    if available > 0 {
        return available;
    }
    let free = total.saturating_sub(used);
    if free > 0 { free } else { total / 4 }
}

/// cgroup v2 (falling back to v1) memory-limit read. Not `#[cfg]`-gated to
/// Linux — the `/sys/fs/cgroup/...` paths simply don't exist on macOS/
/// Windows, so `std::fs::read_to_string` naturally errors and this returns
/// `None` there without needing a platform check; cfg-gating it would only
/// make `parse_cgroup_limit` a dead-code warning on non-Linux builds where
/// its only non-test caller had been compiled away. Returns `None` when no
/// limit is set (an unlimited/"max" cgroup, or no cgroup at all — e.g.
/// running directly on bare metal), in which case `available_bytes()` falls
/// back to the `sysinfo` host read.
fn cgroup_available_bytes() -> Option<u64> {
    // cgroup v2: a single unified hierarchy under /sys/fs/cgroup.
    if let Some(bytes) = read_cgroup_available(
        std::path::Path::new("/sys/fs/cgroup/memory.max"),
        std::path::Path::new("/sys/fs/cgroup/memory.current"),
    ) {
        return Some(bytes);
    }
    // cgroup v1: the memory controller has its own subdirectory.
    read_cgroup_available(
        std::path::Path::new("/sys/fs/cgroup/memory/memory.limit_in_bytes"),
        std::path::Path::new("/sys/fs/cgroup/memory/memory.usage_in_bytes"),
    )
}

/// `limit - usage` from a (limit_path, usage_path) pair, or `None` if either
/// file is missing/unparseable, the limit is the "unlimited" sentinel (cgroup
/// v2 uses the literal string `"max"`; cgroup v1 uses a value so large it's
/// effectively a memory-address-space sentinel — treated as unlimited by
/// `parse_cgroup_limit`), or usage has (implausibly) exceeded the limit.
/// Kept separate from `cgroup_available_bytes()`'s hardcoded system paths so
/// the parsing logic itself is testable against synthetic files — a CI
/// runner won't reliably have a real cgroup mount to read.
fn read_cgroup_available(limit_path: &std::path::Path, usage_path: &std::path::Path) -> Option<u64> {
    let limit = parse_cgroup_limit(limit_path)?;
    let usage_text = std::fs::read_to_string(usage_path).ok()?;
    let usage: u64 = usage_text.trim().parse().ok()?;
    limit.checked_sub(usage)
}

/// Parse a cgroup memory-limit file's content. `None` means "no limit set"
/// (cgroup v2's literal `"max"`, or cgroup v1's convention of an
/// implausibly large value — traditionally `9223372036854771712`,
/// `i64::MAX` rounded down to a page boundary — both meaning "unlimited," not
/// "a 9-exabyte machine"). One arbitrary but generous cutoff (1 PiB) below
/// that sentinel distinguishes a real, if very large, limit from "unlimited."
fn parse_cgroup_limit(path: &std::path::Path) -> Option<u64> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text == "max" {
        return None;
    }
    let value: u64 = text.parse().ok()?;
    const ONE_PEBIBYTE: u64 = 1024 * 1024 * 1024 * 1024 * 1024;
    if value >= ONE_PEBIBYTE {
        return None; // cgroup v1's "unlimited" sentinel, or as good as.
    }
    Some(value)
}

/// Size the parse pool from available memory and a per-file cost estimate.
///
/// Returns `1` (sequential — no rayon pool needed) if the estimated cost of
/// even one file exceeds the budget, rather than returning `0` and leaving
/// callers to special-case "no thread pool." Otherwise returns
/// `min(cores, available / per_file_estimate)`, floored at 1.
pub fn parse_pool_size(available_bytes: u64, per_file_estimate_bytes: u64, cores: usize) -> usize {
    let cores = cores.max(1);
    if per_file_estimate_bytes == 0 {
        return cores;
    }
    let budget_allows = (available_bytes / per_file_estimate_bytes).max(1) as usize;
    cores.min(budget_allows).max(1)
}

/// Default per-file memory cost estimate used when sizing the parse pool —
/// covers the source-file byte buffer, the parsed tree-sitter AST, and
/// extracted symbol/edge structures for one file in flight. Deliberately
/// generous (most source files are far smaller) since undercounting risks
/// the failure mode this module exists to avoid.
pub const DEFAULT_PER_FILE_ESTIMATE_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB

/// Resolve the parse pool size for a full-index run: an explicit
/// `memory_budget_mb` override (if configured) takes precedence over the
/// live `sysinfo` reading, so a constrained-container deployment doesn't
/// have to depend on `sysinfo` correctly seeing a cgroup limit it can't.
pub fn resolve_pool_size(memory_budget_mb: Option<u64>, cores: usize) -> usize {
    let available = match memory_budget_mb {
        Some(mb) => mb.saturating_mul(1024 * 1024),
        None => available_bytes(),
    };
    parse_pool_size(available, DEFAULT_PER_FILE_ESTIMATE_BYTES, cores)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_available_reading_is_treated_as_unknown_not_as_no_memory() {
        let gib = 1u64 << 30;
        assert_eq!(host_available(6 * gib, 16 * gib, 10 * gib), 6 * gib, "a real reading wins");
        assert_eq!(host_available(0, 16 * gib, 12 * gib), 4 * gib, "0 falls back to total - used");
        assert_eq!(host_available(0, 16 * gib, 16 * gib), 4 * gib, "and to a quarter of total if that is 0 too");
        assert_eq!(host_available(0, 0, 0), 0, "nothing known at all stays 0 (pool floor of 1 applies)");
    }

    /// The real reading must be usable on whatever machine runs the tests: with a
    /// zero here every parse pool collapses to 1 core.
    #[test]
    fn the_live_reading_is_never_zero_on_a_machine_with_memory() {
        assert!(available_bytes() > 0);
    }

    #[test]
    fn parse_cgroup_limit_reads_a_real_value() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("memory.max");
        std::fs::write(&path, "2147483648").unwrap(); // 2 GiB
        assert_eq!(parse_cgroup_limit(&path), Some(2_147_483_648));
    }

    #[test]
    fn parse_cgroup_limit_treats_max_sentinel_as_unlimited() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("memory.max");
        std::fs::write(&path, "max\n").unwrap(); // cgroup v2's real format includes a trailing newline
        assert_eq!(parse_cgroup_limit(&path), None);
    }

    #[test]
    fn parse_cgroup_limit_treats_cgroup_v1_huge_sentinel_as_unlimited() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("memory.limit_in_bytes");
        // cgroup v1's traditional "unlimited" value.
        std::fs::write(&path, "9223372036854771712").unwrap();
        assert_eq!(parse_cgroup_limit(&path), None);
    }

    #[test]
    fn parse_cgroup_limit_missing_file_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(parse_cgroup_limit(&dir.path().join("does_not_exist")), None);
    }

    #[test]
    fn parse_cgroup_limit_garbage_content_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("memory.max");
        std::fs::write(&path, "not a number").unwrap();
        assert_eq!(parse_cgroup_limit(&path), None);
    }

    #[test]
    fn read_cgroup_available_computes_limit_minus_usage() {
        let dir = tempfile::TempDir::new().unwrap();
        let limit_path = dir.path().join("memory.max");
        let usage_path = dir.path().join("memory.current");
        std::fs::write(&limit_path, "1073741824").unwrap(); // 1 GiB
        std::fs::write(&usage_path, "268435456").unwrap();  // 256 MiB used
        assert_eq!(read_cgroup_available(&limit_path, &usage_path), Some(805_306_368)); // 768 MiB free
    }

    #[test]
    fn read_cgroup_available_none_when_limit_is_unlimited() {
        let dir = tempfile::TempDir::new().unwrap();
        let limit_path = dir.path().join("memory.max");
        let usage_path = dir.path().join("memory.current");
        std::fs::write(&limit_path, "max").unwrap();
        std::fs::write(&usage_path, "268435456").unwrap();
        assert_eq!(read_cgroup_available(&limit_path, &usage_path), None);
    }

    #[test]
    fn ample_memory_uses_all_cores() {
        let sixteen_gb = 16 * 1024 * 1024 * 1024u64;
        assert_eq!(parse_pool_size(sixteen_gb, DEFAULT_PER_FILE_ESTIMATE_BYTES, 8), 8);
    }

    #[test]
    fn tight_budget_shrinks_below_core_count() {
        // 32 MiB available / 8 MiB per file = 4 files worth of headroom, even
        // though 8 cores are available.
        let tight = 32 * 1024 * 1024u64;
        assert_eq!(parse_pool_size(tight, DEFAULT_PER_FILE_ESTIMATE_BYTES, 8), 4);
    }

    #[test]
    fn near_zero_budget_still_returns_at_least_one() {
        assert_eq!(parse_pool_size(1024, DEFAULT_PER_FILE_ESTIMATE_BYTES, 8), 1);
        assert_eq!(parse_pool_size(0, DEFAULT_PER_FILE_ESTIMATE_BYTES, 8), 1);
    }

    #[test]
    fn zero_cores_is_treated_as_one() {
        let sixteen_gb = 16 * 1024 * 1024 * 1024u64;
        assert_eq!(parse_pool_size(sixteen_gb, DEFAULT_PER_FILE_ESTIMATE_BYTES, 0), 1);
    }

    #[test]
    fn zero_per_file_estimate_falls_back_to_core_count() {
        assert_eq!(parse_pool_size(0, 0, 6), 6);
    }

    #[test]
    fn explicit_override_takes_precedence_over_live_reading() {
        // 16 MiB explicit override / 8 MiB per file = 2, regardless of what
        // the live host's actual available memory is.
        assert_eq!(resolve_pool_size(Some(16), 8), 2);
    }

    #[test]
    fn live_reading_used_when_no_override_given() {
        // Can't assert an exact value (depends on the test host), but it
        // must be a sane, non-zero pool size.
        let n = resolve_pool_size(None, 8);
        assert!((1..=8).contains(&n), "pool size {n} out of expected [1, 8] range");
    }
}
