export type LanguagePreference = "system" | "zh" | "en";
export type ThemePreference = "system" | "light" | "dark";

export function readPreference<Value extends string>(key: string, choices: readonly Value[], fallback: Value): Value {
  try {
    const value = localStorage.getItem(key);
    return choices.includes(value as Value) ? value as Value : fallback;
  } catch { return fallback; }
}

export function savePreference(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch {}
}

export function resolveLanguage(preference: LanguagePreference, languages: readonly string[]): "zh" | "en" {
  if (preference !== "system") return preference;
  for (const language of languages) {
    if (/^zh(?:-|$)/i.test(language)) return "zh";
    if (/^en(?:-|$)/i.test(language)) return "en";
  }
  return "en";
}

export function resolveTheme(preference: ThemePreference, dark: boolean): "light" | "dark" {
  return preference === "system" ? dark ? "dark" : "light" : preference;
}

export const languageKey = "lightos-desktop-language";
export const themeKey = "lightos-desktop-theme";
export const languages = ["system", "zh", "en"] as const;
export const themes = ["system", "light", "dark"] as const;

export const videoCodecKey = "lightos-desktop-video-codec";
export const streamStrategyKey = "lightos-desktop-stream-strategy";
export const videoCodecs = ["auto", "h264", "jpeg"] as const;
export const streamStrategies = ["auto", "low_latency", "smooth", "quality_first"] as const;
