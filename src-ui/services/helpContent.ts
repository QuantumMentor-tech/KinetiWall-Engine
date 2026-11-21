/**
 * Authoritative Help Content Catalog & Diagnostic Mapping (Step 67).
 *
 * Grounded in verified KinetiWall product behavior:
 * - Local-first wallpaper library, registration, and relinking.
 * - Multi-monitor Independent, Span, and Clone layouts.
 * - Resource-aware playback policies (battery throttling, fullscreen pause, inactivity sleep).
 * - System capability readiness inspection.
 * - Evidence-based troubleshooting topics with honest limitations.
 */

import type { HelpTopic, SafeActionId } from '../types/help.ts';

export const HELP_TOPICS: readonly HelpTopic[] = [
  // ---------------------------------------------------------------------------
  // 1. Getting Started
  // ---------------------------------------------------------------------------
  {
    id: 'help-getting-started',
    title: 'Getting Started with KinetiWall',
    category: 'getting_started',
    summary:
      'Introduction to KinetiWall’s local-first architecture, background embedding, and system navigation.',
    content: [
      'KinetiWall is a high-performance live wallpaper engine designed specifically for Windows 10 and 11. It attaches animated video loops and interactive web packages directly behind desktop icons using isolated native Windows WorkerW hierarchy.',
      'Unlike cloud-dependent wallpaper apps, KinetiWall operates 100% locally on your machine: no telemetry, no tracking, and no external account requirements.',
      'Use the primary navigation rail on the left or keyboard shortcuts (Alt+1 for Dashboard, Alt+2 for Library, Alt+3 for Displays, Alt+4 for Settings) to move between application surfaces.',
    ],
    keywords: ['intro', 'welcome', 'overview', 'local', 'first-run', 'start', 'navigation'],
    recommendedAction: {
      actionId: 'onboarding.reopen',
      label: 'Launch First-Run Tour',
    },
  },

  // ---------------------------------------------------------------------------
  // 2. Library & Import
  // ---------------------------------------------------------------------------
  {
    id: 'help-library-manage',
    title: 'Managing Your Wallpaper Library',
    category: 'library_and_import',
    summary:
      'How to browse registered wallpapers, search metadata, filter by format, and inspect file availability.',
    content: [
      'The Wallpaper Library (Alt+2) presents all locally registered wallpapers in a responsive grid. Each wallpaper card shows its title, type (Video or Web), author, and verified disk availability status.',
      'Selecting a card opens the Metadata Inspector panel, where you can verify filesystem paths, edit custom titles or tags, inspect resolution dimensions, and check whether assets are verified on disk.',
      'Selecting a card in the library is strictly for inspection—it will never silently change your active wallpaper or disrupt desktop playback until you explicitly assign it in the Displays view.',
    ],
    keywords: ['library', 'catalog', 'browse', 'search', 'filter', 'sort', 'inspector', 'metadata'],
    recommendedAction: {
      actionId: 'nav.library',
      label: 'Open Wallpaper Library',
    },
  },
  {
    id: 'help-import-content',
    title: 'Importing Video and Web Wallpapers',
    category: 'library_and_import',
    summary:
      'Supported media formats, registration safety, and folder structure requirements.',
    content: [
      'Click "Add Wallpaper" in the Dashboard or Wallpaper Library to register new content. KinetiWall supports two primary wallpaper types:',
      '1. Video Wallpapers: Standard .mp4 and .webm video files with H.264, HEVC, or VP9 codecs.',
      '2. Web Wallpapers: HTML5 packages containing an index.html file, supporting CSS animations, Canvas 2D, WebGL, and optional JavaScript reactivity.',
      'Privacy Guarantee: KinetiWall will never automatically scan arbitrary hard drive folders without your explicit action. Only files and folders you select during registration are evaluated.',
    ],
    keywords: ['import', 'add', 'register', 'video', 'mp4', 'webm', 'html', 'web', 'packages'],
    recommendedAction: {
      actionId: 'nav.library',
      label: 'Go to Library to Add Wallpapers',
    },
  },

  // ---------------------------------------------------------------------------
  // 3. Playback & Policies
  // ---------------------------------------------------------------------------
  {
    id: 'help-playback-policies',
    title: 'System-Aware Playback Policies',
    category: 'playback_and_policies',
    summary:
      'How KinetiWall intelligently manages performance, battery life, fullscreen games, and display sleep.',
    content: [
      'KinetiWall incorporates centralized policy arbitration to prevent live wallpapers from consuming CPU/GPU resources while you work or game.',
      'Key policies include:',
      '• Fullscreen Application Pause: When a fullscreen game, video player, or presentation takes focus, wallpaper playback automatically pauses (or mutes).',
      '• Battery Power Optimization: When your device runs on DC battery power, KinetiWall can automatically throttle playback to 30 FPS or pause completely.',
      '• Inactivity & Display Power: If your system enters display sleep or user inactivity exceeds the threshold, rendering stops until input is detected.',
      '• Workstation Lock: When Windows is locked (Win+L), rendering suspends immediately to conserve energy.',
    ],
    keywords: ['policy', 'performance', 'battery', 'fullscreen', 'pause', 'throttle', 'power', 'arbitration'],
    relatedDiagnosticCodes: ['policy.fullscreen_pause', 'policy.battery_throttle', 'policy.inactivity_pause'],
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Configure Playback Policies in Settings',
    },
  },
  {
    id: 'help-audio-reactivity',
    title: 'Audio Reactivity & Visualizers',
    category: 'playback_and_policies',
    summary:
      'Configuring Windows WASAPI audio capture for sound-reactive wallpapers.',
    content: [
      'Interactive web wallpapers can visualize desktop audio in real time using KinetiWall’s WASAPI loopback capture service.',
      'Audio processing is performed locally with zero latency, utilizing fast Fourier transform (FFT) analysis separated into bass, midrange, and treble frequency bands.',
      'You can adjust visualizer sensitivity and temporal smoothing under the Audio preferences tab in Settings, or mute reactivity when another media player is active.',
    ],
    keywords: ['audio', 'reactive', 'visualizer', 'wasapi', 'sound', 'music', 'fft', 'spectrum'],
    relatedDiagnosticCodes: ['audio.wasapi_capture_active'],
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Open Audio Settings',
    },
  },

  // ---------------------------------------------------------------------------
  // 4. Displays & Layout
  // ---------------------------------------------------------------------------
  {
    id: 'help-displays-layout',
    title: 'Multi-Monitor Presentation & Layout Modes',
    category: 'displays_and_layout',
    summary:
      'Configuring per-display assignments, panoramic Spans, and mirrored Clones across connected monitors.',
    content: [
      'The Displays & Layout view (Alt+3) detects all connected monitors and visualizes their virtual desktop coordinates and resolutions.',
      'Supported presentation modes:',
      '1. Independent (Default): Each monitor can display an independent wallpaper with dedicated volume and scaling controls.',
      '2. Span: A single high-resolution panoramic wallpaper spans across all connected monitors according to their physical arrangement.',
      '3. Clone: A primary wallpaper is synchronously mirrored across all connected displays.',
      'Click "Assign Wallpaper" on any display card to open the picker dialog and apply your selection.',
    ],
    keywords: ['displays', 'monitor', 'multi-monitor', 'layout', 'span', 'clone', 'independent', 'assign'],
    recommendedAction: {
      actionId: 'nav.displays',
      label: 'Manage Displays & Layout',
    },
  },

  // ---------------------------------------------------------------------------
  // 5. Settings & Preferences
  // ---------------------------------------------------------------------------
  {
    id: 'help-settings-preferences',
    title: 'Application Preferences & Personalization',
    category: 'settings_and_preferences',
    summary:
      'Configuring autostart, system tray behavior, themes, accessibility, and notifications.',
    content: [
      'The Settings view (Alt+4) provides centralized, typed preference controls conforming to Schema V1:',
      '• General: Run on Windows Startup, start minimized, minimize to notification tray, interface language, and guided tour launcher.',
      '• Appearance & Themes: Choose between high-contrast Neon Dark, clean Glass Light, or automatic synchronization with Windows Dark/Light mode.',
      '• Reduced Motion: Honor Windows accessibility settings or enforce reduced UI transitions to minimize visual motion.',
      '• In-App Notifications: Control transient toast notifications (All, Errors Only, or Critical Only).',
      'All changes are held in a draft state until you click "Save Changes" (Ctrl+S) or discard them (Ctrl+Shift+Z).',
    ],
    keywords: ['settings', 'preferences', 'theme', 'dark', 'light', 'reduced motion', 'autostart', 'tray'],
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Open Application Settings',
    },
  },
  {
    id: 'help-keyboard-shortcuts',
    title: 'Keyboard Shortcuts Reference',
    category: 'settings_and_preferences',
    summary:
      'Quick reference for accessible keyboard navigation and productivity commands.',
    content: [
      'KinetiWall is fully operable via keyboard shortcuts for fast, accessible desktop navigation:',
      '• Alt+1: Go to Dashboard',
      '• Alt+2: Go to Wallpaper Library',
      '• Alt+3: Go to Displays & Layout',
      '• Alt+4: Go to Settings',
      '• Ctrl+F: Focus Library Search',
      '• F5: Refresh Wallpaper Catalog',
      '• Ctrl+Shift+R: Refresh Display Topology',
      '• Ctrl+S: Save Settings Draft',
      '• Ctrl+Shift+Z: Discard Settings Draft',
      '• ? (or nav footer button): Open Keyboard Shortcuts Reference dialog',
      '• F1 (or nav footer button): Open Help & Troubleshooting dialog',
      '• Escape: Close active modal dialog or inspector panel',
    ],
    keywords: ['shortcuts', 'keyboard', 'hotkeys', 'accessibility', 'f1', 'commands'],
    recommendedAction: {
      actionId: 'help.shortcuts',
      label: 'Open Shortcuts Cheat Sheet',
    },
  },

  // ---------------------------------------------------------------------------
  // 6. Evidence-Based Troubleshooting
  // ---------------------------------------------------------------------------
  {
    id: 'ts-playback-paused',
    title: 'Wallpaper Paused or Not Animating',
    category: 'troubleshooting',
    summary:
      'Troubleshooting wallpapers that stop moving or remain suspended.',
    content: [
      'If your wallpaper is not animating, KinetiWall’s resource policy arbitrator is likely holding it in a paused or suspended state to protect system performance.',
    ],
    keywords: ['paused', 'frozen', 'stopped', 'not moving', 'black', 'battery', 'fullscreen'],
    relatedDiagnosticCodes: [
      'policy.fullscreen_pause',
      'policy.battery_throttle',
      'policy.battery_pause',
      'policy.inactivity_pause',
      'policy.workstation_lock',
    ],
    troubleshootingDetail: {
      whatHappened:
        'The wallpaper playback surface is currently paused and not producing frames.',
      whatIsKnown:
        'A system policy condition (such as a focused fullscreen window, battery conservation threshold, or display inactivity) was detected by the policy arbitrator.',
      likelyInterpretation:
        'Another application is in fullscreen mode (e.g., a game, media player, or maximized browser video), or the PC is running on battery power with battery conservation enabled.',
      recommendedNextStep:
        'Check the Displays view to see the active playback session status badge. To change pause rules, open Settings > Playback Policies and adjust Fullscreen or Battery action settings.',
      limitations:
        'Certain borderless windowed applications may be classified as fullscreen by Windows API heuristics if they cover the entire monitor workspace.',
    },
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Review Playback Policies in Settings',
    },
  },
  {
    id: 'ts-workerw-unsplit',
    title: 'Wallpaper Not Embedding or Black Desktop',
    category: 'troubleshooting',
    summary:
      'Resolving desktop WorkerW shell hierarchy and Windows Explorer embedding conditions.',
    content: [
      'KinetiWall requires a validated WorkerW window behind desktop icons to embed live wallpapers without obscuring your desktop shortcuts.',
    ],
    keywords: ['workerw', 'progman', 'embed', 'desktop', 'black screen', 'icons', 'explorer'],
    relatedDiagnosticCodes: [
      'desktop_integration.limited',
      'desktop.unsplit_topology',
      'desktop.shell_restarting',
    ],
    troubleshootingDetail: {
      whatHappened:
        'The live wallpaper surface failed to attach to the desktop background or displays a solid color.',
      whatIsKnown:
        'The Windows Explorer shell topology is either in a classic unsplit Progman state or Explorer is restarting.',
      likelyInterpretation:
        'Windows Explorer has not yet generated the background WorkerW separation layer, or a custom desktop theme software interfered with window hierarchy.',
      recommendedNextStep:
        'Navigate to Displays & Layout and click "Refresh" (or press Ctrl+Shift+R) to trigger non-invasive desktop re-discovery. If Explorer recently crashed, wait for the desktop to reload.',
      limitations:
        'Third-party shell modifications (e.g. customized taskbars or alternative desktop managers) can alter Win32 window class hierarchies in unpredictable ways.',
    },
    recommendedAction: {
      actionId: 'displays.refresh',
      label: 'Refresh Display Topology',
    },
  },
  {
    id: 'ts-video-codec',
    title: 'Video Wallpaper Fails to Play',
    category: 'troubleshooting',
    summary:
      'Resolving video playback errors, missing codecs, and hardware acceleration issues.',
    content: [
      'Video playback relies on Windows Media Foundation and DirectShow hardware decoders.',
    ],
    keywords: ['codec', 'video error', 'mp4', 'webm', 'h264', 'hevc', 'gpu', 'hardware acceleration'],
    relatedDiagnosticCodes: ['video_player.limited', 'playback.video_backend_error'],
    troubleshootingDetail: {
      whatHappened:
        'A video wallpaper failed to start or rendered with a decoder error.',
      whatIsKnown:
        'The media player process reported an initialization failure or unsupported codec format.',
      likelyInterpretation:
        'The video file uses an uncommon codec (e.g. AV1 or ProRes) not supported by default Windows decoders, or GPU hardware acceleration is disabled.',
      recommendedNextStep:
        'Verify that GPU hardware acceleration is enabled under Settings > Rendering. Ensure the video plays in standard Windows Media Player. Re-check system capability readiness in Settings.',
      limitations:
        'KinetiWall does not bundle third-party external codecs to keep the installation clean and license-compliant.',
    },
    recommendedAction: {
      actionId: 'readiness.recheck',
      label: 'Check System Capability Readiness',
    },
  },
  {
    id: 'ts-webview2-missing',
    title: 'Interactive Web Wallpaper Blank or Disabled',
    category: 'troubleshooting',
    summary:
      'Addressing missing Microsoft Edge WebView2 Evergreen runtime for web wallpapers.',
    content: [
      'Interactive HTML5, WebGL, and JavaScript wallpapers require the Microsoft Edge WebView2 runtime.',
    ],
    keywords: ['webview2', 'edge', 'web wallpaper', 'html', 'javascript', 'canvas', 'blank'],
    relatedDiagnosticCodes: ['web_engine.limited', 'playback.webview2_missing'],
    troubleshootingDetail: {
      whatHappened:
        'An interactive web wallpaper rendered a blank surface or was disabled upon loading.',
      whatIsKnown:
        'The native WebView2 COM interface could not be initialized.',
      likelyInterpretation:
        'The Microsoft Edge WebView2 Evergreen Runtime is not installed on this PC or its installation has been corrupted.',
      recommendedNextStep:
        'Download and install the official Microsoft Edge WebView2 Evergreen Runtime from Microsoft. Once installed, re-check system readiness in Settings.',
      limitations:
        'Enterprise or sanitized Windows installations may restrict WebView2 execution via group policy.',
    },
    recommendedAction: {
      actionId: 'readiness.recheck',
      label: 'Re-check System Readiness',
    },
  },
  {
    id: 'ts-audio-wasapi',
    title: 'Audio Reactivity Visualizer Idle',
    category: 'troubleshooting',
    summary:
      'Resolving audio-reactive wallpapers that do not respond to music or sound.',
    content: [
      'Audio reactivity requires an active Windows WASAPI loopback audio stream.',
    ],
    keywords: ['audio', 'visualizer', 'sound', 'music', 'wasapi', 'no response', 'silent'],
    relatedDiagnosticCodes: ['audio_reactivity.limited', 'audio.no_default_device'],
    troubleshootingDetail: {
      whatHappened:
        'Sound-reactive wallpapers are not animating or visualizing audio frequencies.',
      whatIsKnown:
        'WASAPI audio capture is reported limited, disabled, or no audio signal is arriving.',
      likelyInterpretation:
        'No media or music is currently playing, the output audio device is muted, or audio reactivity is disabled in application settings.',
      recommendedNextStep:
        'Verify that media is playing through your default speakers or headphones. Check Settings > Audio to ensure Master Mute is unchecked and sensitivity is above zero.',
      limitations:
        'WASAPI loopback captures only audio rendered to the default playback device. Specialized ASIO hardware streams are not captured.',
    },
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Open Audio Settings',
    },
  },
  {
    id: 'ts-asset-missing',
    title: 'Wallpaper Asset Reported Missing on Disk',
    category: 'troubleshooting',
    summary:
      'Recovering wallpapers when underlying files have been moved, renamed, or deleted.',
    content: [
      'KinetiWall continuously verifies asset file integrity before attempting playback.',
    ],
    keywords: ['missing', 'file moved', 'deleted', 'inaccessible', 'relink', 'asset not found'],
    relatedDiagnosticCodes: ['catalog.asset_missing', 'catalog.inaccessible'],
    troubleshootingDetail: {
      whatHappened:
        'A wallpaper card in the library displays an "Unavailable" or "Missing" status badge.',
      whatIsKnown:
        'The file path registered in the catalog manifest cannot be opened or does not exist on disk.',
      likelyInterpretation:
        'The media file or folder was moved, renamed, deleted, or resides on an unmounted external drive.',
      recommendedNextStep:
        'Open the Wallpaper Library, select the missing wallpaper card, and use the "Relink Asset" tool in the Metadata Inspector to locate the file in its new location.',
      limitations:
        'If the source file was permanently deleted, the entry can be removed from the catalog using the "Remove from Catalog" action.',
    },
    recommendedAction: {
      actionId: 'nav.library',
      label: 'Open Wallpaper Library to Relink',
    },
  },
  {
    id: 'ts-settings-save-failed',
    title: 'Settings Changes Could Not Be Saved',
    category: 'troubleshooting',
    summary:
      'Resolving preference persistence errors, disk write constraints, and validation errors.',
    content: [
      'KinetiWall uses atomic file replacement and automatic backups to safeguard configuration data.',
    ],
    keywords: ['settings', 'save failed', 'error', 'config.json', 'permissions', 'validation'],
    relatedDiagnosticCodes: ['settings.save_failed', 'settings.validation_error'],
    troubleshootingDetail: {
      whatHappened:
        'The save bar in Settings reported "Save Failed" and preferences were not written to disk.',
      whatIsKnown:
        'The settings store rejected the write operation or the disk rejected file creation.',
      likelyInterpretation:
        'One or more modified settings contain invalid values (e.g. empty language, out-of-range framerate), or the user data folder is write-protected.',
      recommendedNextStep:
        'Inspect the red validation error badges in the Settings view. Correct highlighted errors or click "Discard Changes" (Ctrl+Shift+Z) to restore the last valid saved configuration.',
      limitations:
        'If config.json becomes corrupted, KinetiWall automatically falls back to config.json.bak or safe factory defaults.',
    },
    recommendedAction: {
      actionId: 'nav.settings',
      label: 'Inspect Settings Validation',
    },
  },
  {
    id: 'ts-unknown-issue',
    title: 'Unrecognized Condition or Diagnostic Notice',
    category: 'troubleshooting',
    summary:
      'Guidance for issues without a verified root cause or specific diagnostic code.',
    content: [
      'When an unexpected state occurs that does not match a known issue code, KinetiWall reports the condition honestly without fabricating explanations.',
    ],
    keywords: ['unknown', 'general', 'error', 'notice', 'diagnostic', 'bug', 'crash'],
    relatedDiagnosticCodes: ['unknown', 'generic_error'],
    troubleshootingDetail: {
      whatHappened:
        'An application event or warning occurred that has no specific automated diagnostic code.',
      whatIsKnown:
        'The exact root cause cannot be conclusively confirmed from current evidence.',
      likelyInterpretation:
        'The condition could stem from transient system resource constraints, driver updates, or Windows display switching.',
      recommendedNextStep:
        '1. Check System Capability Readiness in Settings to verify all essential subsystems.\n2. Refresh display topology in Displays (Ctrl+Shift+R).\n3. Re-read the latest notification in the notification toast or log overlay.',
      limitations:
        'Do not terminate unrelated Windows processes or edit registry keys based on speculation.',
    },
    recommendedAction: {
      actionId: 'readiness.recheck',
      label: 'Run System Capability Readiness Check',
    },
  },
] as const;

/**
 * Explicit mapping between diagnostic reason codes and troubleshooting help topic IDs.
 */
export const DIAGNOSTIC_REASON_MAP: Readonly<Record<string, string>> = {
  // Policies & Arbitration
  'policy.fullscreen_pause': 'ts-playback-paused',
  'policy.battery_throttle': 'ts-playback-paused',
  'policy.battery_pause': 'ts-playback-paused',
  'policy.inactivity_pause': 'ts-playback-paused',
  'policy.workstation_lock': 'ts-playback-paused',

  // Desktop & Shell
  'desktop_integration.limited': 'ts-workerw-unsplit',
  'desktop.unsplit_topology': 'ts-workerw-unsplit',
  'desktop.shell_restarting': 'ts-workerw-unsplit',

  // Media & Backends
  'video_player.limited': 'ts-video-codec',
  'playback.video_backend_error': 'ts-video-codec',
  'web_engine.limited': 'ts-webview2-missing',
  'playback.webview2_missing': 'ts-webview2-missing',

  // Audio Reactivity
  'audio_reactivity.limited': 'ts-audio-wasapi',
  'audio.no_default_device': 'ts-audio-wasapi',

  // Catalog & Assets
  'catalog.asset_missing': 'ts-asset-missing',
  'catalog.inaccessible': 'ts-asset-missing',

  // Settings & Storage
  'settings.save_failed': 'ts-settings-save-failed',
  'settings.validation_error': 'ts-settings-save-failed',
};

/**
 * Resolves the corresponding HelpTopic for an observed diagnostic reason code.
 * Falls back to 'ts-unknown-issue' when the code is not explicitly recognized.
 */
export function resolveHelpTopicForDiagnostic(reasonCode?: string | null): HelpTopic {
  if (!reasonCode) {
    return getHelpTopicById('ts-unknown-issue')!;
  }

  const topicId = DIAGNOSTIC_REASON_MAP[reasonCode];
  if (topicId) {
    const topic = getHelpTopicById(topicId);
    if (topic) {
      return topic;
    }
  }

  return getHelpTopicById('ts-unknown-issue')!;
}

export function getHelpTopicById(id: string): HelpTopic | undefined {
  return HELP_TOPICS.find((t) => t.id === id);
}

export const ALL_HELP_TOPICS = HELP_TOPICS;

