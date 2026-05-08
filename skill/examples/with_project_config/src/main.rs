use cli_framework::project_config::find_and_load;

#[derive(serde::Deserialize, Debug)]
struct MyConfig {
    pub wiki_root: String,
    pub default_editor: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let start = std::env::current_dir()?;
    match find_and_load::<MyConfig>(&start, "myapp.toml") {
        Ok((cfg, root)) => {
            println!("Config loaded from: {:?}", root.root_dir);
            println!("wiki_root: {}", cfg.wiki_root);
            if let Some(editor) = cfg.default_editor {
                println!("default_editor: {}", editor);
            }
        }
        Err(e) => {
            eprintln!("Config not found or failed to load: {}", e);
        }
    }
    Ok(())
}
