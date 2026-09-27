import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const invoke = vi.fn();
let desktop = true;

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  isTauri: () => desktop,
  convertFileSrc: (path: string) => `file://${path}`,
}));

import type { CacheReport, Settings } from '../lib/bindings';
import { SettingsRoute } from './SettingsRoute';
import { useSettingsStore } from '../stores/settings-store';

function settings(overrides: Partial<Settings> = {}): Settings {
  return {
    theme: 'system',
    startWithWindows: false,
    scanSchedule: 'on_launch',
    lastScanUnix: 1_800_000_000,
    nextScanUnix: null,
    exclusions: [],
    ...overrides,
  };
}

function cache(overrides: Partial<CacheReport> = {}): CacheReport {
  return {
    thumbnailCachePath: 'C:\\Users\\me\\AppData\\Local\\Sift\\thumbs',
    thumbnailCacheBytes: 2_097_152,
    thumbnailCacheFiles: 42,
    skippedFolders: [],
    skippedFoldersTruncated: false,
    skippedTotal: 0,
    ...overrides,
  };
}

/** Answers the two commands the screen asks for on mount. */
function serve(settingsValue: Settings, cacheValue: CacheReport) {
  invoke.mockImplementation((command: string) => {
    if (command === 'get_settings') return Promise.resolve(settingsValue);
    if (command === 'get_cache_report') return Promise.resolve(cacheValue);
    return Promise.resolve(undefined);
  });
}

beforeEach(() => {
  desktop = true;
  invoke.mockReset();
  useSettingsStore.setState({
    settings: null,
    cache: null,
    loading: false,
    saving: false,
    error: null,
    cacheError: null,
    startWithWindowsError: null,
    exclusionError: null,
  });
});

afterEach(cleanup);

describe('Settings screen', () => {
  it('says nothing is excluded when the list is empty', async () => {
    serve(settings(), cache());
    render(<SettingsRoute desktopAvailable />);
    expect(await screen.findByText('No folders are excluded')).toBeTruthy();
  });

  it('lists an excluded folder and can remove it', async () => {
    const value = settings({ exclusions: [{ path: 'C:\\Users\\me\\Videos\\Old', label: 'Old', addedAtUnix: 1 }] });
    serve(value, cache());
    render(<SettingsRoute desktopAvailable />);
    const remove = await screen.findByRole('button', { name: /stop excluding old/i });
    expect(screen.getByText('C:\\Users\\me\\Videos\\Old')).toBeTruthy();
    fireEvent.click(remove);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('remove_exclusion', { path: 'C:\\Users\\me\\Videos\\Old' }));
  });

  it('marks the folders Windows would not let Sift read', async () => {
    serve(settings(), cache({
      skippedTotal: 3,
      skippedFolders: [{ path: 'C:\\Users\\me\\Documents\\Private', reason: 'Sift needs permission to read this folder' }],
    }));
    render(<SettingsRoute desktopAvailable />);
    expect(await screen.findByText('3 folders passed over, mostly for lack of permission.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /view list/i }));
    expect(screen.getByText('C:\\Users\\me\\Documents\\Private')).toBeTruthy();
  });

  it('reports the thumbnail cache and clears it', async () => {
    serve(settings(), cache());
    render(<SettingsRoute desktopAvailable />);
    expect(await screen.findByText(/2.0 MB in 42 images/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Clear the thumbnail cache' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('clear_thumbnail_cache'));
  });

  it('saves an appearance choice', async () => {
    serve(settings(), cache());
    render(<SettingsRoute desktopAvailable />);
    fireEvent.click(await screen.findByRole('button', { name: 'Dark' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_theme', { theme: 'dark' }));
    expect(useSettingsStore.getState().theme).toBe('dark');
  });

  it('keeps the choice usable without the desktop app', async () => {
    desktop = false;
    serve(settings(), cache());
    render(<SettingsRoute desktopAvailable={false} />);
    expect(await screen.findByText(/only appearance and the scan schedule are kept/)).toBeTruthy();
    // The native-only rows say so instead of pretending to work.
    expect(screen.getByRole('switch', { name: /start with windows/i }).hasAttribute('disabled')).toBe(true);
  });
});
