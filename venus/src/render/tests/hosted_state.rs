//! The hosted backdrop belongs to the private Rune candidate (eon-yp6vg.7.7). With the
//! feature off, the renderer carries none of its state, as on edge.

const RENDER: &str = include_str!("../../render.rs");
const GATE: &str = "#[cfg(feature = \"candidate-rune\")]";

/// The `mod` line, the field, its initializer, and the two calls.
const SITES: usize = 5;

#[test]
fn hosted_backdrop_state_exists_only_with_the_candidate_feature() {
    let lines: Vec<&str> = RENDER.lines().collect();
    let mut sites = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let code = line.trim();
        if code.starts_with("//") || !code.contains("hosted_backdrop") {
            continue;
        }
        let above = index.checked_sub(1).map(|above| lines[above].trim());
        assert_eq!(
            above,
            Some(GATE),
            "render.rs:{} reaches the hosted backdrop outside candidate-rune: {code}",
            index + 1
        );
        sites.push(code);
    }
    assert_eq!(sites.len(), SITES, "{sites:#?}");
}
