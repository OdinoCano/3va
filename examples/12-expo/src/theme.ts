// 12 - Expo: theme tokens para una app Expo (React Native).
// Mismo patrón que AppRegistry registerComponent — tokens de diseño
// que una app Expo comparte entre la app nativa, web y herramientas.

export const Color = {
  light: {
    background: "#ffffff",
    surface: "#f5f5f5",
    text: "#1a1a1a",
    muted: "#666666",
    primary: "#007AFF",
    danger: "#FF3B30",
    success: "#34C759",
    border: "#e0e0e0",
  },
  dark: {
    background: "#000000",
    surface: "#1c1c1e",
    text: "#ffffff",
    muted: "#999999",
    primary: "#0A84FF",
    danger: "#FF453A",
    success: "#30D158",
    border: "#38383a",
  },
} as const;

export type ColorToken = keyof typeof Color.light;

export function themeColor(color: ColorToken, darkMode: boolean): string {
  return darkMode ? Color.dark[color] : Color.light[color];
}

export const Spacing = { xs: 4, sm: 8, md: 16, lg: 24, xl: 32 } as const;
export const Radius = { sm: 6, md: 12, lg: 20, full: 9999 } as const;
export const FontSize = { caption: 12, body: 16, h3: 20, h2: 24, h1: 32 } as const;