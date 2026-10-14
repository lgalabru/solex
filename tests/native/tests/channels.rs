//! Runs under whichever channel `SOLEX_CHANNEL` selected at compile time
//! (default `stable`). `just test-native` covers all three.

use solex_native_tests as p;

#[test]
fn channel_matches_manifest_tags() {
    // (has stable, has nightly, has canary, nightly-only item compiled in)
    let got = (
        p::HAS_STABLE,
        p::HAS_NIGHTLY,
        p::HAS_CANARY,
        p::NIGHTLY_ITEM_PRESENT,
    );
    let expected = match p::CHANNEL {
        "stable" => (true, false, false, false),
        "nightly" => (true, true, false, true),
        "canary" => (true, true, true, true),
        other => panic!("unexpected channel {other}"),
    };
    assert_eq!(got, expected);
}

#[test]
fn program_id_follows_channel() {
    let expected = match p::CHANNEL {
        "stable" => "11111111111111111111111111111111",
        "nightly" => "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "canary" => "Sysvar1111111111111111111111111111111111111",
        other => panic!("unexpected channel {other}"),
    };
    assert_eq!(p::ID.to_string(), expected);
    assert!(p::check_id(&p::id()));
}

#[test]
fn ungated_items_run() {
    assert_eq!(p::stable_only().unwrap(), 1);
    let mut c = p::Counter(0);
    c.bump().unwrap();
    assert_eq!(c.0, 1);
    assert_eq!(p::program::ship_everywhere().unwrap(), 10);
}

/// Off-chain there is no Clock sysvar, so a gated function surfaces the
/// sysvar error from the gate instead of running its body. That proves the
/// gate executed; on-chain semantics are covered by the example program.
#[test]
fn epoch_gate_runs_before_body() {
    let mut c = p::Counter(0);
    let err = c.far_future().unwrap_err();
    assert_eq!(err, solex::ProgramError::UnsupportedSysvar);
    assert_eq!(c.0, 0, "body must not run when the gate fails");
}

#[test]
fn require_epoch_surfaces_sysvar_error_off_chain() {
    assert!(solex::current_epoch().is_err());
}
