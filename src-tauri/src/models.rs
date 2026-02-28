use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub size_label: &'static str,
    pub folder: &'static str,
}

pub const MODEL_SPECS: [ModelSpec; 2] = [
    ModelSpec {
        id: "large-v3-turbo",
        name: "Large v3 Turbo",
        size_label: "0.81 GB",
        folder: "whisper-large-v3-turbo",
    },
    ModelSpec {
        id: "large-v3",
        name: "Large v3",
        size_label: "3.10 GB",
        folder: "whisper-large-v3",
    },
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub size_label: String,
}

pub fn default_model_id() -> &'static str {
    "large-v3-turbo"
}

pub fn find_model(model_id: &str) -> Option<ModelSpec> {
    MODEL_SPECS
        .iter()
        .copied()
        .find(|model| model.id == model_id)
}

pub fn model_path(app_data_dir: &Path, model_id: &str) -> Option<PathBuf> {
    let model = find_model(model_id)?;
    Some(app_data_dir.join("models").join(model.folder))
}

pub fn model_options() -> Vec<ModelOption> {
    MODEL_SPECS
        .iter()
        .map(|model| ModelOption {
            id: model.id.to_string(),
            name: model.name.to_string(),
            size_label: model.size_label.to_string(),
        })
        .collect()
}
