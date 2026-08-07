const MANIFEST: &str = include_str!("../Cargo.toml");

const MACOS_DEPENDENCY_PATH: [&str; 3] = ["target", "cfg(target_os = \"macos\")", "dependencies"];

fn visit_objc2_dependencies(
    value: &toml::Value,
    path: &mut Vec<String>,
    visitor: &mut impl FnMut(&[String], &toml::Value),
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        path.push(key.clone());
        if key == "objc2" || key.starts_with("objc2-") {
            visitor(path, value);
        }
        visit_objc2_dependencies(value, path, visitor);
        path.pop();
    }
}

fn parsed_manifest() -> toml::Value {
    toml::from_str(MANIFEST).expect("Cargo.toml must be valid TOML")
}

#[test]
fn objc2_direct_dependencies_are_macos_only() {
    let manifest = parsed_manifest();
    let mut found = 0;

    visit_objc2_dependencies(&manifest, &mut Vec::new(), &mut |path, _| {
        found += 1;
        assert_eq!(
            &path[..path.len() - 1],
            MACOS_DEPENDENCY_PATH,
            "{} must be a direct macOS-only dependency",
            path.last().unwrap()
        );
    });
    assert!(
        found > 0,
        "at least one direct objc2 dependency is required"
    );
}

#[test]
fn objc2_direct_dependencies_disable_default_features() {
    let manifest = parsed_manifest();
    let mut found = 0;

    visit_objc2_dependencies(&manifest, &mut Vec::new(), &mut |path, dependency| {
        found += 1;
        assert_eq!(
            dependency
                .as_table()
                .and_then(|table| table.get("default-features"))
                .and_then(toml::Value::as_bool),
            Some(false),
            "{} must set default-features = false",
            path.last().unwrap()
        );
    });
    assert!(
        found > 0,
        "at least one direct objc2 dependency is required"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn required_objc2_framework_symbols_compile() {
    fn type_exists<T: ?Sized>() {}

    type_exists::<objc2_core_graphics::CGEvent>();
    type_exists::<objc2_application_services::AXUIElement>();
    type_exists::<objc2_app_kit::NSRunningApplication>();
    type_exists::<objc2_app_kit::NSWorkspace>();
    type_exists::<objc2_core_foundation::CFString>();
    type_exists::<objc2_quartz_core::CALayer>();
}
