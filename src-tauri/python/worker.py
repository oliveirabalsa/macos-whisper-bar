#!/usr/bin/env python3
"""WhisperBar local transcription worker.

Captures microphone + desktop audio on macOS and streams partial transcript
chunks as JSON lines to stdout.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import threading
import traceback
from pathlib import Path
from queue import SimpleQueue

import mlx_whisper
import numpy as np

MICROPHONE_KEYWORDS = (
    "microphone",
    "microfone",
    "mic",
    "built-in",
    "interno",
    "headset",
    "airpods",
)
PREFERRED_BUILTIN_MIC_KEYWORDS = ("macbook", "built-in", "internal")
DEPRIORITIZED_MOBILE_MIC_KEYWORDS = ("iphone", "continuity", "desk view")
APP_VIRTUAL_AUDIO_KEYWORDS = ("teams audio", "zoomaudio", "discord", "slack")


def emit(event_type: str, **fields: object) -> None:
    payload = {"type": event_type, **fields}
    print(json.dumps(payload), flush=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--language", default="en")
    parser.add_argument("--model-path", required=True)
    parser.add_argument("--mic-device", default="")
    parser.add_argument("--sck-helper-path", default="")
    parser.add_argument("--chunk-seconds", type=float, default=2.8)
    return parser.parse_args()


def normalize_language(language: str) -> str:
    normalized = language.strip().lower().replace("_", "-")
    if normalized in {"pt-br", "ptbr", "pt"} or normalized.startswith("pt-"):
        return "pt"
    return "en"


def _contains_any(text: str, keywords: tuple[str, ...]) -> bool:
    lowered = text.lower()
    return any(keyword in lowered for keyword in keywords)


def list_audio_devices() -> list[tuple[str, str]]:
    ffmpeg_bin = resolve_ffmpeg_binary()
    command = [ffmpeg_bin, "-f", "avfoundation", "-list_devices", "true", "-i", ""]
    result = subprocess.run(command, text=True, capture_output=True, check=False)

    output = "\n".join([result.stdout, result.stderr])
    devices: list[tuple[str, str]] = []

    regex = re.compile(r"\[(\d+)\]\s+(.+)$")
    in_audio_section = False

    for raw_line in output.splitlines():
        line = raw_line.strip()

        if "AVFoundation audio devices" in line:
            in_audio_section = True
            continue
        if "AVFoundation video devices" in line:
            in_audio_section = False
            continue
        if not in_audio_section:
            continue

        match = regex.search(line)
        if not match:
            continue

        index, name = match.group(1), match.group(2).strip()
        devices.append((index, name))

    return devices


def resolve_device(ref: str, devices: list[tuple[str, str]]) -> tuple[str, str] | None:
    trimmed = ref.strip()
    if not trimmed:
        return None
    if trimmed.lower() in {"none", "__none__"}:
        return None

    if trimmed.startswith(":"):
        idx = trimmed[1:]
        for dev_idx, dev_name in devices:
            if dev_idx == idx:
                return f":{dev_idx}", dev_name
        return trimmed, f"custom {trimmed}"

    if trimmed.isdigit():
        for dev_idx, dev_name in devices:
            if dev_idx == trimmed:
                return f":{dev_idx}", dev_name
        return f":{trimmed}", f"device {trimmed}"

    lowered = trimmed.lower()
    for dev_idx, dev_name in devices:
        if lowered in dev_name.lower():
            return f":{dev_idx}", dev_name

    return None


def score_mic(name: str) -> int:
    lowered = name.lower()
    score = 0

    if _contains_any(lowered, MICROPHONE_KEYWORDS):
        score += 160
    if _contains_any(lowered, PREFERRED_BUILTIN_MIC_KEYWORDS):
        score += 35
    if _contains_any(lowered, DEPRIORITIZED_MOBILE_MIC_KEYWORDS):
        score -= 30
    if _contains_any(lowered, APP_VIRTUAL_AUDIO_KEYWORDS):
        score -= 130

    return score


def pick_microphone_device(devices: list[tuple[str, str]], preferred: str) -> tuple[str, str]:
    explicit = resolve_device(preferred, devices)
    if explicit is not None:
        return explicit

    ranked = sorted(devices, key=lambda item: score_mic(item[1]), reverse=True)
    idx, name = ranked[0]
    return f":{idx}", name


def spawn_ffmpeg_mic_only(mic_input: str) -> subprocess.Popen[bytes]:
    ffmpeg_bin = resolve_ffmpeg_binary()
    command = [
        ffmpeg_bin,
        "-hide_banner",
        "-loglevel",
        "warning",
        "-thread_queue_size",
        "512",
        "-f",
        "avfoundation",
        "-i",
        mic_input,
        "-ac",
        "1",
        "-ar",
        "16000",
        "-f",
        "s16le",
        "-",
    ]
    return subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def spawn_screencapturekit_helper(helper_path: str) -> subprocess.Popen[bytes]:
    command = [helper_path, "--sck-audio-helper"]
    return subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def choose_optional_mic(args: argparse.Namespace) -> tuple[str | None, str | None]:
    devices = list_audio_devices()
    if not devices:
        return None, None

    preferred_mic = args.mic_device or os.environ.get("WHISPERBAR_MIC_DEVICE", "").strip()
    if preferred_mic.lower() in {"none", "__none__"}:
        return None, None

    mic_input, mic_name = pick_microphone_device(devices, preferred_mic)
    return mic_input, mic_name


def mix_pcm_streams(primary: bytes, secondary: bytes | None) -> bytes:
    first = np.frombuffer(primary, dtype=np.int16).astype(np.float32)
    second = (
        np.frombuffer(secondary, dtype=np.int16).astype(np.float32)
        if secondary
        else np.empty(0, dtype=np.float32)
    )

    if first.size == 0 and second.size == 0:
        return b""

    target_len = int(max(first.size, second.size))
    mixed = np.zeros(target_len, dtype=np.float32)
    if first.size:
        mixed[: first.size] += first * 0.8
    if second.size:
        second_gain = 1.35
        if first.size:
            first_rms = rms_level(first / 32768.0)
            second_rms = rms_level(second / 32768.0)
            if first_rms > 1e-4 and second_rms > 1e-4:
                # Keep mic intelligible when desktop audio is louder.
                second_gain *= float(np.clip(first_rms / second_rms, 0.8, 2.2))
        mixed[: second.size] += second * second_gain

    np.clip(mixed, -32768.0, 32767.0, out=mixed)
    return mixed.astype(np.int16).tobytes()


def start_stderr_reader(process: subprocess.Popen[bytes], queue: SimpleQueue[str]) -> threading.Thread:
    def stderr_reader() -> None:
        if process.stderr is None:
            return
        for raw in process.stderr:
            try:
                text = raw.decode("utf-8", errors="replace").strip()
            except Exception:  # noqa: BLE001
                text = str(raw)
            if text:
                queue.put(text)

    thread = threading.Thread(target=stderr_reader, daemon=True)
    thread.start()
    return thread


def drain_queue(queue: SimpleQueue[str]) -> str:
    lines: list[str] = []
    while not queue.empty():
        lines.append(queue.get())
    return "\n".join(lines)


def rms_level(audio: np.ndarray) -> float:
    if audio.size == 0:
        return 0.0
    return float(np.sqrt(np.mean(np.square(audio), dtype=np.float64)))


def main() -> int:
    args = parse_args()
    language = normalize_language(args.language)
    model_path = Path(args.model_path).expanduser().resolve()

    if resolve_ffmpeg_binary(raise_if_missing=False) is None:
        emit(
            "error",
            message=(
                "ffmpeg is required for microphone capture. "
                "Install it with Homebrew (`brew install ffmpeg`) or set WHISPERBAR_FFMPEG_PATH."
            ),
        )
        return 1

    stop_event = threading.Event()

    def stop_from_stdin() -> None:
        for line in sys.stdin:
            if line.strip().lower() == "stop":
                stop_event.set()
                break

    def stop_from_signal(_signum: int, _frame: object) -> None:
        stop_event.set()

    signal.signal(signal.SIGINT, stop_from_signal)
    signal.signal(signal.SIGTERM, stop_from_signal)

    stdin_thread = threading.Thread(target=stop_from_stdin, daemon=True)
    stdin_thread.start()

    desktop_proc: subprocess.Popen[bytes] | None = None
    mic_proc: subprocess.Popen[bytes] | None = None
    desktop_stderr_queue: SimpleQueue[str] | None = None
    mic_stderr_queue: SimpleQueue[str] | None = None

    try:
        emit("status", message="Loading model")

        sample_rate = 16000
        bytes_per_second = sample_rate * 2
        # Slightly shorter chunks reduce missed transitions in conversational speech.
        chunk_bytes = max(int(bytes_per_second * args.chunk_seconds), int(bytes_per_second * 1.2))

        helper_path = args.sck_helper_path.strip()
        if not helper_path:
            raise RuntimeError("ScreenCaptureKit helper path is missing")
        if not Path(helper_path).exists():
            raise RuntimeError(f"ScreenCaptureKit helper binary not found: {helper_path}")

        mic_input, mic_name = choose_optional_mic(args)
        if mic_input and mic_name:
            emit(
                "status",
                message=f"Listening desktop (ScreenCaptureKit) + mic: {mic_name}",
            )
        else:
            emit("status", message="Listening desktop (ScreenCaptureKit)")

        desktop_proc = spawn_screencapturekit_helper(helper_path)
        if desktop_proc.stdout is None:
            raise RuntimeError("ScreenCaptureKit helper stdout unavailable")
        if desktop_proc.stderr is None:
            raise RuntimeError("ScreenCaptureKit helper stderr unavailable")
        desktop_stderr_queue = SimpleQueue()
        start_stderr_reader(desktop_proc, desktop_stderr_queue)

        if mic_input:
            mic_proc = spawn_ffmpeg_mic_only(mic_input)
            if mic_proc.stdout is None:
                raise RuntimeError("microphone ffmpeg stdout unavailable")
            if mic_proc.stderr is None:
                raise RuntimeError("microphone ffmpeg stderr unavailable")
            mic_stderr_queue = SimpleQueue()
            start_stderr_reader(mic_proc, mic_stderr_queue)

        collected: list[str] = []

        while not stop_event.is_set():
            if desktop_proc is None or desktop_proc.stdout is None:
                raise RuntimeError("desktop capture process is not running")

            desktop_bytes = desktop_proc.stdout.read(chunk_bytes)
            if not desktop_bytes:
                if desktop_proc.poll() is not None:
                    stderr_text = (
                        drain_queue(desktop_stderr_queue)
                        if desktop_stderr_queue is not None
                        else ""
                    )
                    raise RuntimeError(
                        "ScreenCaptureKit helper stopped unexpectedly. "
                        + (stderr_text or "Check Screen Recording permission in macOS settings.")
                    )
                continue

            mic_bytes = b""
            if mic_proc is not None and mic_proc.stdout is not None:
                mic_chunk = mic_proc.stdout.read(chunk_bytes)
                if mic_chunk:
                    mic_bytes = mic_chunk
                elif mic_proc.poll() is not None:
                    mic_error = (
                        drain_queue(mic_stderr_queue)
                        if mic_stderr_queue is not None
                        else ""
                    )
                    emit(
                        "status",
                        message=(
                            "Microphone capture stopped; continuing with desktop only. "
                            + (mic_error or "Check macOS Microphone permission.")
                        ),
                    )
                    mic_proc = None
                    mic_stderr_queue = None

            pcm_bytes = mix_pcm_streams(desktop_bytes, mic_bytes)

            if not pcm_bytes:
                if desktop_proc.poll() is not None:
                    stderr_text = (
                        drain_queue(desktop_stderr_queue)
                        if desktop_stderr_queue is not None
                        else ""
                    )
                    raise RuntimeError(
                        "ffmpeg stopped unexpectedly. "
                        + (stderr_text or "Check microphone permissions and selected input devices.")
                    )
                continue

            pcm = np.frombuffer(pcm_bytes, dtype=np.int16).astype(np.float32) / 32768.0
            if pcm.size < int(sample_rate * 0.8):
                continue
            if rms_level(pcm) < 0.0006:
                continue

            result = mlx_whisper.transcribe(
                pcm,
                path_or_hf_repo=str(model_path),
                language=language,
                # Lower no_speech_threshold keeps short Portuguese/English fragments.
                no_speech_threshold=0.45,
                temperature=0.0,
                condition_on_previous_text=True,
                word_timestamps=False,
            )
            chunk_text = str(result.get("text", "")).strip()
            if chunk_text:
                collected.append(chunk_text)
                emit("partial", text=chunk_text)

        final_text = "\n".join(collected).strip()
        emit("final", text=final_text)
        emit("status", message="Worker stopped")
        return 0
    except Exception as exc:  # noqa: BLE001
        emit("error", message=f"{exc}\n{traceback.format_exc()}")
        return 1
    finally:
        for proc in [desktop_proc, mic_proc]:
            if proc is None:
                continue
            if proc.poll() is not None:
                continue
            try:
                if proc.stdin is not None:
                    proc.stdin.write(b"stop\n")
                    proc.stdin.flush()
            except Exception:  # noqa: BLE001
                pass
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()


def shutil_which(binary: str) -> str | None:
    from shutil import which

    return which(binary)


def resolve_ffmpeg_binary(raise_if_missing: bool = True) -> str | None:
    explicit = os.environ.get("WHISPERBAR_FFMPEG_PATH", "").strip()
    candidates: list[str] = []
    if explicit:
        candidates.append(explicit)

    in_path = shutil_which("ffmpeg")
    if in_path:
        candidates.append(in_path)

    candidates.extend(
        [
            "/opt/homebrew/bin/ffmpeg",  # Apple Silicon Homebrew default
            "/usr/local/bin/ffmpeg",     # Intel Homebrew default
            "/usr/bin/ffmpeg",           # Rare fallback
        ]
    )

    for candidate in candidates:
        path = Path(candidate)
        if path.exists() and path.is_file():
            return str(path)

    if raise_if_missing:
        raise RuntimeError("ffmpeg binary not found")
    return None


if __name__ == "__main__":
    sys.exit(main())
