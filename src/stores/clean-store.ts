import { create } from 'zustand';
import {
  commands,
  type CleanCard,
  type CleanSummary,
  type DuplicateReport,
  type InstalledApp,
} from '../lib/bindings';

/** What the animated "You freed X" banner reports after a cleanup. */
export interface CleanResult {
  bytes: number;
  items: number;
  label: string;
  /** Set when some items could not be moved, so the number is explained. */
  detail: string | null;
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Sift could not finish that cleanup.';
}

function plural(count: number, word: string): string {
  return `${count.toLocaleString()} ${word}${count === 1 ? '' : 's'}`;
}

interface CleanStore {
  summary: CleanSummary | null;
  loading: boolean;
  /** The duplicate hash scan, which is far slower than the rest of the summary. */
  scanning: boolean;
  /** A cleanup is in flight. */
  busy: boolean;
  error: string | null;
  notice: string | null;
  duplicates: DuplicateReport | null;
  result: CleanResult | null;
  load: () => Promise<void>;
  scanDuplicates: () => Promise<void>;
  cleanPaths: (paths: string[], label: string) => Promise<void>;
  cleanJunk: (groupIds: string[], expectedBytes: number) => Promise<void>;
  uninstall: (app: InstalledApp) => Promise<void>;
  clearResult: () => void;
  setNotice: (notice: string | null) => void;
  clearError: () => void;
}

export const useCleanStore = create<CleanStore>((set, get) => ({
  summary: null,
  loading: false,
  scanning: false,
  busy: false,
  error: null,
  notice: null,
  duplicates: null,
  result: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      const summary = await commands.getCleanSummary();
      set({ summary });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ loading: false });
    }
  },

  scanDuplicates: async () => {
    set({ scanning: true, error: null });
    try {
      const duplicates = await commands.scanDuplicates();
      set({ duplicates });
      set({ notice: `${plural(duplicates.sets.length, 'duplicate set')} found.` });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ scanning: false });
    }
  },

  cleanPaths: async (paths, label) => {
    if (paths.length === 0) return;
    set({ busy: true, error: null, notice: null, result: null });
    try {
      const result = await commands.cleanPaths(paths);
      const skipped = result.skipped;
      set({
        result: {
          bytes: result.freedBytes,
          items: result.moved,
          label,
          detail:
            skipped > 0
              ? `${plural(skipped, 'item')} could not be moved to the Recycle Bin.`
              : result.cancelled
                ? 'Stopped early. Everything already moved is in the Recycle Bin.'
                : null,
        },
      });
      // The cards size themselves from the index, so re-read it after a cleanup.
      await get().load();
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ busy: false });
    }
  },

  cleanJunk: async (groupIds, expectedBytes) => {
    if (groupIds.length === 0) return;
    set({ busy: true, error: null, notice: null, result: null });
    try {
      const result = await commands.cleanJunk(groupIds, expectedBytes);
      set({
        result: {
          bytes: result.freedBytes,
          items: result.deleted,
          label: 'Junk files',
          detail:
            result.skipped > 0
              ? `${plural(result.skipped, 'file')} are in use and were skipped.`
              : result.cancelled
                ? 'Stopped early. Everything already moved is in the Recycle Bin.'
                : null,
        },
      });
      await get().load();
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ busy: false });
    }
  },

  uninstall: async (app) => {
    if (!app.uninstallCommand) {
      set({ error: `${app.name} has no uninstaller registered.` });
      return;
    }
    set({ busy: true, error: null, notice: null });
    try {
      await commands.uninstallApp(app.uninstallCommand);
      set({ notice: `${app.name}'s uninstaller is starting.` });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ busy: false });
    }
  },

  clearResult: () => set({ result: null }),
  setNotice: (notice) => set({ notice }),
  clearError: () => set({ error: null }),
}));

export function cardById(summary: CleanSummary | null, id: string): CleanCard | null {
  return summary?.cards.find((card) => card.id === id) ?? null;
}

/** Total space the six cards could give back, excluding the apps card (uninstalling is
 * the app's own business, not a Recycle Bin move). */
export function totalReclaimable(summary: CleanSummary | null, duplicates: DuplicateReport | null): number {
  const cards = summary?.cards.filter((card) => card.id !== 'unused-apps') ?? [];
  const fromDuplicates = duplicates?.reclaimableBytes ?? 0;
  return cards.reduce((total, card) => total + card.reclaimableBytes, 0) + fromDuplicates;
}
