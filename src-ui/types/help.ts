/**
 * In-App Help, Contextual Guidance & Troubleshooting Contracts (Step 67).
 *
 * Defines the typed content model, category taxonomy, troubleshooting details,
 * and safe application action bindings.
 */

export type HelpCategory =
  | 'getting_started'
  | 'library_and_import'
  | 'playback_and_policies'
  | 'displays_and_layout'
  | 'settings_and_preferences'
  | 'troubleshooting';

export const HELP_CATEGORIES: readonly { id: HelpCategory; label: string; icon: string }[] = [
  { id: 'getting_started', label: 'Getting Started', icon: '🚀' },
  { id: 'library_and_import', label: 'Library & Import', icon: '📁' },
  { id: 'playback_and_policies', label: 'Playback & Policies', icon: '▶' },
  { id: 'displays_and_layout', label: 'Displays & Layout', icon: '🖥' },
  { id: 'settings_and_preferences', label: 'Settings & Preferences', icon: '⚙' },
  { id: 'troubleshooting', label: 'Troubleshooting & Diagnostics', icon: '🔧' },
] as const;

/**
 * Whitelist of safe, strictly verified application action identifiers that help topics
 * are permitted to reference. Arbitrary shell execution or privileged operations are forbidden.
 */
export type SafeActionId =
  | 'nav.dashboard'
  | 'nav.library'
  | 'nav.displays'
  | 'nav.settings'
  | 'library.refresh'
  | 'displays.refresh'
  | 'onboarding.reopen'
  | 'readiness.recheck'
  | 'help.shortcuts';

export interface TroubleshootingDetail {
  whatHappened: string;
  whatIsKnown: string;
  likelyInterpretation: string;
  recommendedNextStep: string;
  limitations: string;
}

export interface HelpTopic {
  id: string;
  title: string;
  category: HelpCategory;
  summary: string;
  content: string[];
  keywords: string[];
  relatedDiagnosticCodes?: string[];
  recommendedAction?: {
    actionId: SafeActionId;
    label: string;
  };
  troubleshootingDetail?: TroubleshootingDetail;
}

/**
 * Filters and searches help topics by query string and optional category filter.
 * Case-insensitive, trims whitespace, searches title, summary, and keywords.
 */
export function searchHelpTopics(
  topics: readonly HelpTopic[],
  query: string,
  categoryFilter: HelpCategory | 'all' = 'all',
): HelpTopic[] {
  const trimmed = query.trim().toLowerCase();

  return topics.filter((topic) => {
    // 1. Category filter
    if (categoryFilter !== 'all' && topic.category !== categoryFilter) {
      return false;
    }

    // 2. Query matching
    if (!trimmed) {
      return true;
    }

    if (topic.title.toLowerCase().includes(trimmed)) {
      return true;
    }

    if (topic.summary.toLowerCase().includes(trimmed)) {
      return true;
    }

    if (topic.keywords.some((kw) => kw.toLowerCase().includes(trimmed))) {
      return true;
    }

    if (topic.content.some((para) => para.toLowerCase().includes(trimmed))) {
      return true;
    }

    return false;
  });
}

