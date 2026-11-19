/**
 * Centralized Navigation Registry (Step 22 `KW-SHL-002` & Step 27 §5.1).
 * Unimplemented destinations are explicitly marked unavailable (`DEC-SHL-04`).
 */

export type RouteId = 'dashboard' | 'library' | 'displays' | 'settings';

export interface NavigationRouteDefinition {
  id: RouteId;
  label: string;
  shortDescription: string;
  iconName: 'layout-dashboard' | 'grid' | 'monitor' | 'sliders';
  isAvailable: boolean;
  unavailableReason?: string;
}

export const NAVIGATION_ROUTES: readonly NavigationRouteDefinition[] = [
  {
    id: 'dashboard',
    label: 'Dashboard',
    shortDescription: 'Engine status overview and quick navigation',
    iconName: 'layout-dashboard',
    isAvailable: true,
  },
  {
    id: 'library',
    label: 'Wallpaper Library',
    shortDescription: 'Browse registered local wallpapers and inspect asset availability',
    iconName: 'grid',
    isAvailable: true,
  },
  {
    id: 'displays',
    label: 'Displays',
    shortDescription: 'Per-monitor wallpaper assignment and layout',
    iconName: 'monitor',
    isAvailable: true,
  },
  {
    id: 'settings',
    label: 'Settings',
    shortDescription: 'Performance, power policies, and application preferences',
    iconName: 'sliders',
    isAvailable: true,
  },
] as const;

export function getRouteDefinition(routeId: RouteId): NavigationRouteDefinition {
  const found = NAVIGATION_ROUTES.find((route) => route.id === routeId);
  if (!found) {
    throw new Error(`Unknown route id: ${String(routeId)}`);
  }
  return found;
}

