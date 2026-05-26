use std::{env, fs::File};

use swpui::app::App;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    if matches!(env::var("DEBUG").as_deref(), Ok("1" | "true")) {
        let log_file = File::create("swpui.log")?;
        tracing_subscriber::fmt()
            .with_writer(log_file)
            .with_ansi(false)
            .with_env_filter(EnvFilter::new("swpui=debug"))
            .init();
    }

    let root = env::current_dir()?;
    let config = swpui::config::Loader::load(&root);
    ratatui::run(|term| App::new(root, config)?.run(term))?;
    Ok(())
}
