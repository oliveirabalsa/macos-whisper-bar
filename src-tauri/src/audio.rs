use std::process::Stdio;

use regex::Regex;
use serde::Serialize;
use tokio::process::Command;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceOption {
    pub id: String,
    pub name: String,
    pub is_microphone_like: bool,
}

const MICROPHONE_KEYWORDS: [&str; 7] = [
    "microphone",
    "microfone",
    "mic",
    "built-in",
    "interno",
    "headset",
    "airpods",
];
const PREFERRED_BUILTIN_MIC_KEYWORDS: [&str; 3] = ["macbook", "built-in", "internal"];
const DEPRIORITIZED_MOBILE_MIC_KEYWORDS: [&str; 3] = ["iphone", "continuity", "desk view"];
const APP_VIRTUAL_AUDIO_KEYWORDS: [&str; 4] = ["teams audio", "zoomaudio", "discord", "slack"];

pub async fn list_audio_devices() -> anyhow::Result<Vec<AudioDeviceOption>> {
    let output = Command::new("ffmpeg")
        .arg("-f")
        .arg("avfoundation")
        .arg("-list_devices")
        .arg("true")
        .arg("-i")
        .arg("")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let re = Regex::new(r"\[(\d+)\]\s+(.+)$")?;
    let mut in_audio_section = false;
    let mut devices = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();

        if line.contains("AVFoundation audio devices") {
            in_audio_section = true;
            continue;
        }

        if line.contains("AVFoundation video devices") {
            in_audio_section = false;
            continue;
        }

        if !in_audio_section {
            continue;
        }

        let Some(caps) = re.captures(line) else {
            continue;
        };

        let Some(id_match) = caps.get(1) else {
            continue;
        };
        let Some(name_match) = caps.get(2) else {
            continue;
        };

        let id = id_match.as_str().to_string();
        let name = name_match.as_str().trim().to_string();
        let lowered = name.to_lowercase();

        devices.push(AudioDeviceOption {
            id,
            name,
            is_microphone_like: contains_any(&lowered, &MICROPHONE_KEYWORDS),
        });
    }

    Ok(devices)
}

pub fn choose_default_mic(devices: &[AudioDeviceOption]) -> Option<String> {
    devices
        .iter()
        .max_by_key(|device| score_mic(&device.name))
        .map(|device| device.id.clone())
}

fn contains_any(input: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| input.contains(term))
}

fn score_mic(name: &str) -> i32 {
    let lowered = name.to_lowercase();
    let mut score = 0;

    if contains_any(&lowered, &MICROPHONE_KEYWORDS) {
        score += 160;
    }
    if contains_any(&lowered, &PREFERRED_BUILTIN_MIC_KEYWORDS) {
        score += 35;
    }
    if contains_any(&lowered, &DEPRIORITIZED_MOBILE_MIC_KEYWORDS) {
        score -= 30;
    }
    if contains_any(&lowered, &APP_VIRTUAL_AUDIO_KEYWORDS) {
        score -= 130;
    }

    score
}
