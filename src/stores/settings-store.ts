import { create } from 'zustand';
import { isTauri } from '@tauri-apps/api/core';
import { commands, type CacheReport, type Settings } from '../lib/bindings';

export type ThemeMode = 'system' | 'light' | 'dark';
export type ScanSchedule = 'on_launch' | 'daily' | 'weekly' | 'manual';

const THEME_KEY = 'sift-theme';
const SCHEDULE_KEY = 'sift-scan-schedule';

export const themes: { value: ThemeMode; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

export const schedules: { value: ScanSchedule; label: string; hint: string }[] = [
  { value: 'on_launch', label: 'Every time Sift opens', hint: 'The default. Your index is rebuilt at every start.' },
  { value: 'daily', label: 'Once a day', hint: 'A full scan at most once every 24 hours.' },
  { value: 'weekly', label: 'Once a week', hint: 'A full scan at most once every 7 days.' },
  { value: 'manual', label: 'Never automatically', hint: 'Sift only watches for changes as you work.' },
];

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStored(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // A browser that refuses storage still gets the choice for this session.
  }
}

function storedTheme(): ThemeMode {
  const saved = readStored(THEME_KEY);
  return saved === 'light' || saved === 'dark' || saved === 'system' ? saved : 'system';
}

function storedSchedule(): ScanSchedule {
  const saved = readStored(SCHEDULE_KEY);
  return saved === 'daily' || saved === 'weekly' || saved === 'manual' || saved === 'on_launch' ? saved : 'on_launch';
}

function messageFrom(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error) return error;
  return fallback;
}

interface SettingsStore {
  theme: ThemeMode;
  scanSchedule: ScanSchedule;
  startWithWindows: boolean;
  settings: Settings | null;
  cache: CacheReport | null;
  loading: boolean;
  saving: boolean;
  /** A failure reading or writing the preferences themselves. */
  error: string | null;
  /** A failure reading or clearing the two caches. */
  cacheError: string | null;
  /** Windows refused the autostart change. Kept apart because it is recoverable
   *  in Task Manager rather than by retrying. */
  startWithWindowsError: string | null;
  exclusionError: string | null;
  load: () => Promise<void>;
  chooseTheme: (theme: ThemeMode) => Promise<void>;
  chooseSchedule: (schedule: ScanSchedule) => Promise<void>;
  toggleStartWithWindows: (enabled: boolean) => Promise<void>;
  addExclusion: (path: string) => Promise<boolean>;
  removeExclusion: (path: string) => Promise<void>;
  loadCache: () => Promise<void>;
  clearThumbnailCache: () => Promise<void>;
  clearSkippedFolders: () => Promise<void>;
  clearError: () => void;
}

export const useSettingsStore = create<SettingsStore>((set) => ({
  theme: storedTheme(),
  scanSchedule: storedSchedule(),
  startWithWindows: false,
  settings: null,
  cache: null,
  loading: false,
  saving: false,
  error: null,
  cacheError: null,
  startWithWindowsError: null,
  exclusionError: null,

  load: async () => {
    if (!isTauri()) return;
    set({ loading: true, error: null });
    try {
      const settings = await commands.getSettings();
      writeStored(THEME_KEY, settings.theme);
      writeStored(SCHEDULE_KEY, settings.scanSchedule);
      set({
        settings,
        theme: storedTheme(),
        scanSchedule: storedSchedule(),
        startWithWindows: settings.startWithWindows,
        loading: false,
      });
    } catch (error: unknown) {
      set({ error: messageFrom(error, 'Sift could not read your settings.'), loading: false });
    }
  },

  chooseTheme: async (theme) => {
    // Applied first: appearance should never wait on a round trip.
    writeStored(THEME_KEY, theme);
    set({ theme, error: null });
    if (!isTauri()) return;
    try {
      set({ settings: await commands.setTheme(theme) });
    } catch (error: unknown) {
      set({ error: messageFrom(error, 'Sift could not save that appearance choice.') });
    }
  },

  chooseSchedule: async (schedule) => {
    writeStored(SCHEDULE_KEY, schedule);
    set({ scanSchedule: schedule, error: null });
    if (!isTauri()) return;
    try {
      set({ settings: await commands.setScanSchedule(schedule), saving: false });
    } catch (error: unknown) {
      set({ error: messageFrom(error, 'Sift could not save that scan schedule.') });
    }
  },

  toggleStartWithWindows: async (enabled) => {
    set({ saving: true, startWithWindowsError: null, error: null });
    if (!isTauri()) {
      set({ saving: false });
      return;
    }
    try {
      // Trust the answer, not the request: Windows owns this entry.
      const applied = await commands.setStartWithWindows(enabled);
      const settings = await commands.getSettings();
      set({ startWithWindows: applied && settings.startWithWindows, settings, saving: false });
    } catch (error: unknown) {
      set({
        startWithWindowsError: messageFrom(error, 'Windows would not let Sift change its startup entry.'),
        saving: false,
      });
    }
  },

  addExclusion: async (path) => {
    set({ exclusionError: null, saving: true });
    if (!isTauri()) {
      set({ saving: false, exclusionError: 'Excluding a folder needs the Windows app, which can check the path is really inside your profile.' });
      return false;
    }
    try {
      set({ settings: await commands.addExclusion(path), saving: false });
      return true;
    } catch (error: unknown) {
      set({ exclusionError: messageFrom(error, 'Sift could not exclude that folder.'), saving: false });
      return false;
    }
  },

  removeExclusion: async (path) => {
    set({ exclusionError: null, saving: true });
    if (!isTauri()) {
      set({ saving: false });
      return;
    }
    try {
      set({ settings: await commands.removeExclusion(path), saving: false });
    } catch (error: unknown) {
      set({ exclusionError: messageFrom(error, 'Sift could not remove that folder.'), saving: false });
    }
  },

  loadCache: async () => {
    if (!isTauri()) return;
    set({ cacheError: null });
    try {
      set({ cache: await commands.getCacheReport() });
    } catch (error: unknown) {
      set({ cacheError: messageFrom(error, 'Sift could not measure its caches.') });
    }
  },

  clearThumbnailCache: async () => {
    if (!isTauri()) return;
    set({ saving: true, cacheError: null });
    try {
      set({ cache: await commands.clearThumbnailCache(), saving: false });
    } catch (error: unknown) {
      set({ cacheError: messageFrom(error, 'Sift could not clear the thumbnail cache.'), saving: false });
    }
  },

  clearSkippedFolders: async () => {
    if (!isTauri()) return;
    set({ saving: true, cacheError: null });
    try {
      set({ cache: await commands.clearSkippedFolders(), saving: false });
    } catch (error: unknown) {
      set({ cacheError: messageFrom(error, 'Sift could not clear the skipped folder list.'), saving: false });
    }
  },

  clearError: () => set({ error: null, cacheError: null, startWithWindowsError: null, exclusionError: null }),
}));

/** "in 3 hours" / "yesterday", for the scan schedule and the last full scan. */
export function relativeTime(unixSeconds: number | null, now: number = Date.now()): string {
  if (unixSeconds === null) return 'never';
  const seconds = Math.round((now / 1000 - unixSeconds));
  const future = seconds < 0;
  const magnitude = Math.abs(seconds);
  const span =
    magnitude < 90 ? 'a moment' :
    magnitude < 5_400 ? `${Math.round(magnitude / 60)} minutes` :
    magnitude < 172_800 ? `${Math.round(magnitude / 3_600)} hours` :
    `${Math.round(magnitude / 86_400)} days`;
  return future ? `in ${span}` : `${span} ago`;
}
