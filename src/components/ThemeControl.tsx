import { Moon, Sun } from "lucide-react";
import { IconButton } from "./IconButton";
import { useTheme } from "../design-system/theme";

export function ThemeControl() {
  const {
    state: { theme },
    actions: { setTheme },
  } = useTheme();
  const next = theme === "dark" ? "light" : "dark";
  return (
    <IconButton
      label={`Switch to ${next} mode`}
      icon={next === "light" ? Sun : Moon}
      size="small"
      onClick={() => setTheme(next)}
    />
  );
}
