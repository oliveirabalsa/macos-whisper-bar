# WhisperBar (MVP)

WhisperBar is a macOS menu bar transcription app built with Tauri v2 + React + a local Python `mlx-whisper` worker.

It provides:
- Menu bar icon + compact dropdown panel
- Start/Stop recording controls
- Language selector: `en` and `pt-BR`
- Draggable dark-mode tray/floating windows
- Model selector (Large v3 Turbo / Large v3)
- Dedicated microphone selector
- Live floating transcript window during recording
- Markdown transcript export on stop to `~/Documents/WhisperBar/`

## Stack

- macOS desktop shell: Tauri v2 (Rust)
- Frontend UI: React + TypeScript + Vite
- Local transcription: Python worker + `mlx-whisper`
- Audio capture: ScreenCaptureKit (desktop audio) + `ffmpeg`/`avfoundation` (microphone)

## Prerequisites

- macOS (Apple Silicon supported)
- Rust toolchain
- Node.js 20+
- Python 3.9+
- `ffmpeg` installed and available in `PATH`
  - Install with Homebrew: `brew install ffmpeg`
- Grant microphone permission to WhisperBar when prompted on first recording
- Grant Screen Recording permission to WhisperBar/Terminal when prompted

## Run (Development)

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## First-run Dependency Bootstrap

On app startup, Rust triggers `src-tauri/python/bootstrap.py`.

Bootstrap behavior:
1. Checks for private venv in app data directory
2. Installs/updates Python dependencies:
   - `mlx-whisper`
   - `numpy`
   - `huggingface-hub`
3. Downloads selected model (default: `small`)
4. Reports progress back to UI via JSON lines
5. Sets app status to `Ready` when complete

Runtime data path (Tauri app data) contains:
- `python-env/` (venv)
- `models/whisper-*/` (model files)
- `python/bootstrap.py`, `python/worker.py` (runtime scripts copied from source)

If bootstrap fails, status becomes `Error` and the UI exposes `Retry Install`.

## Recording Flow

- `Start Recording`
  - Starts Python worker process
  - Ensures selected model is installed
  - Captures desktop audio via ScreenCaptureKit and microphone via ffmpeg
  - Opens floating always-on-top transcript window
  - Streams partial transcript lines to the UI
- `Stop Recording`
  - Signals worker to stop gracefully
  - Saves transcript to markdown when transcript text exists
  - Returns status to `Ready` (or `Error` with guidance if no speech was captured)

## Model Selection

- Available models:
  - Large v3 Turbo (`0.81 GB`)
  - Large v3 (`3.10 GB`)
- Changing model in the UI triggers download/install automatically when missing.

## Audio Device Selection

- Desktop audio is captured directly via ScreenCaptureKit.
- `Microphone Input` lets you choose which microphone to mix with desktop audio.
- Worker auto-selects a microphone when `Auto` is chosen.

Environment overrides:

```bash
# Force a specific microphone by name snippet, index, or avfoundation syntax
WHISPERBAR_MIC_DEVICE=\"MacBook Pro Microphone\" npm run tauri dev
WHISPERBAR_MIC_DEVICE=\"1\" npm run tauri dev
WHISPERBAR_MIC_DEVICE=\":1\" npm run tauri dev
```

## Output Location

Transcript files are saved to:

```text
~/Documents/WhisperBar/
```

Filename format:

```text
Transcript-YYYY-MM-DD-HH-mm.md
```

Example:

```text
Transcript-2026-02-27-14-35.md
```

Example markdown content:

```md
Hello everyone, thank you for joining.
We are reviewing the product milestones for March.
Vamos finalizar os detalhes de entrega até sexta-feira.
```

## Known Limitations (MVP)

- ScreenCaptureKit desktop audio requires macOS Screen Recording permission.
- On some macOS versions, ScreenCaptureKit desktop capture can include small timing jitter.
- Capturing both microphone and system audio simultaneously may require an Aggregate Device in Audio MIDI Setup.
- Worker currently transcribes fixed audio chunks; boundary artifacts and repeated lines can occur.
- Default transcription uses Apple Silicon MLX acceleration for low-latency inference.
- No speaker diarization or punctuation post-processing in MVP.
- Menu dropdown window placement is simple toggle behavior (not pixel-perfect anchored to tray icon on every display layout).

## Project Structure

```text
src/                 # React UI (tray panel + floating transcript)
src-tauri/src/
  app_state.rs       # state machine + shared state snapshot
  bootstrap.rs       # dependency bootstrap runner
  models.rs          # local model catalog + size metadata
  worker.rs          # python process manager + live event handling
  transcript_file.rs # markdown save logic
  tray.rs            # tray/menu icon setup
  ui.rs              # tray + floating window creation/toggling
src-tauri/python/
  bootstrap.py       # venv + package + model install
  worker.py          # ffmpeg capture + mlx-whisper streaming
```
