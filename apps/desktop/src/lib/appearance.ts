import { useEffect, useState } from "react";

export type Appearance = "system" | "dark" | "light";

export function useAppearance() {
  const [appearance, setChoice] = useState<Appearance>(() => {
    const saved = localStorage.getItem("autono-theme");
    return saved === "light" || saved === "system" ? saved : "dark";
  });
  const [texture, setTextureChoice] = useState(
    () => localStorage.getItem("autono-texture") !== "off",
  );
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        appearance === "system"
          ? media.matches
            ? "dark"
            : "light"
          : appearance;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [appearance]);
  useEffect(() => {
    document.documentElement.dataset.texture = texture ? "on" : "off";
  }, [texture]);
  return {
    appearance,
    texture,
    setAppearance(value: Appearance) {
      localStorage.setItem("autono-theme", value);
      setChoice(value);
    },
    setTexture(value: boolean) {
      localStorage.setItem("autono-texture", value ? "on" : "off");
      setTextureChoice(value);
    },
  };
}
export type AppearanceSettings = ReturnType<typeof useAppearance>;
