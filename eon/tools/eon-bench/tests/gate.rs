//! Tier 1: real attached Orbit, no timing assertion and no display.
#[test]
fn attached_quick_corpora_obey_frame_invariants() {
    let expected = [
        ("ascii", include_str!("fixtures/gate/ascii.txt")),
        (
            "styled-light",
            include_str!("fixtures/gate/styled-light.txt"),
        ),
        (
            "styled-heavy",
            include_str!("fixtures/gate/styled-heavy.txt"),
        ),
        ("unicode", include_str!("fixtures/gate/unicode.txt")),
        ("cursor", include_str!("fixtures/gate/cursor.txt")),
        ("cat64", include_str!("fixtures/gate/cat64.txt")),
    ];
    eon_bench::orbit::invariants::check(
        std::path::Path::new(env!("CARGO_BIN_EXE_eon-bench")),
        &std::path::PathBuf::from(
            std::env::var_os("ORBIT_BIN").expect("ORBIT_BIN set by eon-test"),
        ),
        &expected,
    )
    .unwrap();
}
