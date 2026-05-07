use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn provider_compatibility_doc_mentions_every_canonical_profile() {
    let path = workspace_root().join("docs/provider-compatibility.md");
    let content = fs::read_to_string(&path).expect("read provider compatibility doc");

    let documented: BTreeSet<String> = content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("| `") {
                return None;
            }
            let first = trimmed.split('|').nth(1)?.trim();
            Some(first.trim_matches('`').to_string())
        })
        .collect();

    let expected: BTreeSet<String> = providers::CANONICAL_PROFILE_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    assert_eq!(documented, expected, "{}", path.display());
}
