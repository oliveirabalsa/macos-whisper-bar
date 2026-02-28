#!/usr/bin/env python3
"""WhisperBar bootstrap and model installer.

Creates a private virtual environment under app data, installs required packages,
and downloads the selected faster-whisper model.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import venv
from pathlib import Path

PACKAGES = [
    "pip>=24.0",
    "setuptools>=68",
    "wheel",
    "numpy>=1.24",
    "mlx-whisper>=0.4.2",
    "huggingface-hub>=0.24",
]

MODEL_REPOS = {
    "large-v3-turbo": "mlx-community/whisper-large-v3-turbo",
    "large-v3": "mlx-community/whisper-large-v3-mlx",
}

MODEL_FOLDERS = {
    "large-v3-turbo": "whisper-large-v3-turbo",
    "large-v3": "whisper-large-v3",
}


def emit(event_type: str, **fields: object) -> None:
    payload = {"type": event_type, **fields}
    print(json.dumps(payload), flush=True)


def run(command: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        text=True,
        capture_output=True,
        env=env,
        check=False,
    )


def venv_python_path(venv_dir: Path) -> Path:
    return venv_dir / "bin" / "python"


def check_python_ready(venv_python: Path) -> tuple[bool, str]:
    if not venv_python.exists():
        return False, "python virtual environment missing"

    probe = run(
        [
            str(venv_python),
            "-c",
            "import mlx_whisper; import numpy; import huggingface_hub",
        ]
    )
    if probe.returncode != 0:
        return False, probe.stderr.strip() or "missing Python dependencies"

    return True, "ready"


def check_model_ready(model_path: Path) -> tuple[bool, str]:
    if not model_path.exists():
        return False, "selected model is missing"

    config = model_path / "config.json"
    has_weights = any(model_path.glob("weights.*")) or any(model_path.glob("model*.safetensors"))
    if not config.exists() or not has_weights:
        return False, "selected model is missing"
    return True, "ready"


def install_venv(venv_dir: Path) -> None:
    emit("progress", progress=0.12, message="Creating Python environment")
    builder = venv.EnvBuilder(with_pip=True, clear=False, upgrade=False)
    builder.create(venv_dir)


def install_packages(venv_python: Path) -> None:
    emit("progress", progress=0.3, message="Installing Python packages")
    command = [str(venv_python), "-m", "pip", "install", "--upgrade", *PACKAGES]
    result = run(command)

    if result.returncode != 0:
        raise RuntimeError(
            "pip install failed: "
            + (result.stderr.strip() or result.stdout.strip() or "unknown error")
        )


def download_model(venv_python: Path, model_repo: str, model_path: Path, hf_home: Path) -> None:
    emit("progress", progress=0.62, message=f"Downloading model {model_path.name}")

    script = f"""
from huggingface_hub import snapshot_download
snapshot_download(
    repo_id={model_repo!r},
    local_dir={str(model_path)!r},
)
print("ok")
"""

    env = dict(os.environ)
    env["HF_HOME"] = str(hf_home)

    result = run([str(venv_python), "-c", script], env=env)
    if result.returncode != 0:
        raise RuntimeError(
            "model download failed: "
            + (result.stderr.strip() or result.stdout.strip() or "unknown error")
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app-data-dir", required=True)
    parser.add_argument("--model-id", default="large-v3-turbo")
    parser.add_argument("--reset", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    model_id = args.model_id.strip()
    model_repo = MODEL_REPOS.get(model_id)
    model_folder = MODEL_FOLDERS.get(model_id)
    if model_repo is None or model_folder is None:
        emit("error", message=f"Unsupported model id: {model_id}")
        return 1

    app_data_dir = Path(args.app_data_dir).expanduser().resolve()
    venv_dir = app_data_dir / "python-env"
    model_path = app_data_dir / "models" / model_folder
    hf_home = app_data_dir / "hf-cache"
    venv_python = venv_python_path(venv_dir)

    try:
        emit("progress", progress=0.02, message="Checking dependencies")

        if args.reset:
            emit("progress", progress=0.05, message="Resetting existing dependencies")
            if venv_dir.exists():
                shutil.rmtree(venv_dir)
            if model_path.exists():
                shutil.rmtree(model_path)

        python_ready, python_reason = check_python_ready(venv_python)
        model_ready, _ = check_model_ready(model_path)

        if python_ready and model_ready:
            emit("progress", progress=1.0, message=f"Model {model_id} already installed")
            emit(
                "ready",
                progress=1.0,
                message="Ready",
                model_id=model_id,
                venv_python=str(venv_python),
                model_path=str(model_path),
            )
            return 0

        emit("progress", progress=0.08, message=f"Preparing install ({python_reason})")
        app_data_dir.mkdir(parents=True, exist_ok=True)
        model_path.parent.mkdir(parents=True, exist_ok=True)
        hf_home.mkdir(parents=True, exist_ok=True)

        if not venv_python.exists():
            install_venv(venv_dir)

        python_ready, _ = check_python_ready(venv_python)
        if not python_ready:
            install_packages(venv_python)

        model_ready, _ = check_model_ready(model_path)
        if not model_ready:
            download_model(venv_python, model_repo, model_path, hf_home)

        python_ready, python_reason = check_python_ready(venv_python)
        model_ready, model_reason = check_model_ready(model_path)
        if not python_ready:
            raise RuntimeError(f"verification failed: {python_reason}")
        if not model_ready:
            raise RuntimeError(f"verification failed: {model_reason}")

        emit("progress", progress=1.0, message=f"Model {model_id} ready")
        emit(
            "ready",
            progress=1.0,
            message="Ready",
            model_id=model_id,
            venv_python=str(venv_python),
            model_path=str(model_path),
        )
        return 0
    except Exception as exc:  # noqa: BLE001
        emit("error", message=str(exc))
        return 1


if __name__ == "__main__":
    sys.exit(main())
