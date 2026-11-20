/**
 * Authoritative Notification Store & Lifecycle Manager (Step 65).
 * Coordinates toast presentation, auto-expiry timers, deduplication, and accessibility announcements.
 */

import type { KinetiWallConfigDto, NotificationPreferenceDto } from '../types/settings.ts';
import {
  DEFAULT_DEDUP_WINDOW_MS,
  DEFAULT_NOTIFICATION_DURATIONS,
  MAX_VISIBLE_NOTIFICATIONS,
  sanitizeNotificationMessage,
  shouldPresentNotification,
  type NotificationItem,
  type NotificationRequest,
  type NotificationSeverity,
} from '../types/notification.ts';
import { announce } from '../utils/announcer.ts';

export type NotificationListener = (notifications: NotificationItem[]) => void;

let globalNotificationCounter = 0;

export class NotificationStore {
  private notifications: NotificationItem[] = [];
  private preference: NotificationPreferenceDto = 'all';
  private readonly listeners = new Set<NotificationListener>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly dedupHistory = new Map<string, { timestamp: number; id: string }>();

  constructor(initialPreference: NotificationPreferenceDto = 'all') {
    this.preference = initialPreference;
  }

  public getState(): readonly NotificationItem[] {
    return this.notifications;
  }

  public getPreference(): NotificationPreferenceDto {
    return this.preference;
  }

  public setPreference(preference: NotificationPreferenceDto): void {
    if (this.preference === preference) return;
    this.preference = preference;
  }

  public syncWithSettings(config: KinetiWallConfigDto): void {
    if (config?.general?.notifications) {
      this.setPreference(config.general.notifications);
    }
  }

  public subscribe(listener: NotificationListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notifySubscribers(): void {
    const snapshot = [...this.notifications];
    for (const listener of this.listeners) {
      try {
        listener(snapshot);
      } catch (err) {
        console.error('[NotificationStore] Listener notification error:', err);
      }
    }
  }

  /**
   * Dispatches an in-app notification. Returns the notification ID, or null if filtered.
   */
  public notify(request: NotificationRequest): string | null {
    // 1. Check user notification preference
    if (!shouldPresentNotification(request.severity, this.preference)) {
      return null;
    }

    const now = Date.now();
    const sanitizedMsg = sanitizeNotificationMessage(request.message);
    if (!sanitizedMsg) {
      return null;
    }

    // 2. Deduplication check
    const dedupKey = request.dedupKey ?? `${request.severity}:${request.reasonCode ?? ''}:${sanitizedMsg}`;
    const recent = this.dedupHistory.get(dedupKey);
    if (recent && now - recent.timestamp < DEFAULT_DEDUP_WINDOW_MS) {
      // Rapid identical notification within deduplication window: update timestamp and return
      recent.timestamp = now;
      return recent.id;
    }

    const id = request.id ?? `notif_${now}_${++globalNotificationCounter}`;
    this.dedupHistory.set(dedupKey, { timestamp: now, id });

    // 3. Resolve duration and auto-dismissal
    let durationMs: number;
    if (typeof request.durationMs === 'number') {
      durationMs = request.durationMs <= 0 ? 0 : request.durationMs;
    } else {
      const defaultDuration = DEFAULT_NOTIFICATION_DURATIONS[request.severity];
      durationMs = defaultDuration ?? 0;
    }

    const expiresAt = durationMs > 0 ? now + durationMs : null;

    const item: NotificationItem = {
      id,
      severity: request.severity,
      title: request.title,
      message: sanitizedMsg,
      reasonCode: request.reasonCode,
      timestamp: now,
      expiresAt,
      durationMs,
      dedupKey,
      action: request.action,
      dismissible: request.dismissible ?? true,
    };

    // 4. Bounded queue management: enforce MAX_VISIBLE_NOTIFICATIONS
    if (this.notifications.length >= MAX_VISIBLE_NOTIFICATIONS) {
      // Remove oldest transient notification first, or oldest if all persistent
      const transientIdx = this.notifications.findIndex((n) => n.expiresAt !== null);
      const removeIdx = transientIdx >= 0 ? transientIdx : 0;
      const removed = this.notifications[removeIdx];
      this.clearTimer(removed.id);
      this.notifications.splice(removeIdx, 1);
    }

    this.notifications.push(item);

    // 5. Schedule auto-expiry timer if transient
    if (durationMs > 0) {
      const timer = setTimeout(() => {
        this.dismiss(id);
      }, durationMs);
      this.timers.set(id, timer);
    }

    // 6. Accessible screen-reader live announcement
    const politeness = request.severity === 'error' || request.severity === 'warning' ? 'assertive' : 'polite';
    announce(item.title ? `${item.title}: ${sanitizedMsg}` : sanitizedMsg, politeness);

    this.notifySubscribers();
    return id;
  }

  public info(message: string, options?: Partial<NotificationRequest>): string | null {
    return this.notify({ ...options, severity: 'info', message });
  }

  public success(message: string, options?: Partial<NotificationRequest>): string | null {
    return this.notify({ ...options, severity: 'success', message });
  }

  public warning(message: string, options?: Partial<NotificationRequest>): string | null {
    return this.notify({ ...options, severity: 'warning', message });
  }

  public error(message: string, options?: Partial<NotificationRequest>): string | null {
    return this.notify({ ...options, severity: 'error', message });
  }

  /**
   * Dismisses a notification by ID.
   */
  public dismiss(id: string): boolean {
    const idx = this.notifications.findIndex((n) => n.id === id);
    if (idx === -1) {
      return false;
    }

    this.clearTimer(id);
    this.notifications.splice(idx, 1);
    this.notifySubscribers();
    return true;
  }

  public dismissAll(): void {
    for (const n of this.notifications) {
      this.clearTimer(n.id);
    }
    this.notifications = [];
    this.notifySubscribers();
  }

  private clearTimer(id: string): void {
    const timer = this.timers.get(id);
    if (timer) {
      clearTimeout(timer);
      this.timers.delete(id);
    }
  }

  public dispose(): void {
    for (const timer of this.timers.values()) {
      clearTimeout(timer);
    }
    this.timers.clear();
    this.dedupHistory.clear();
    this.listeners.clear();
    this.notifications = [];
  }
}

