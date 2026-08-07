const MANIFEST: &str = include_str!("../Cargo.toml");

fn dependency_sections() -> (&'static str, &'static str, &'static str) {
    let (before_macos, from_macos) = MANIFEST
        .split_once("[target.'cfg(target_os = \"macos\")'.dependencies]")
        .unwrap();
    let (macos, after_macos) = from_macos.split_once("[dev-dependencies]").unwrap();
    (before_macos, macos, after_macos)
}

#[test]
fn objc2_direct_dependencies_are_macos_only() {
    let (before_macos, macos, after_macos) = dependency_sections();

    assert!(macos.lines().any(|line| line.trim().starts_with("objc2-")));
    assert!(!before_macos.contains("objc2-"));
    assert!(!after_macos.contains("objc2-"));
}

#[test]
fn objc2_direct_dependencies_disable_default_features() {
    let objc2_dependencies = dependency_sections()
        .1
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("objc2-"))
        .collect::<Vec<_>>();

    assert!(!objc2_dependencies.is_empty());
    assert!(objc2_dependencies
        .iter()
        .all(|line| line.contains("default-features = false")));
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
