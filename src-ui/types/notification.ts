/**
 * Authoritative Notification & In-App Feedback Types (Step 65).
 */

import type { NotificationPreferenceDto } from './settings.ts';

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error';

export interface NotificationAction {
  label: string;
  onClick: () => void | Promise<void>;
  commandId?: string;
}

export interface NotificationRequest {
  id?: string;
  severity: NotificationSeverity;
  title?: string;
  message: string;
  reasonCode?: string;
  durationMs?: number; // null/undefined uses severity default; 0 means persistent
  dedupKey?: string;
  action?: NotificationAction;
  dismissible?: boolean;
}

export interface NotificationItem {
  id: string;
  severity: NotificationSeverity;
  title?: string;
  message: string;
  reasonCode?: string;
  timestamp: number;
  expiresAt: number | null; // null means persistent (no auto-expiry)
  durationMs: number;
  dedupKey?: string;
  action?: NotificationAction;
  dismissible: boolean;
}

export const DEFAULT_NOTIFICATION_DURATIONS: Record<NotificationSeverity, number | null> = {
  info: 4000,
  success: 3500,
  warning: 5000,
  error: null, // Persistent by default for error accountability
};

export const MAX_VISIBLE_NOTIFICATIONS = 4;
export const DEFAULT_DEDUP_WINDOW_MS = 1000;

/**
 * Sanitizes user-facing notification messages to prevent leaking raw Rust stack traces,
 * database internals, or absolute filesystem paths.
 */
export function sanitizeNotificationMessage(message: string): string {
  if (!message) return '';
  let sanitized = message.trim();

  // Redact absolute Windows/UNC file paths (e.g. C:\Users\... or E:\Projects\...)
  sanitized = sanitized.replace(/[A-Za-z]:\\[^"'\s\n\r<>]+/g, (match) => {
    const parts = match.split('\\');
    return `[path]/${parts[parts.length - 1]}`;
  });

  // Strip raw Rust debug Error representations
  sanitized = sanitized.replace(/Custom\s*\{\s*kind:\s*[^,]+,\s*error:\s*([^}]+)\}/g, '$1').trim();

  return sanitized;
}

/**
 * Determines whether a notification should be presented based on the user's notification preference.
 * - 'all': shows all notifications
 * - 'errors_only': suppresses info and success toasts; preserves warnings and errors
 * - 'disabled': suppresses non-critical transient toasts; preserves critical errors
 */
export function shouldPresentNotification(
  severity: NotificationSeverity,
  preference: NotificationPreferenceDto = 'all',
): boolean {
  switch (preference) {
    case 'disabled':
      return severity === 'error';
    case 'errors_only':
      return severity === 'error' || severity === 'warning';
    case 'all':
    default:
      return true;
  }
}
