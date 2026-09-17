use db::TokenHash;

#[test]
fn token_hash_accepts_only_64_lowercase_hexadecimal_characters() {
    let valid = "0123456789abcdef".repeat(4);
    assert_eq!(TokenHash::from_hex(valid.clone()).unwrap().as_str(), valid);
    for invalid in [
        "a".repeat(63),
        "a".repeat(65),
        format!("{}A", "a".repeat(63)),
        format!("{}g", "a".repeat(63)),
        "é".repeat(32),
    ] {
        assert!(TokenHash::from_hex(invalid).is_err());
    }
}

#[test]
fn transaction_creation_and_settings_have_one_source() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut directories = vec![root.clone()];
    let mut begin_sites = Vec::new();
    let mut setting_sites = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path)?;
            for line in source.lines() {
                if line.contains("pool.begin()") {
                    begin_sites.push(path.clone());
                }
                if line.contains("set_config(") {
                    setting_sites.push(path.clone());
                }
            }
        }
    }
    assert_eq!(begin_sites, [root.join("context.rs")]);
    assert_eq!(setting_sites, [root.join("context.rs")]);
    Ok(())
}
