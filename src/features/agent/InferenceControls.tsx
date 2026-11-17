import { useId } from "react";
import type {
  AgentSettings,
  HardwareProfile,
  InferenceBackend,
} from "../../services/contracts";

import { contextLevels, contextTokens } from "./inferenceSettings";

/** Configure local answer generation with native, keyboard-accessible controls. */
export function InferenceControls({
  settings,
  hardware,
  capacity,
  onSettings,
}: {
  settings: AgentSettings;
  hardware: HardwareProfile;
  capacity: number;
  onSettings: (settings: AgentSettings) => void;
}) {
  const id = useId();
  const level = settings.contextWindow ?? "medium";
  const selected = contextLevels.findIndex((entry) => entry.value === level);
  const tokens = contextTokens(level, capacity);
  const gpuAvailable = Boolean(hardware.gpuDevices?.length);
  const backend = settings.inferenceBackend ?? "auto";
  const choices: { value: InferenceBackend; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "cpu", label: "CPU" },
    { value: "gpu", label: "GPU" },
  ];
  return (
    <div className="agent-inference-controls">
      <fieldset
        className="agent-backend-setting"
        aria-describedby={`${id}-backend-help`}
      >
        <legend>Inference backend</legend>
        <div className="agent-backend-options">
          {choices.map((choice) => (
            <label key={choice.value}>
              <input
                type="radio"
                name={`${id}-backend`}
                value={choice.value}
                checked={backend === choice.value}
                disabled={choice.value === "gpu" && !gpuAvailable}
                onChange={() =>
                  onSettings({ ...settings, inferenceBackend: choice.value })
                }
              />
              <span>{choice.label}</span>
            </label>
          ))}
        </div>
      </fieldset>
      <p id={`${id}-backend-help`}>
        {backend === "cpu"
          ? "CPU uses system memory for answers. The next question uses your selection."
          : backend === "gpu"
            ? "GPU accelerates answers. If it cannot run the model, Graphite asks you to switch."
            : "Auto prefers an available GPU and falls back to CPU if needed."}
        {!gpuAvailable &&
          (hardware.compiledBackend === "CPU"
            ? " GPU is unavailable in this CPU-only build. A GPU-enabled build is required."
            : " No compatible inference GPU is available in this session.")}
      </p>
      <div className="agent-context-heading">
        <label htmlFor={`${id}-context`}>Context window</label>
        <output htmlFor={`${id}-context`}>
          {tokens.toLocaleString()} tokens
        </output>
      </div>
      <input
        className="agent-context-slider"
        id={`${id}-context`}
        type="range"
        min={0}
        max={3}
        step={1}
        value={selected}
        aria-valuetext={`${contextLevels[selected].label}, ${tokens.toLocaleString()} tokens`}
        aria-describedby={`${id}-context-help`}
        onChange={(event) =>
          onSettings({
            ...settings,
            contextWindow: contextLevels[Number(event.target.value)].value,
          })
        }
      />
      <div className="agent-context-stops">
        {contextLevels.map((entry) => (
          <button
            key={entry.value}
            type="button"
            className={entry.value === level ? "is-active" : ""}
            aria-pressed={entry.value === level}
            onClick={() =>
              onSettings({ ...settings, contextWindow: entry.value })
            }
          >
            {entry.label}
          </button>
        ))}
      </div>
      <p id={`${id}-context-help`}>
        Room for the question, retrieved passages, and answer. Medium is the
        default. Higher levels can fit more evidence but use more memory and may
        be slower; they do not guarantee better answers. Limits follow the
        selected model. Answer detail controls response length separately.
      </p>
    </div>
  );
}
