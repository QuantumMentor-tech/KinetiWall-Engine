/**
 * Visual Display Arrangement Canvas Component (Step 46).
 * Renders the virtual desktop topology proportionally, handling negative coordinate offsets,
 * monitor aspect ratios, primary monitor indicators, and interactive selection.
 */

import { h, type UiNode } from '../primitives/uiNode.ts';
import type {
  DisplayId,
  DisplaySessionStatusDto,
  DisplayTopologySnapshotDto,
  ResolvedDisplayAssignmentDto,
} from '../../types/display.ts';

export interface DisplayArrangementViewProps {
  snapshot: DisplayTopologySnapshotDto | null;
  selectedDisplayId: DisplayId | null;
  resolvedAssignments: ResolvedDisplayAssignmentDto[];
  sessions: DisplaySessionStatusDto[];
  onSelectDisplay: (displayId: DisplayId) => void;
}

export function DisplayArrangementView(props: DisplayArrangementViewProps): UiNode {
  const { snapshot, selectedDisplayId, resolvedAssignments, sessions, onSelectDisplay } = props;

  if (!snapshot || snapshot.displays.length === 0) {
    return h(
      'div',
      {
        attrs: {
          'data-testid': 'display-arrangement-empty',
          class: 'kw-display-arrangement kw-display-arrangement-empty',
        },
      },
      h('p', { attrs: { class: 'kw-arrangement-empty-msg' } }, 'No displays detected.'),
    );
  }

  const bounds = snapshot.virtualDesktopBounds;
  const vWidth = Math.max(bounds.width, 640);
  const vHeight = Math.max(bounds.height, 480);
  const minX = bounds.left;
  const minY = bounds.top;

  // Normalized arrangement area dimensions
  const canvasWidth = 600;
  const canvasHeight = 240;
  const padding = 24;

  const availableW = canvasWidth - padding * 2;
  const availableH = canvasHeight - padding * 2;
  const scale = Math.min(availableW / vWidth, availableH / vHeight);

  const renderedW = vWidth * scale;
  const renderedH = vHeight * scale;
  const offsetX = padding + (availableW - renderedW) / 2;
  const offsetY = padding + (availableH - renderedH) / 2;

  const monitorNodes = snapshot.displays.map((display, index) => {
    const isSelected = display.id === selectedDisplayId;
    const assignment = resolvedAssignments.find((a) => a.displayId === display.id);
    const session = sessions.find((s) => s.displayId === display.id);
    const isPlaying = session?.state === 'active';

    const left = offsetX + (display.geometry.left - minX) * scale;
    const top = offsetY + (display.geometry.top - minY) * scale;
    const width = Math.max(display.geometry.width * scale, 60);
    const height = Math.max(display.geometry.height * scale, 45);

    const friendlyName = display.friendlyName || display.deviceName || `Display ${index + 1}`;
    const wallpaperTitle = assignment?.effectiveTitle ?? (assignment?.wallpaperId ? 'Assigned' : 'Unassigned');

    return h(
      'div',
      {
        key: display.id,
        attrs: {
          'data-testid': `display-box-${display.id}`,
          'data-display-id': display.id,
          'data-selected': isSelected ? 'true' : 'false',
          'data-primary': display.isPrimary ? 'true' : 'false',
          'data-active-session': isPlaying ? 'true' : 'false',
          role: 'button',
          tabindex: '0',
          'aria-label': `${friendlyName}${display.isPrimary ? ' (Primary)' : ''}, ${display.geometry.width} by ${display.geometry.height}, ${wallpaperTitle}`,
          'aria-selected': isSelected ? 'true' : 'false',
          class: `kw-arrangement-monitor${isSelected ? ' kw-monitor-selected' : ''}${
            display.isPrimary ? ' kw-monitor-primary' : ''
          }`,
          style: `position: absolute; left: ${left.toFixed(1)}px; top: ${top.toFixed(1)}px; width: ${width.toFixed(1)}px; height: ${height.toFixed(1)}px;`,
        },
        onClick: () => onSelectDisplay(display.id),
        onKeyDown: (e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault?.();
            onSelectDisplay(display.id);
          }
        },
      },
      // Header tag with display number and primary star
      h(
        'div',
        { attrs: { class: 'kw-monitor-box-header' } },
        h('span', { attrs: { class: 'kw-monitor-number' } }, String(index + 1)),
        display.isPrimary
          ? h('span', { attrs: { class: 'kw-monitor-star', title: 'Primary Display' } }, '★')
          : null,
        isPlaying
          ? h('span', { attrs: { class: 'kw-monitor-live-dot', title: 'Wallpaper Live' } }, '●')
          : null,
      ),
      // Central display name & resolution
      h(
        'div',
        { attrs: { class: 'kw-monitor-box-content' } },
        h('span', { attrs: { class: 'kw-monitor-name-label' } }, friendlyName),
        h(
          'span',
          { attrs: { class: 'kw-monitor-res-label' } },
          `${display.geometry.width}×${display.geometry.height}`,
        ),
      ),
      // Footer assigned wallpaper label
      h(
        'div',
        { attrs: { class: 'kw-monitor-box-footer' } },
        h(
          'span',
          { attrs: { class: 'kw-monitor-wallpaper-label', title: wallpaperTitle } },
          wallpaperTitle,
        ),
      ),
    );
  });

  return h(
    'div',
    {
      attrs: {
        'data-testid': 'display-arrangement-canvas',
        class: 'kw-display-arrangement',
        role: 'region',
        'aria-label': 'Visual Display Layout',
      },
    },
    h(
      'div',
      {
        attrs: {
          class: 'kw-arrangement-canvas-inner',
          style: `position: relative; width: ${canvasWidth}px; height: ${canvasHeight}px;`,
        },
      },
      ...monitorNodes,
    ),
  );
}

