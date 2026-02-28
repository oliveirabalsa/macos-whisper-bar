import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type AppStatus = "Idle" | "Installing" | "Ready" | "Recording" | "Error";
type Language = "en" | "pt-BR";

interface AppSnapshot {
  status: AppStatus;
  statusMessage: string;
  language: Language;
  selectedModelId: string;
  selectedModelInstalled: boolean;
  selectedMicDevice: string | null;
  transcript: string;
  lastSavedPath: string | null;
  installProgress: number | null;
  errorMessage: string | null;
}

interface ModelOption {
  id: string;
  name: string;
  sizeLabel: string;
}

interface AudioDeviceOption {
  id: string;
  name: string;
  isMicrophoneLike: boolean;
}

const INITIAL_STATE: AppSnapshot = {
  status: "Idle",
  statusMessage: "Idle",
  language: "en",
  selectedModelId: "large-v3-turbo",
  selectedModelInstalled: false,
  selectedMicDevice: null,
  transcript: "",
  lastSavedPath: null,
  installProgress: null,
  errorMessage: null
};

const FALLBACK_MODELS: ModelOption[] = [
  { id: "large-v3-turbo", name: "Large v3 Turbo", sizeLabel: "0.81 GB" },
  { id: "large-v3", name: "Large v3", sizeLabel: "3.10 GB" }
];

export function App() {
  const [state, setState] = useState<AppSnapshot>(INITIAL_STATE);
  const [windowLabel, setWindowLabel] = useState("tray");
  const [modelOptions, setModelOptions] = useState<ModelOption[]>(FALLBACK_MODELS);
  const [audioDevices, setAudioDevices] = useState<AudioDeviceOption[]>([]);
  const [recordingSeconds, setRecordingSeconds] = useState(0);

  const refreshAudioDevices = useCallback(async () => {
    try {
      const devices = await invoke<AudioDeviceOption[]>("refresh_audio_devices");
      setAudioDevices(devices);
    } catch {
      // Keep previous list on failure.
    }
  }, []);

  useEffect(() => {
    const current = getCurrentWindow();
    setWindowLabel(current.label);

    void invoke<AppSnapshot>("get_app_state").then(setState).catch(() => undefined);
    void invoke<ModelOption[]>("get_model_options").then(setModelOptions).catch(() => undefined);
    void refreshAudioDevices();

    const unlistenPromise = listen<AppSnapshot>("whisperbar://state", (event) => setState(event.payload));
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [refreshAudioDevices]);

  useEffect(() => {
    if (state.status !== "Recording") {
      setRecordingSeconds(0);
      return;
    }

    const startedAt = Date.now();
    setRecordingSeconds(0);
    const timer = window.setInterval(() => {
      setRecordingSeconds(Math.max(0, Math.floor((Date.now() - startedAt) / 1000)));
    }, 1000);

    return () => {
      window.clearInterval(timer);
    };
  }, [state.status]);

  if (windowLabel === "floating") {
    return <FloatingTranscript transcript={state.transcript} status={state.status} recordingSeconds={recordingSeconds} />;
  }

  return (
    <TrayPanel
      state={state}
      modelOptions={modelOptions}
      audioDevices={audioDevices}
      recordingSeconds={recordingSeconds}
    />
  );
}

function TrayPanel({
  state,
  modelOptions,
  audioDevices,
  recordingSeconds
}: {
  state: AppSnapshot;
  modelOptions: ModelOption[];
  audioDevices: AudioDeviceOption[];
  recordingSeconds: number;
}) {
  const [actionError, setActionError] = useState<string | null>(null);

  const currentError = state.errorMessage ?? actionError;
  const canStart = state.status === "Ready" && state.selectedModelInstalled;
  const canStop = state.status === "Recording";
  const recordingClock = useMemo(() => formatDuration(recordingSeconds), [recordingSeconds]);
  const statusDetail = useMemo(() => getStatusDetail(state), [state]);

  const selectedModel = useMemo(
    () => modelOptions.find((model) => model.id === state.selectedModelId) ?? null,
    [modelOptions, state.selectedModelId]
  );
  const micDevices = useMemo(() => audioDevices.filter((device) => device.isMicrophoneLike), [audioDevices]);
  const installProgressPercent =
    state.status !== "Installing" || state.installProgress === null
      ? null
      : Math.round(Math.max(0, Math.min(100, state.installProgress * 100)));

  const runCommand = useCallback(async (command: string, payload?: Record<string, unknown>) => {
    setActionError(null);
    try {
      await invoke(command, payload);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  return (
    <main className="tray-shell">
      <PanelHeader title="WhisperBar" status={state.status} recordingClock={canStop ? recordingClock : null} />

      <SessionCard detail={statusDetail} progressPercent={installProgressPercent} />

      <section className="control-grid">
        <SelectCard
          id="language"
          label="Language"
          value={state.language}
          disabled={state.status === "Recording"}
          onChange={(value) => void runCommand("set_language", { language: value as Language })}
          options={[
            { value: "en", label: "English (en)" },
            { value: "pt-BR", label: "Portuguese (Brazil) (pt-BR)" }
          ]}
        />

        <SelectCard
          id="model"
          label="Model"
          value={state.selectedModelId}
          disabled={state.status === "Recording" || state.status === "Installing"}
          onChange={(value) => void runCommand("set_model", { modelId: value })}
          options={modelOptions.map((model) => ({ value: model.id, label: `${model.name} (${model.sizeLabel})` }))}
          footer={selectedModel ? `${selectedModel.name} selected` : state.selectedModelId}
        />
        {!state.selectedModelInstalled ? (
          <button
            className="btn btn-muted"
            disabled={state.status === "Installing" || state.status === "Recording"}
            onClick={() => void runCommand("install_selected_model")}
          >
            Install Model
          </button>
        ) : null}
      </section>

      <section className="block card">
        <SelectCard
          id="mic-device"
          label="Microphone Input"
          value={state.selectedMicDevice ?? ""}
          disabled={state.status === "Recording" || state.status === "Installing"}
          onChange={(value) =>
            void runCommand("set_audio_inputs", {
              micDevice: value || null
            })
          }
          options={[{ value: "", label: "Auto" }, ...micDevices.map((device) => ({ value: device.id, label: device.name }))]}
          compact
        />

        <label htmlFor="system-device">Desktop Audio Input</label>
        <p className="subtle">ScreenCaptureKit</p>
      </section>

      <RecordControlButton canStart={canStart} canStop={canStop} onStart={() => void runCommand("start_recording")} onStop={() => void runCommand("stop_recording")} />

      {currentError ? (
        <section className="block card error-box">
          <p>{currentError}</p>
          <button
            className="btn btn-muted"
            onClick={() => void runCommand((currentError ?? "").toLowerCase().includes("install") ? "retry_bootstrap" : "clear_error")}
          >
            {(currentError ?? "").toLowerCase().includes("install") ? "Retry Install" : "Dismiss Error"}
          </button>
        </section>
      ) : null}

      {state.lastSavedPath ? <p className="saved-path">Saved: {state.lastSavedPath}</p> : null}
    </main>
  );
}

function FloatingTranscript({ transcript, status, recordingSeconds }: { transcript: string; status: AppStatus; recordingSeconds: number }) {
  const preview = transcript.trim().length > 0 ? transcript : "Listening... transcript will appear here.";
  const canStop = status === "Recording";
  const [actionError, setActionError] = useState<string | null>(null);

  const stopFromFloating = async () => {
    setActionError(null);
    try {
      await invoke("stop_recording");
    } catch (error) {
      setActionError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <main className="floating-shell">
      <PanelHeader title="Live Transcript" status={status} recordingClock={canStop ? formatDuration(recordingSeconds) : null} />
      <section className="transcript-body">{preview}</section>
      {canStop ? (
        <div className="floating-actions">
          <button className="btn btn-stop floating-stop" onClick={stopFromFloating}>
            Stop Recording
          </button>
        </div>
      ) : null}
      {actionError ? <p className="subtle">{actionError}</p> : null}
    </main>
  );
}

function PanelHeader({
  title,
  status,
  recordingClock
}: {
  title: string;
  status: AppStatus;
  recordingClock: string | null;
}) {
  return (
    <header className="panel-header">
      <div className="title-wrap">
        <h1>{title}</h1>
      </div>
      <div className="header-meta">
        {recordingClock ? <p className="recording-clock">{recordingClock}</p> : null}
        <span className={`status-pill status-${status.toLowerCase()}`}>{status}</span>
      </div>
    </header>
  );
}

function SessionCard({ detail, progressPercent }: { detail: string; progressPercent: number | null }) {
  return (
    <section className="block card">
      <div className="status-row">
        <p className="status-title">Session</p>
        {progressPercent !== null ? <p className="status-percent">{progressPercent}%</p> : null}
      </div>
      <p className="status-message">{detail}</p>
      {progressPercent !== null ? (
        <div className="progress-track installing" role="progressbar" aria-valuenow={progressPercent}>
          <div className="progress-value" style={{ width: `${Math.max(8, progressPercent)}%` }} />
        </div>
      ) : null}
    </section>
  );
}

function SelectCard({
  id,
  label,
  value,
  disabled,
  options,
  onChange,
  footer,
  compact
}: {
  id: string;
  label: string;
  value: string;
  disabled: boolean;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
  footer?: string;
  compact?: boolean;
}) {
  return (
    <section className={compact ? "block" : "block card"}>
      <label htmlFor={id}>{label}</label>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)} disabled={disabled}>
        {options.map((option) => (
          <option key={`${id}-${option.value}`} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      {footer ? <p className="subtle">{footer}</p> : null}
    </section>
  );
}

function RecordControlButton({
  canStart,
  canStop,
  onStart,
  onStop
}: {
  canStart: boolean;
  canStop: boolean;
  onStart: () => void;
  onStop: () => void;
}) {
  return (
    <div className="row actions">
      {canStop ? (
        <button className="btn btn-stop primary-action" onClick={onStop}>
          Stop Recording
        </button>
      ) : (
        <button className="btn btn-start primary-action" disabled={!canStart} onClick={onStart}>
          Start Recording
        </button>
      )}
    </div>
  );
}

function getStatusDetail(state: AppSnapshot): string {
  const message = state.statusMessage.trim();
  const normalizedMessage = message.toLowerCase();
  const normalizedStatus = state.status.toLowerCase();

  if (state.status === "Ready") {
    if (message && normalizedMessage !== normalizedStatus && normalizedMessage !== "ready") {
      return message;
    }
    return "Ready to start a recording.";
  }

  if (state.status === "Idle") {
    return "Initializing app environment.";
  }

  if (state.status === "Recording") {
    if (!message || normalizedMessage === normalizedStatus || normalizedMessage === "recording") {
      return "Listening and transcribing in near real-time.";
    }
    return message;
  }

  if (state.status === "Installing") {
    if (!message || normalizedMessage === normalizedStatus || normalizedMessage.includes("ready")) {
      return "Installing dependencies and preparing transcription model.";
    }
    return message;
  }

  return message || "An error occurred.";
}

function formatDuration(totalSeconds: number): string {
  const safe = Math.max(0, totalSeconds);
  const hours = Math.floor(safe / 3600);
  const minutes = Math.floor((safe % 3600) / 60);
  const seconds = safe % 60;

  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}
