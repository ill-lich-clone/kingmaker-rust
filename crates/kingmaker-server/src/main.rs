use kingmaker_io::{runtime_config::default_runtime_path, RuntimeConfig};
use kingmaker_server::{routes::create_router, state::AppState};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut runtime = default_runtime_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runtime" => {
                runtime = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--runtime requires a path"))?,
                )
            }
            "--help" | "-h" => {
                println!("kingmaker-server --runtime <runtime.json>\nRead-only API. Native RAM week implemented; full-world commands await checkpoint and runner validation.");
                return Ok(());
            }
            _ => anyhow::bail!("Unknown argument: {arg}"),
        }
    }
    let config = RuntimeConfig::load(&runtime)?;
    println!("Rust: чтение и диагностика. Расчёт RAM-недели реализован; запуск полного мира ожидает проверки сохранения и программы прогона.");
    println!(
        "База: {}; кампания: {}; workers={} (настройка исходного runtime)",
        config.db_path.display(),
        config.campaign_id,
        config.workers
    );
    let start = std::time::Instant::now();
    let world = config.load_world()?;
    println!(
        "Загрузка завершена за {:.2?}: {} провинций, {} ПОПов. Дата сохранена.",
        start.elapsed(),
        world.provinces.len(),
        world.pops.len()
    );
    let state = AppState {
        world: Arc::new(RwLock::new(world)),
        recipes: Arc::new(HashMap::new()),
    };
    let viewer = config.project_root.join("viewer");
    let app = create_router(state)
        .nest_service("/viewer", ServeDir::new(&viewer))
        .fallback_service(ServeDir::new(&viewer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    println!("API чтения: http://127.0.0.1:8080/api/health");
    println!("Полный viewer API и игровые команды ещё не перенесены.");
    axum::serve(listener, app).await?;
    Ok(())
}
