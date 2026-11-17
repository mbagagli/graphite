import type { ContextWindow } from "../../services/contracts";

export const contextLevels = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "extra-high", label: "Extra-High" },
] as const;

/** Resolve the same bounded profile fractions as the native engine. */
export function contextTokens(level: ContextWindow, capacity: number): number {
  const quarter = contextLevels.findIndex((entry) => entry.value === level) + 1;
  return Math.min(capacity, Math.max(2048, Math.floor(capacity / 4) * quarter));
}
