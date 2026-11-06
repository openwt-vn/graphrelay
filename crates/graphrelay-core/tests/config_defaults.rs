use graphrelay_core::config::{Config, IndexConfig, PathPinMode, WatcherConfig};

#[test]
fn default_ignore_index_excludes_common_dependency_directories() {
    let ignore = IndexConfig::default().ignore;
    for pattern in [".venv/**", "venv/**", "__pycache__/**", "vendor/**"] {
        assert!(
            ignore.iter().any(|p| p == pattern),
            "expected default [index].ignore to contain {pattern:?}, got {ignore:?}"
        );
    }
}

#[test]
fn default_ignore_watcher_excludes_common_dependency_directories() {
    let ignore = WatcherConfig::default().ignore;
    for pattern in [".venv/**", "venv/**", "__pycache__/**", "vendor/**"] {
        assert!(
            ignore.iter().any(|p| p == pattern),
            "expected default [watcher].ignore to contain {pattern:?}, got {ignore:?}"
        );
    }
}

/// Regression test: `IndexConfig::default()`/`WatcherConfig::default()` used to
/// hardcode their own separate copies of the ignore list instead of calling the
/// same serde-default function TOML deserialization uses, so a pattern added to
/// one silently didn't apply to the other -- the same duplication shape that
/// previously caused 3 disagreeing definitions of the default embedding model.
#[test]
fn index_and_watcher_defaults_agree_with_toml_deserialization_defaults() {
    let via_default = IndexConfig::default().ignore;
    let via_toml: Config = toml::from_str("").unwrap();
    assert_eq!(via_default, via_toml.index.ignore);

    let via_default = WatcherConfig::default().ignore;
    let via_toml: Config = toml::from_str("").unwrap();
    assert_eq!(via_default, via_toml.watcher.ignore);
}

/// Regression test for a real bug found during Plan 22 item 3's own
/// validation sweep: a `[retrieval.path_pin]` table that specifies some
/// fields but omits `mode` must fall back to the *same* default as an
/// entirely absent `path_pin` table. `PathPinConfig::default()` used to
/// hardcode `PathPinMode::Off` directly instead of calling
/// `PathPinMode::default()` — harmless while both defaults happened to
/// agree, but exactly the kind of silent two-defaults drift that caused an
/// entire day's worth of "arm" measurements to silently compare `Off`
/// against `Off` on every corpus (a pre-existing per-project
/// `.graphrelay/config.toml` supplied a `[retrieval.path_pin]` table with
/// only `mode` overridden in some experiments, but any table specifying only
/// `bonus` and omitting `mode` would have silently resolved to `Off` if this
/// drift shipped instead of being caught).
#[test]
fn path_pin_mode_omitted_from_an_explicit_table_matches_the_absent_table_default() {
    let via_default = graphrelay_core::config::PathPinConfig::default().mode;

    let absent_table: Config = toml::from_str("").unwrap();
    assert_eq!(absent_table.retrieval.path_pin.mode, via_default);

    let partial_table: Config = toml::from_str("[retrieval.path_pin]\nbonus = 0.3\n").unwrap();
    assert_eq!(
        partial_table.retrieval.path_pin.mode, via_default,
        "a [retrieval.path_pin] table that omits `mode` must default identically \
         to an absent table, not silently fall back to Off"
    );

    // Pin the actual shipped value, not just internal consistency — this is
    // the real regression: the default must be `Strip` (arm A, shipped after
    // measurement), not `Off`.
    assert_eq!(via_default, PathPinMode::Strip);
}

/// Plan 22 item 4: config-driven PPR iteration count / early-stop epsilon /
/// direct-hit graph-credit damping must default to exactly the pre-item-4
/// hardcoded constants (4 iterations, no early stop, damping 0.75) — this
/// item's whole job is making a future sweep config-driven, not changing
/// today's shipped behavior.
#[test]
fn ppr_and_damping_defaults_match_pre_item_4_constants() {
    let cfg: Config = toml::from_str("").unwrap();
    assert_eq!(cfg.retrieval.ppr.iterations, 4);
    assert_eq!(cfg.retrieval.ppr.early_stop_epsilon, None);
    assert_eq!(cfg.retrieval.direct_hit_graph_credit_damping, 0.75);
}

// ── Plan 25: Laya judge configuration ────────────────────────────────────────

#[test]
fn no_judge_is_enabled_by_default() {
    let c = graphrelay_core::Config::default();
    assert!(!c.retrieval.laya.enabled && !c.retrieval.jev.enabled);
    assert_eq!(c.retrieval.judge_provider(), None);
}

#[test]
fn laya_defaults_are_the_measured_conservative_ones() {
    let l = graphrelay_core::config::LayaConfig::default();
    assert_eq!(l.top_n, 20, "each candidate costs one forward pass");
    assert!((l.weight - 0.05).abs() < f32::EPSILON, "decision-rule default (Plan 25)");
    assert_eq!(l.concurrency, 4);
    assert!(l.api_key.is_none() && l.model.is_empty() && l.max_len.is_none());
    assert!(l.base_url.ends_with("/v1/systemone"));
}

#[test]
fn laya_section_deserializes_and_selects_the_provider() {
    let c: graphrelay_core::Config = toml::from_str(
        r#"
        [retrieval.laya]
        enabled = true
        base_url = "http://gpu-box:9000/v1/systemone"
        model = "multilingual"
        weight = 0.6
        concurrency = 2
        max_len = 1024
        "#,
    )
    .unwrap();
    assert_eq!(c.retrieval.judge_provider(), Some(graphrelay_core::config::JudgeProvider::Laya));
    assert_eq!(c.retrieval.laya.model, "multilingual");
    assert_eq!(c.retrieval.laya.max_len, Some(1024));
    assert_eq!(c.retrieval.judge_top_n(), 20, "unset keys keep their defaults");
    assert!((c.retrieval.judge_weight() - 0.6).abs() < f32::EPSILON);
}

#[test]
fn jev_takes_precedence_when_both_judges_are_enabled() {
    let c: graphrelay_core::Config = toml::from_str(
        r#"
        [retrieval.jev]
        enabled = true
        top_n = 50
        [retrieval.laya]
        enabled = true
        top_n = 7
        "#,
    )
    .unwrap();
    assert_eq!(c.retrieval.judge_provider(), Some(graphrelay_core::config::JudgeProvider::Jev));
    assert_eq!(c.retrieval.judge_top_n(), 50, "the active provider's pool width, not Laya's");
}
