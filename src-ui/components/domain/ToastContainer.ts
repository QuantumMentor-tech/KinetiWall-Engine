/**
 * In-App Toast & Notification Viewport Component (Step 65).
 * Renders stacked, accessible toasts with visual severity icons, action controls, and dismiss buttons.
 */

import { Button } from '../primitives/controls.ts';
import { h, type UiNode } from '../primitives/uiNode.ts';
import type { NotificationItem, NotificationSeverity } from '../../types/notification.ts';

export interface ToastContainerProps {
  notifications: readonly NotificationItem[];
  onDismiss: (id: string) => void;
}

const SEVERITY_ICONS: Record<NotificationSeverity, string> = {
  success: '✓',
  info: 'ℹ',
  warning: '⚠',
  error: '✕',
};

export function ToastContainer(props: ToastContainerProps): UiNode | null {
  const { notifications, onDismiss } = props;

  if (!notifications || notifications.length === 0) {
    return null;
  }

  return h(
    'div',
    {
      attrs: {
        role: 'region',
        'aria-label': 'Notifications',
        'data-testid': 'toast-container',
        class: 'kw-toast-container',
      },
    },
    ...notifications.map((item) => {
      const isUrgent = item.severity === 'error' || item.severity === 'warning';
      const role = isUrgent ? 'alert' : 'status';

      return h(
        'div',
        {
          key: item.id,
          attrs: {
            id: `toast-${item.id}`,
            role,
            'data-testid': `toast-${item.id}`,
            'data-severity': item.severity,
            class: `kw-toast kw-toast-${item.severity}`,
          },
        },
        // Icon
        h(
          'span',
          {
            attrs: {
              'aria-hidden': 'true',
              class: `kw-toast-icon kw-toast-icon-${item.severity}`,
            },
          },
          SEVERITY_ICONS[item.severity],
        ),
        // Content
        h(
          'div',
          { attrs: { class: 'kw-toast-content' } },
          item.title
            ? h('strong', { attrs: { class: 'kw-toast-title' } }, item.title)
            : null,
          h('p', { attrs: { class: 'kw-toast-message' } }, item.message),
        ),
        // Actions
        h(
          'div',
          { attrs: { class: 'kw-toast-actions' } },
          item.action
            ? Button({
                testId: `toast-action-${item.id}`,
                label: item.action.label,
                variant: 'primary',
                size: 'sm',
                onClick: () => {
                  void item.action?.onClick();
                  onDismiss(item.id);
                },
              })
            : null,
          item.dismissible
            ? Button({
                testId: `toast-dismiss-${item.id}`,
                label: '✕',
                ariaLabel: 'Dismiss notification',
                variant: 'ghost',
                size: 'sm',
                onClick: () => onDismiss(item.id),
              })
            : null,
        ),
      );
    }),
  );
}

