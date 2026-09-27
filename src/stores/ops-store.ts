import { create } from 'zustand';
import {
  commands,
  type ConflictAction,
  type ConflictDecision,
  type DeleteResult,
  type FavoriteItem,
  type OperationProgress,
  type RecentItem,
  type TransferPlan,
  type TransferResult,
} from '../lib/bindings';

export type ClipboardMode = 'copy' | 'cut';

export interface ClipboardState {
  mode: ClipboardMode;
  paths: string[];
  names: string[];
}

export interface ConflictRequest {
  kind: 'copy' | 'move';
  sources: string[];
  destination: string;
  plan: TransferPlan;
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Sift could not finish that action.';
}

function summarize(result: TransferResult, kind: 'copy' | 'move'): string {
  const verb = kind === 'copy' ? 'Copied' : 'Moved';
  const parts = [`${verb} ${result.completed} item${result.completed === 1 ? '' : 's'}`];
  if (result.skipped > 0) parts.push(`${result.skipped} skipped`);
  if (result.failed > 0) parts.push(`${result.failed} failed`);
  if (result.cancelled) parts.push('stopped early');
  return `${parts.join(' · ')}.`;
}

interface OpsStore {
  operations: OperationProgress[];
  clipboard: ClipboardState | null;
  conflict: ConflictRequest | null;
  busy: boolean;
  error: string | null;
  notice: string | null;
  favorites: FavoriteItem[];
  recents: RecentItem[];
  /** Set by the Browse route so the listing refreshes after a filesystem change. */
  refresh: (() => void) | null;
  setRefresh: (refresh: (() => void) | null) => void;
  acceptProgress: (progress: OperationProgress) => void;
  refreshOperations: () => Promise<void>;
  cancel: (jobId: string) => Promise<void>;
  copyToClipboard: (paths: string[], names: string[], mode: ClipboardMode) => void;
  clearClipboard: () => void;
  pasteInto: (destination: string) => Promise<void>;
  resolveConflict: (decisions: ConflictDecision[], defaultAction: ConflictAction) => Promise<void>;
  dismissConflict: () => void;
  deletePaths: (paths: string[]) => Promise<DeleteResult | null>;
  renamePath: (path: string, newName: string, replace?: boolean) => Promise<boolean>;
  loadFavorites: () => Promise<void>;
  toggleFavorite: (path: string, name: string, isDirectory: boolean) => Promise<void>;
  loadRecents: () => Promise<void>;
  clearRecents: () => Promise<void>;
  setNotice: (notice: string | null) => void;
  clearError: () => void;
}

export const useOpsStore = create<OpsStore>((set, get) => ({
  operations: [],
  clipboard: null,
  conflict: null,
  busy: false,
  error: null,
  notice: null,
  favorites: [],
  recents: [],
  refresh: null,
  setRefresh: (refresh) => set({ refresh }),

  acceptProgress: (progress) => {
    const operations = get().operations.filter((operation) => operation.jobId !== progress.jobId);
    if (progress.state === 'running') operations.push(progress);
    set({ operations });
  },

  refreshOperations: async () => {
    try {
      const operations = await commands.listOperations();
      set({ operations });
    } catch {
      // A missing job table is not worth interrupting the user for.
    }
  },

  cancel: async (jobId) => {
    try {
      await commands.cancelOperation(jobId);
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },

  copyToClipboard: (paths, names, mode) => {
    if (paths.length === 0) return;
    set({ clipboard: { mode, paths, names }, notice: null });
  },

  clearClipboard: () => set({ clipboard: null }),

  pasteInto: async (destination) => {
    const clipboard = get().clipboard;
    if (!clipboard || clipboard.paths.length === 0) {
      set({ notice: 'Copy or cut something first.' });
      return;
    }
    const kind = clipboard.mode === 'cut' ? 'move' : 'copy';
    set({ busy: true, error: null });
    try {
      const plan = await commands.planTransfer(clipboard.paths, destination, kind);
      if (plan.conflicts.length > 0) {
        set({ conflict: { kind, sources: clipboard.paths, destination, plan }, busy: false });
        return;
      }
      await runTransfer(get, set, kind, clipboard.paths, destination, [], 'keepBoth');
      if (kind === 'move') set({ clipboard: null });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    } finally {
      set({ busy: false });
    }
  },

  resolveConflict: async (decisions, defaultAction) => {
    const conflict = get().conflict;
    if (!conflict) return;
    set({ busy: true, error: null });
    try {
      await runTransfer(get, set, conflict.kind, conflict.sources, conflict.destination, decisions, defaultAction);
      set({ conflict: null, clipboard: conflict.kind === 'move' ? null : get().clipboard });
    } catch (error: unknown) {
      set({ error: errorText(error), conflict: null });
    } finally {
      set({ busy: false });
    }
  },

  dismissConflict: () => set({ conflict: null, busy: false }),

  deletePaths: async (paths) => {
    if (paths.length === 0) return null;
    set({ busy: true, error: null });
    try {
      const result = await commands.deletePaths(paths);
      set({
        notice: result.cancelled
          ? 'Delete stopped.'
          : `${result.moved} item${result.moved === 1 ? '' : 's'} moved to the Recycle Bin.`,
        clipboard: null,
      });
      get().refresh?.();
      void get().loadFavorites();
      void get().loadRecents();
      return result;
    } catch (error: unknown) {
      set({ error: errorText(error) });
      return null;
    } finally {
      set({ busy: false });
    }
  },

  renamePath: async (path, newName, replace = false) => {
    set({ busy: true, error: null });
    try {
      const outcome = await commands.renamePath(path, newName, replace);
      set({ notice: `Renamed to ${outcome.newPath.split(/[\\/]/).filter(Boolean).pop() ?? newName}.` });
      get().refresh?.();
      void get().loadFavorites();
      void get().loadRecents();
      return true;
    } catch (error: unknown) {
      set({ error: errorText(error) });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  loadFavorites: async () => {
    try {
      const favorites = await commands.listFavorites();
      set({ favorites });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },

  toggleFavorite: async (path, name, isDirectory) => {
    const pinned = get().favorites.some((favorite) => favorite.path.toLowerCase() === path.toLowerCase());
    try {
      const favorites = pinned ? await commands.removeFavorite(path) : await commands.addFavorite(path, name, isDirectory);
      set({ favorites, notice: pinned ? `Removed ${name} from Favorites.` : `Added ${name} to Favorites.` });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },

  loadRecents: async () => {
    try {
      const recents = await commands.listRecents(12);
      set({ recents });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },

  clearRecents: async () => {
    try {
      await commands.clearRecents();
      set({ recents: [], notice: 'Recent files cleared.' });
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },

  setNotice: (notice) => set({ notice }),
  clearError: () => set({ error: null }),
}));

type SetState = (partial: Partial<OpsStore>) => void;
type GetState = () => OpsStore;

async function runTransfer(
  get: GetState,
  set: SetState,
  kind: 'copy' | 'move',
  sources: string[],
  destination: string,
  decisions: ConflictDecision[],
  defaultAction: ConflictAction,
): Promise<TransferResult> {
  const result = kind === 'copy'
    ? await commands.copyPaths(sources, destination, decisions, defaultAction)
    : await commands.movePaths(sources, destination, decisions, defaultAction);
  set({ notice: summarize(result, kind) });
  get().refresh?.();
  void get().loadRecents();
  if (result.errors.length > 0) set({ error: result.errors[0] ?? null });
  return result;
}
