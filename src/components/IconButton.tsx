import type { ComponentPropsWithoutRef, ComponentType } from "react";

interface IconButtonProps extends Omit<
  ComponentPropsWithoutRef<"button">,
  "aria-label" | "children" | "className" | "type"
> {
  label: string;
  icon: ComponentType<{ size?: number; strokeWidth?: number }>;
  active?: boolean;
  size?: "small" | "medium";
}

export function IconButton({
  label,
  icon: Icon,
  active = false,
  size = "medium",
  ...buttonProps
}: IconButtonProps) {
  return (
    <button
      {...buttonProps}
      type="button"
      className={`icon-button icon-button--${size}${active ? " is-active" : ""}`}
      aria-label={label}
      title={label}
    >
      <Icon size={size === "small" ? 14 : 17} strokeWidth={1.7} />
    </button>
  );
}
