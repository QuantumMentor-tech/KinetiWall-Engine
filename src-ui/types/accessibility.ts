/**
 * Accessibility & Inclusive Interaction Domain Types (Step 63).
 */

import type { ReducedMotionPreferenceDto } from './settings.ts';

export interface AccessibilityState {
  /** User preference intent stored in configuration: 'system' | 'always_reduce' | 'no_preference' */
  reducedMotionPreference: ReducedMotionPreferenceDto;
  /** Whether the underlying operating system currently requests reduced motion */
  isSystemReducedMotion: boolean;
  /** Computed effective reduced motion decision governing UI animations */
  effectiveReducedMotion: boolean;
  /** Whether the runtime environment supports querying system motion preferences */
  isSystemSupported: boolean;
}

/**
 * Resolves effective reduced motion behavior deterministically.
 * - 'always_reduce': forces reduced motion regardless of OS setting
 * - 'no_preference': enables normal motion regardless of OS setting
 * - 'system': synchronizes with OS prefers-reduced-motion media query
 */
export function resolveEffectiveReducedMotion(
  preference: ReducedMotionPreferenceDto,
  isSystemReducedMotion: boolean,
): boolean {
  switch (preference) {
    case 'always_reduce':
      return true;
    case 'no_preference':
      return false;
    case 'system':
    default:
      return isSystemReducedMotion;
  }
}

