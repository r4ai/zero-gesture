const MANIFEST: &str = include_str!("../Cargo.toml");
const CI: &str = include_str!("../../.github/workflows/ci.yml");
const ADR: &str = include_str!("../../docs/adr/0022-objc2-macos-library-foundation.md");
const P04_CONTRACTS: [(&str, usize); 5] = [
    (include_str!("../../contracts/p04a-macos-packaging.json"), 8),
    (
        include_str!("../../contracts/p04b1-macos-uds-control.json"),
        38,
    ),
    (
        include_str!("../../contracts/p04b2-macos-event-tap-owner.json"),
        8,
    ),
    (
        include_str!("../../contracts/p04b3a-macos-context-resolver.json"),
        24,
    ),
    (
        include_str!("../../contracts/p04b3b-macos-action-executor.json"),
        17,
    ),
];

#[test]
fn objc2_dependencies_are_macos_only_and_disable_defaults() {
    let (before_macos, from_macos) = MANIFEST
        .split_once("[target.'cfg(target_os = \"macos\")'.dependencies]")
        .unwrap();
    let (macos, after_macos) = from_macos.split_once("[dev-dependencies]").unwrap();
    let objc2_lines = macos
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("objc2-"))
        .collect::<Vec<_>>();

    assert!(!objc2_lines.is_empty());
    assert!(objc2_lines
        .iter()
        .all(|line| line.contains("default-features = false")));
    assert!(!before_macos.contains("objc2-"));
    assert!(!after_macos.contains("objc2-"));
    assert!(macos.lines().any(|line| line.trim() == "libc = \"0.2\""));
}

#[test]
fn p04r0_behavior_neutral_delta_is_recorded() {
    assert!(ADR.contains("Reviewed base: `8e72da8`"));
    assert!(ADR.contains("Production Rust behavior delta: `0`"));
}

#[test]
fn existing_p04_contracts_remain_95_obligations() {
    let mut total = 0;
    for (contract, expected) in P04_CONTRACTS {
        let actual = contract.matches("\"id\": \"P04").count();
        assert_eq!(actual, expected);
        total += actual;
    }
    assert_eq!(total, 95);
    for callback_evidence in [
        "callback_core_normalizes_mouse_input_without_allocating",
        "callback_queue_overload_drops_new_input_and_preserves_fifo_order",
        "event_tap_spec_is_exactly_listen_only_mouse_observation",
        "self_tagged_callback_event_is_filtered_before_input_queue",
    ] {
        assert!(
            P04_CONTRACTS
                .iter()
                .any(|(contract, _)| contract.contains(callback_evidence)),
            "missing inherited callback evidence: {callback_evidence}"
        );
    }
}

#[test]
fn apple_silicon_ci_keeps_compile_and_packaging_gates() {
    for required in [
        "runs-on: macos-26",
        "targets: aarch64-apple-darwin",
        "--target aarch64-apple-darwin --all-targets",
        "--lib --test p04r0_objc2_foundation",
        "pnpm tauri build --debug --bundles app --target aarch64-apple-darwin --ci",
        "--test p04a_macos_packaging",
    ] {
        assert!(CI.contains(required), "missing macOS CI gate: {required}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn required_objc2_framework_symbols_compile() {
    fn type_exists<T: ?Sized>() {}

    type_exists::<objc2_core_graphics::CGEvent>();
    type_exists::<objc2_application_services::AXUIElement>();
    type_exists::<objc2_app_kit::NSApplication>();
    type_exists::<objc2_quartz_core::CALayer>();
}

#[test]
fn p04r0_quality_delta_is_recorded() {
    for delta in [
        "Production cognitive complexity delta: max `0`, sum `0`",
        "Production cyclomatic complexity delta: max `0`, sum `0`",
        "Production function delta: `0`",
        "Production PLOC delta: `0`",
        "Production unsafe-token delta: `0`",
    ] {
        assert!(ADR.contains(delta), "missing P04R0 quality record: {delta}");
    }
}
