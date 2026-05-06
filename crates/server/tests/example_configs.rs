use std::fs;
use std::path::{Path, PathBuf};

use codex_shim::config::Config;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn bundled_shim_examples_load_and_validate() {
    let root = workspace_root();
    let examples_dir = root.join("examples");
    let mut config_paths = Vec::new();

    for entry in fs::read_dir(&examples_dir).expect("read examples directory") {
        let entry = entry.expect("example entry");
        let path = entry.path();
        if path.is_dir() {
            let config_path = path.join("config.yaml");
            if config_path.exists() {
                config_paths.push(config_path);
            }
        }
    }

    config_paths.push(examples_dir.join("all-options.yaml"));
    config_paths.sort();

    for config_path in config_paths {
        let config = Config::load(Some(config_path.to_str().expect("utf-8 path")))
            .unwrap_or_else(|err| panic!("failed to load {}: {err}", config_path.display()));
        config
            .validate()
            .unwrap_or_else(|err| panic!("failed to validate {}: {err}", config_path.display()));
    }
}
