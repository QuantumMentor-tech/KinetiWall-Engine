/**
 * KinetiWall Approved Design System Tokens (Neon Glass — Steps 20 & 21)
 * Single authoritative token definition for surfaces, semantic states, spacing, radii, and responsive breakpoints.
 */

export const DARK_PALETTE = {
  surfaceVoid: '#0A0B10',
  surfaceAcrylic: '#12141F',
  surfaceElevated: '#1A1D2E',
  surfaceInput: '#0F111A',
  borderSubtle: '#1F2338',
  borderDefault: '#2A2E45',
  borderFocus: '#00F0FF',
  accentCyan: '#00F0FF',
  accentMagenta: '#FF0055',
  statusAvailable: '#00E5A3',
  statusWarning: '#FFB800',
  statusError: '#FF0055',
  statusNeutral: '#8E96B8',
  textPrimary: '#F4F6FF',
  textSecondary: '#A8B1D1',
  textMuted: '#7D86A8',
  textOnCyan: '#0A0B10',
} as const;

export const LIGHT_PALETTE = {
  surfaceVoid: '#F0F2F8',
  surfaceAcrylic: '#FFFFFF',
  surfaceElevated: '#E6E9F2',
  surfaceInput: '#F8F9FD',
  borderSubtle: '#D8DCEB',
  borderDefault: '#C4CADF',
  borderFocus: '#007A8A',
  accentCyan: '#007A8A',
  accentMagenta: '#D60045',
  statusAvailable: '#008A5E',
  statusWarning: '#9E5E00',
  statusError: '#D60045',
  statusNeutral: '#5E6A8A',
  textPrimary: '#0D111C',
  textSecondary: '#384260',
  textMuted: '#5E6A8A',
  textOnCyan: '#FFFFFF',
} as const;

export const NEON_GLASS_TOKENS = {
  colors: DARK_PALETTE,
  dark: DARK_PALETTE,
  light: LIGHT_PALETTE,
  radii: {
    sm: '6px',
    md: '8px',
    lg: '12px',
    full: '9999px',
  },
  spacing: {
    xs: '4px',
    sm: '8px',
    md: '12px',
    lg: '16px',
    xl: '24px',
    xxl: '32px',
  },
  motion: {
    fast: '120ms cubic-bezier(0.2, 0, 0, 1)',
    normal: '180ms cubic-bezier(0.2, 0, 0, 1)',
    reducedMotionMediaQuery: '@media (prefers-reduced-motion: reduce)',
  },
  breakpoints: {
    minWindowWidth: 960,
    minWindowHeight: 600,
    compactMax: 1199,
    standardMin: 1200,
    wideMin: 1680,
    ultraWideMin: 2400,
    uhd4kMin: 3200,
  },
} as const;

export type ResponsiveTier = 'compact' | 'standard' | 'wide' | 'ultrawide' | 'uhd4k';

export interface ResponsiveGridMetrics {
  tier: ResponsiveTier;
  columns: number;
  gapPx: number;
  cardAspectRatio: '16 / 9';
  minCardWidthPx: number;
}

/**
 * Computes responsive grid layout metrics for supported desktop window widths (Step 22 & Step 27 §6.1, §10).
 */
export function computeResponsiveGridMetrics(viewportWidthPx: number): ResponsiveGridMetrics {
  const clampedWidth = Math.max(NEON_GLASS_TOKENS.breakpoints.minWindowWidth, viewportWidthPx);

  if (clampedWidth >= NEON_GLASS_TOKENS.breakpoints.uhd4kMin) {
    return {
      tier: 'uhd4k',
      columns: 6,
      gapPx: 24,
      cardAspectRatio: '16 / 9',
      minCardWidthPx: 260,
    };
  }
  if (clampedWidth >= NEON_GLASS_TOKENS.breakpoints.ultraWideMin) {
    return {
      tier: 'ultrawide',
      columns: 5,
      gapPx: 20,
      cardAspectRatio: '16 / 9',
      minCardWidthPx: 250,
    };
  }
  if (clampedWidth >= NEON_GLASS_TOKENS.breakpoints.wideMin) {
    return {
      tier: 'wide',
      columns: 4,
      gapPx: 20,
      cardAspectRatio: '16 / 9',
      minCardWidthPx: 240,
    };
  }
  if (clampedWidth >= NEON_GLASS_TOKENS.breakpoints.standardMin) {
    return {
      tier: 'standard',
      columns: 3,
      gapPx: 16,
      cardAspectRatio: '16 / 9',
      minCardWidthPx: 230,
    };
  }
  return {
    tier: 'compact',
    columns: 2,
    gapPx: 16,
    cardAspectRatio: '16 / 9',
    minCardWidthPx: 220,
  };
}

