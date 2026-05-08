use cli_framework::project_config::find_and_load;
use std::fs;
use tempfile::TempDir;

#[derive(serde::Deserialize, Debug, PartialEq)]
struct MyConfig {
    wiki_root: String,
    default_editor: Option<String>,
}

/// Build a 3-level nested directory tree inside a TempDir.
/// Returns (TempDir, leaf_path) where leaf is 3 levels below root.
fn three_level_tree(root: &TempDir) -> std::path::PathBuf {
    let leaf = root.path().join("a").join("b").join("c");
    fs::create_dir_all(&leaf).unwrap();
    leaf
}

#[test]
fn find_and_load_three_levels_deep() {
    let root = TempDir::new().unwrap();
    let leaf = three_level_tree(&root);

    // Write the TOML at the root
    let toml_content = "wiki_root = \"/home/user/wiki\"\ndefault_editor = \"nvim\"";
    fs::write(root.path().join("myapp.toml"), toml_content).unwrap();

    let (cfg, project_root) = find_and_load::<MyConfig>(&leaf, "myapp.toml").unwrap();

    assert_eq!(project_root.root_dir, root.path());
    assert_eq!(project_root.config_file, root.path().join("myapp.toml"));
    assert_eq!(cfg.wiki_root, "/home/user/wiki");
    assert_eq!(cfg.default_editor.as_deref(), Some("nvim"));
}

#[test]
fn find_and_load_file_at_leaf() {
    let root = TempDir::new().unwrap();
    let leaf = three_level_tree(&root);

    // Write the TOML at the leaf itself
    let toml_content = "wiki_root = \"/docs\"\n";
    fs::write(leaf.join("myapp.toml"), toml_content).unwrap();

    let (cfg, project_root) = find_and_load::<MyConfig>(&leaf, "myapp.toml").unwrap();

    assert_eq!(project_root.root_dir, leaf);
    assert_eq!(cfg.wiki_root, "/docs");
    assert_eq!(cfg.default_editor, None);
}
