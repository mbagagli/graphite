import darkLogoUrl from "../../graphics_theme/Dark_Theme/logo/graphite_logo.svg";
import lightLogoUrl from "../../graphics_theme/Light_Theme/logo/graphite_logo.svg";
import { useTheme } from "../design-system/theme";

export function BrandMark() {
  const {
    state: { theme },
  } = useTheme();
  const logoUrl = theme === "dark" ? darkLogoUrl : lightLogoUrl;

  return (
    <img
      className="brand-mark"
      src={logoUrl}
      width={38}
      height={38}
      alt=""
      aria-hidden="true"
    />
  );
}
