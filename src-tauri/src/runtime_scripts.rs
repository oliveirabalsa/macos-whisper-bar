use anyhow::Context;
use tokio::fs;

use crate::app_state::SharedState;

const BOOTSTRAP_SCRIPT: &str = include_str!("../python/bootstrap.py");
const WORKER_SCRIPT: &str = include_str!("../python/worker.py");

pub async fn ensure_scripts(state: &SharedState) -> anyhow::Result<()> {
    let (scripts_dir, bootstrap_script, worker_script) = {
        let guard = state.0.lock().await;
        (
            guard.scripts_dir.clone(),
            guard.bootstrap_script.clone(),
            guard.worker_script.clone(),
        )
    };

    fs::create_dir_all(&scripts_dir).await.with_context(|| {
        format!(
            "failed creating scripts directory {}",
            scripts_dir.display()
        )
    })?;

    write_if_changed(&bootstrap_script, BOOTSTRAP_SCRIPT).await?;
    write_if_changed(&worker_script, WORKER_SCRIPT).await?;

    Ok(())
}

async fn write_if_changed(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    let needs_write = match fs::read_to_string(path).await {
        Ok(existing) => existing != content,
        Err(_) => true,
    };

    if needs_write {
        fs::write(path, content)
            .await
            .with_context(|| format!("failed writing {}", path.display()))?;
    }

    Ok(())
}
