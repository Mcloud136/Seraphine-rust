import {
  createDarkTheme,
  createLightTheme,
  webLightTheme,
  type BrandVariants,
  type Theme,
} from "@fluentui/react-components";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { AppConfig } from "./config";

type Rgb = [number, number, number];

function parseColor(value: string): Rgb | null {
  const raw = value.trim().replace(/^#/, "");
  const hex = raw.length === 8 ? raw.slice(2) : raw;
  if (!/^[0-9a-fA-F]{6}$/.test(hex)) return null;
  return [
    Number.parseInt(hex.slice(0, 2), 16),
    Number.parseInt(hex.slice(2, 4), 16),
    Number.parseInt(hex.slice(4, 6), 16),
  ];
}

function mix(from: Rgb, to: Rgb, weight: number): string {
  const channels = from.map((channel, index) =>
    Math.round(channel + (to[index] - channel) * weight),
  );
  return `#${channels
    .map((channel) => channel.toString(16).padStart(2, "0"))
    .join("")}`;
}

export function brandVariants(color: string): BrandVariants {
  const base = parseColor(color) ?? [0, 159, 170];
  const black: Rgb = [0, 0, 0];
  const white: Rgb = [255, 255, 255];
  return {
    10: mix(base, black, 0.85),
    20: mix(base, black, 0.7),
    30: mix(base, black, 0.55),
    40: mix(base, black, 0.4),
    50: mix(base, black, 0.25),
    60: mix(base, black, 0.12),
    70: mix(base, black, 0),
    80: mix(base, white, 0.12),
    90: mix(base, white, 0.24),
    100: mix(base, white, 0.36),
    110: mix(base, white, 0.48),
    120: mix(base, white, 0.6),
    130: mix(base, white, 0.7),
    140: mix(base, white, 0.8),
    150: mix(base, white, 0.9),
    160: mix(base, white, 0.95),
  };
}

export function buildTheme(config: AppConfig | null): Theme {
  if (!config) return webLightTheme;
  const dark = config.theme.themeMode.toLowerCase() === "dark";
  const base = dark
    ? createDarkTheme(brandVariants(config.theme.themeColor))
    : createLightTheme(brandVariants(config.theme.themeColor));
  const fonts = config.theme.fontFamilies
    .map((font) => font.trim())
    .filter(Boolean);
  return fonts.length ? { ...base, fontFamilyBase: fonts.join(", ") } : base;
}

export function zoomFor(value: string | undefined): number {
  if (!value || value.toLowerCase() === "auto") return 1;
  const zoom = Number(value);
  return Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
}

export async function applyZoom(value: string | undefined): Promise<void> {
  try {
    await getCurrentWebview().setZoom(zoomFor(value));
  } catch {
    // Zoom is cosmetic; ignore failures on platforms without support.
  }
}