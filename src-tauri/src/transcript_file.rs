use anyhow::{anyhow, Context};
use chrono::Local;
use tokio::fs;

pub async fn save_markdown(transcript: &str) -> anyhow::Result<std::path::PathBuf> {
    let documents_dir =
        dirs::document_dir().ok_or_else(|| anyhow!("unable to locate Documents directory"))?;
    let output_dir = documents_dir.join("WhisperBar");
    fs::create_dir_all(&output_dir)
        .await
        .with_context(|| format!("failed creating {}", output_dir.display()))?;

    let timestamp = Local::now().format("%Y-%m-%d-%H-%M");
    let file_path = output_dir.join(format!("Transcript-{timestamp}.md"));
    fs::write(&file_path, transcript)
        .await
        .with_context(|| format!("failed writing {}", file_path.display()))?;

    Ok(file_path)
}
