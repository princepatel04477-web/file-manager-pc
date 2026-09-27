import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import {
  CalendarClock, Check, FolderX, HardDrive, Image as ImageIcon, LockKeyhole, MonitorCog, Plus,
  RefreshCcw, Sparkles, Trash2, TriangleAlert, X,
} from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import { isTauri } from '@tauri-apps/api/core';
import { readableSize } from '../components/FileList';
import {
  relativeTime,
  schedules,
  themes,
  useSettingsStore,
  type ScanSchedule,
  type ThemeMode,
} from '../stores/settings-store';

const PERMISSION_REASON = 'Sift needs permission to read this folder';

function Section({
  icon,
  title,
  description,
  children,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="settings-section">
      <header className="settings-section-head">
        <span className="settings-section-icon">{icon}</span>
        <div>
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
      </header>
      <div className="settings-section-body">{children}</div>
    </section>
  );
}

function SettingsSkeleton() {
  return (
    <div className="settings-skeleton" role="status" aria-label="Loading your settings">
      {[0, 1, 2, 3].map((row) => (
        <div className="skeleton-line skeleton-row" key={row} />
      ))}
      <span className="sr-only">Loading your settings…</span>
    </div>
  );
}

interface SettingsRouteProps {
  desktopAvailable: boolean;
}

/**
 * Settings: the folders Sift never indexes, how it looks, whether it starts with
 * Windows, when it scans, and the two caches it keeps.
 */
export function SettingsRoute({ desktopAvailable }: SettingsRouteProps) {
  const reducedMotion = useReducedMotion();
  const settings = useSettingsStore((state) => state.settings);
  const cache = useSettingsStore((state) => state.cache);
  const loading = useSettingsStore((state) => state.loading);
  const saving = useSettingsStore((state) => state.saving);
  const error = useSettingsStore((state) => state.error);
  const cacheError = useSettingsStore((state) => state.cacheError);
  const startWithWindowsError = useSettingsStore((state) => state.startWithWindowsError);
  const exclusionError = useSettingsStore((state) => state.exclusionError);
  const theme = useSettingsStore((state) => state.theme);
  const scanSchedule = useSettingsStore((state) => state.scanSchedule);
  const startWithWindows = useSettingsStore((state) => state.startWithWindows);
  const load = useSettingsStore((state) => state.load);
  const chooseTheme = useSettingsStore((state) => state.chooseTheme);
  const chooseSchedule = useSettingsStore((state) => state.chooseSchedule);
  const toggleStartWithWindows = useSettingsStore((state) => state.toggleStartWithWindows);
  const addExclusion = useSettingsStore((state) => state.addExclusion);
  const removeExclusion = useSettingsStore((state) => state.removeExclusion);
  const loadCache = useSettingsStore((state) => state.loadCache);
  const clearThumbnailCache = useSettingsStore((state) => state.clearThumbnailCache);
  const clearSkippedFolders = useSettingsStore((state) => state.clearSkippedFolders);
  const clearError = useSettingsStore((state) => state.clearError);

  const [path, setPath] = useState('');
  const [showSkipped, setShowSkipped] = useState(false);

  useEffect(() => {
    if (!desktopAvailable) return;
    void load();
    void loadCache();
  }, [desktopAvailable, load, loadCache]);

  const exclusions = settings?.exclusions ?? [];
  const skipped = cache?.skippedFolders ?? [];
  const scheduleHint = schedules.find((option) => option.value === scanSchedule)?.hint ?? '';
  const motionProps = reducedMotion ? { duration: 0 } : { duration: 0.26, ease: [0.22, 1, 0.36, 1] as const };

  async function submitExclusion(event: FormEvent) {
    event.preventDefault();
    const trimmed = path.trim();
    if (!trimmed) return;
    if (await addExclusion(trimmed)) setPath('');
  }

  return (
    <div className="settings-page">
      {error && (
        <div className="alert-banner error-banner" role="alert">
          <TriangleAlert size={17} />
          <span>{error}</span>
          <button type="button" aria-label="Dismiss error" onClick={clearError}><X size={15} /></button>
        </div>
      )}

      {loading && !settings ? <SettingsSkeleton /> : (
        <motion.div className="settings-sections" initial={{ opacity: 0, y: reducedMotion ? 0 : 6 }} animate={{ opacity: 1, y: 0 }} transition={motionProps}>
          <Section icon={<Sparkles size={16} />} title="Appearance" description="How Sift looks on this PC. System follows Windows.">
            <div className="segmented-control" role="group" aria-label="Appearance">
              {themes.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  className={`segment${theme === option.value ? ' selected' : ''}`}
                  aria-pressed={theme === option.value}
                  onClick={() => void chooseTheme(option.value as ThemeMode)}
                >
                  {option.value === 'system' ? <MonitorCog size={14} /> : null}
                  {option.label}
                </button>
              ))}
            </div>
            <p className="settings-hint">Sift never asks Windows to change your system theme; it only changes its own window.</p>
          </Section>

          <Section icon={<MonitorCog size={16} />} title="Startup" description="Whether Sift is already running when you sign in to Windows.">
            <div className="settings-row">
              <div className="settings-row-copy">
                <strong>Start with Windows</strong>
                <span>{startWithWindows ? 'Sift opens in the background when you sign in.' : 'Sift only opens when you open it.'}</span>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={startWithWindows}
                aria-label="Start with Windows"
                className={`switch${startWithWindows ? ' on' : ''}`}
                disabled={!desktopAvailable || saving}
                onClick={() => void toggleStartWithWindows(!startWithWindows)}
              >
                <span className="switch-knob" />
              </button>
            </div>
            {!desktopAvailable && <p className="settings-hint">Starting with Windows is a Windows feature, so it only works in the desktop app.</p>}
            {startWithWindowsError && (
              <div className="permission-note" role="alert">
                <LockKeyhole size={15} />
                <span>{startWithWindowsError}</span>
              </div>
            )}
          </Section>

          <Section icon={<CalendarClock size={16} />} title="Scanning" description="When Sift walks your folders to keep its index up to date.">
            <div className="settings-row">
              <div className="settings-row-copy">
                <strong>Scan schedule</strong>
                <span>{scheduleHint}</span>
              </div>
              <select
                className="settings-select"
                aria-label="Scan schedule"
                value={scanSchedule}
                disabled={saving}
                onChange={(event) => void chooseSchedule(event.target.value as ScanSchedule)}
              >
                {schedules.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
              </select>
            </div>
            <p className="settings-hint">
              Last full scan {relativeTime(settings?.lastScanUnix ?? null)}.
              {settings?.nextScanUnix
                ? ` Next one is due ${relativeTime(settings.nextScanUnix)}.`
                : ' Sift watches for changes as you work, whatever the schedule.'}
            </p>
          </Section>

          <Section icon={<FolderX size={16} />} title="Folders Sift skips" description="Anything inside these folders is never indexed, searched, or counted in Clean.">
            <form className="exclusion-form" onSubmit={(event) => void submitExclusion(event)}>
              <input
                value={path}
                onChange={(event) => setPath(event.target.value)}
                placeholder="C:\Users\you\Videos\Recordings"
                aria-label="Full path of a folder to exclude"
                spellCheck={false}
                disabled={!desktopAvailable || saving}
              />
              <button type="submit" className="secondary-button" disabled={!desktopAvailable || saving || !path.trim()}>
                {saving ? <span className="button-spinner" /> : <Plus size={15} />} Exclude
              </button>
            </form>
            {exclusionError && (
              <div className="inline-error" role="alert"><TriangleAlert size={15} />{exclusionError}</div>
            )}
            {exclusions.length === 0 ? (
              <div className="settings-empty">
                <span className="settings-empty-icon"><FolderX size={17} /></span>
                <div>
                  <strong>No folders are excluded</strong>
                  <p>Sift indexes everything inside your own profile. Add a full path above to keep a folder out of the index.</p>
                </div>
              </div>
            ) : (
              <ul className="exclusion-list">
                <AnimatePresence initial={false}>
                  {exclusions.map((exclusion) => (
                    <motion.li
                      key={exclusion.path}
                      layout={!reducedMotion}
                      initial={{ opacity: 0, y: reducedMotion ? 0 : 6 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, x: reducedMotion ? 0 : 12 }}
                      transition={motionProps}
                    >
                      <div className="exclusion-copy">
                        <strong>{exclusion.label}</strong>
                        <span title={exclusion.path}>{exclusion.path}</span>
                      </div>
                      <button
                        type="button"
                        className="exclusion-remove"
                        aria-label={`Stop excluding ${exclusion.label}`}
                        title="Include this folder again"
                        disabled={saving}
                        onClick={() => void removeExclusion(exclusion.path)}
                      >
                        <X size={15} />
                      </button>
                    </motion.li>
                  ))}
                </AnimatePresence>
              </ul>
            )}
          </Section>

          <Section icon={<HardDrive size={16} />} title="Caches" description="What Sift keeps on disk that it can rebuild at any time.">
            {cacheError && (
              <div className="alert-banner error-banner" role="alert">
                <TriangleAlert size={16} />
                <span>{cacheError}</span>
                <button type="button" aria-label="Retry" onClick={() => void loadCache()}><RefreshCcw size={15} /></button>
              </div>
            )}
            {!cache && !cacheError ? (
              <div className="settings-empty" role="status">
                <span className="button-spinner" />
                <div><strong>Measuring…</strong><p>Sift is working out how much disk the thumbnail cache is using.</p></div>
              </div>
            ) : cache ? (
              <div className="cache-rows">
                <div className="settings-row">
                  <div className="settings-row-copy">
                    <strong>Skipped folders</strong>
                    <span>
                      {cache.skippedTotal === 0
                        ? 'The last scan read every folder it was allowed to.'
                        : `${cache.skippedTotal.toLocaleString()} folder${cache.skippedTotal === 1 ? '' : 's'} passed over, mostly for lack of permission.`}
                    </span>
                  </div>
                  <div className="settings-row-actions">
                    {skipped.length > 0 && (
                      <button type="button" className="text-button" onClick={() => setShowSkipped((value) => !value)}>
                        {showSkipped ? 'Hide list' : 'View list'}
                      </button>
                    )}
                    <button
                      type="button"
                      className="secondary-button"
                      aria-label="Clear the skipped folder list"
                      disabled={saving || cache.skippedTotal === 0}
                      onClick={() => void clearSkippedFolders()}
                    >
                      <Trash2 size={15} /> Clear
                    </button>
                  </div>
                </div>
                <AnimatePresence initial={false}>
                  {showSkipped && skipped.length > 0 && (
                    <motion.ul
                      className="skipped-list"
                      initial={{ opacity: 0, height: reducedMotion ? 'auto' : 0 }}
                      animate={{ opacity: 1, height: 'auto' }}
                      exit={{ opacity: 0, height: reducedMotion ? 'auto' : 0 }}
                      transition={motionProps}
                    >
                      {skipped.map((folder) => (
                        <li key={folder.path} className={folder.reason === PERMISSION_REASON ? 'is-denied' : undefined}>
                          <span className="skipped-reason">{folder.reason}</span>
                          <span className="skipped-path" title={folder.path}>{folder.path}</span>
                        </li>
                      ))}
                      {cache.skippedFoldersTruncated && (
                        <li className="skipped-more">Only the first {skipped.length.toLocaleString()} are listed.</li>
                      )}
                    </motion.ul>
                  )}
                </AnimatePresence>

                <div className="settings-row">
                  <div className="settings-row-copy">
                    <strong>Thumbnail cache</strong>
                    <span>
                      {readableSize(cache.thumbnailCacheBytes)} in {cache.thumbnailCacheFiles.toLocaleString()} image
                      {cache.thumbnailCacheFiles === 1 ? '' : 's'}. Clearing it costs nothing but a moment of re-thumbing.
                    </span>
                    <span className="settings-path" title={cache.thumbnailCachePath}>{cache.thumbnailCachePath}</span>
                  </div>
                  <div className="settings-row-actions">
                    <span className="cache-icon"><ImageIcon size={16} /></span>
                    <button
                      type="button"
                      className="secondary-button"
                      aria-label="Clear the thumbnail cache"
                      disabled={saving || cache.thumbnailCacheFiles === 0}
                      onClick={() => void clearThumbnailCache()}
                    >
                      <Trash2 size={15} /> Clear
                    </button>
                  </div>
                </div>
              </div>
            ) : null}
          </Section>

          <p className="settings-footnote">
            <Check size={14} />
            {isTauri()
              ? 'Settings are stored in Sift’s own database on this PC. Nothing here is sent anywhere.'
              : 'This is the browser preview, so only appearance and the scan schedule are kept, in this browser. Everything else needs the Windows app.'}
          </p>
        </motion.div>
      )}
    </div>
  );
}
