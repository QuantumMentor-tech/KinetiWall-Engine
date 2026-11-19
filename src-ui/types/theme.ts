/**
 * Theme & Appearance Preferences Types & Resolution Utilities (Step 62).
 * Adheres to WCAG 2.1 AA accessibility standards and KinetiWall Neon Glass aesthetic.
 */

import type { ThemePreferenceDto } from './settings.ts';

export type StoredThemePreference = ThemePreferenceDto; // 'system' | 'dark' | 'light'

export type EffectiveTheme = 'dark' | 'light';

export interface ThemeState {
  /** The user's persisted preference intent ('system', 'dark', or 'light'). */
  preference: StoredThemePreference;
  /** The currently resolved visual theme applied to the UI ('dark' or 'light'). */
  effectiveTheme: EffectiveTheme;
  /** The current operating system appearance observation ('dark' or 'light'). */
  systemTheme: EffectiveTheme;
  /** Whether operating system preference detection is supported on the current platform. */
  isSystemSupported: boolean;
}

/**
 * Deterministically resolves the effective visual theme.
 *
 * Rules:
 * - If preference is 'light' -> always 'light'.
 * - If preference is 'dark' -> always 'dark'.
 * - If preference is 'system' -> follows systemTheme observation, falling back to 'dark'.
 */
export function resolveEffectiveTheme(
  preference: StoredThemePreference,
  systemTheme: EffectiveTheme = 'dark',
): EffectiveTheme {
  switch (preference) {
    case 'light':
      return 'light';
    case 'dark':
      return 'dark';
    case 'system':
      return systemTheme === 'light' ? 'light' : 'dark';
    default:
      return 'dark';
  }
}

/**
 * Calculates the relative luminance of an sRGB color per WCAG 2.1 specifications.
 */
export function calculateRelativeLuminance(hexColor: string): number {
  const cleanHex = hexColor.replace('#', '');
  const r = parseInt(cleanHex.substring(0, 2), 16) / 255;
  const g = parseInt(cleanHex.substring(2, 4), 16) / 255;
  const b = parseInt(cleanHex.substring(4, 6), 16) / 255;

  const srgbToLinear = (c: number): number =>
    c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);

  const lr = srgbToLinear(r);
  const lg = srgbToLinear(g);
  const lb = srgbToLinear(b);

  return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

/**
 * Calculates the WCAG 2.1 contrast ratio between two hex colors.
 * Returns a value between 1.0 (no contrast) and 21.0 (black vs white).
 */
export function calculateContrastRatio(hex1: string, hex2: string): number {
  const lum1 = calculateRelativeLuminance(hex1);
  const lum2 = calculateRelativeLuminance(hex2);

  const brighter = Math.max(lum1, lum2);
  const darker = Math.min(lum1, lum2);

  return (brighter + 0.05) / (darker + 0.05);
}

