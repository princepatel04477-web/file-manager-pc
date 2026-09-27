import { create } from 'zustand';
import { commands, type CategorySummary, type DriveStorage, type HomeLocation, type IndexProgress, type IndexedEntry, type SearchFilter } from '../lib/bindings';

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'The index is temporarily unavailable.';
}

let searchSequence = 0;

interface IndexStore {
  progress: IndexProgress | null;
  categories: CategorySummary[];
  drives: DriveStorage[];
  locations: HomeLocation[];
  entries: IndexedEntry[];
  activePath: string | null;
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  searching: boolean;
  error: string | null;
  initialize: () => Promise<void>;
  acceptProgress: (progress: IndexProgress) => void;
  loadDirectory: (path: string, sort?: string, descending?: boolean, offset?: number) => Promise<void>;
  search: (filter: SearchFilter) => Promise<void>;
  clearSearch: () => void;
}

export const useIndexStore = create<IndexStore>((set, get) => ({
  progress: null,
  categories: [],
  drives: [],
  locations: [],
  entries: [],
  activePath: null,
  loading: false,
  loadingMore: false,
  hasMore: false,
  searching: false,
  error: null,
  initialize: async () => {
    set({ error: null });
    try {
      const [progress, categories, drives, locations] = await Promise.all([
        commands.getIndexStatus(),
        commands.getCategorySummary(),
        commands.getDriveStorage(),
        commands.listHomeLocations(),
      ]);
      const home = locations.find((location) => location.id === 'home');
      set({ progress, categories, drives, locations, activePath: home?.path ?? null });
      if (home) await get().loadDirectory(home.path);
    } catch (error: unknown) {
      set({ error: errorText(error) });
    }
  },
  acceptProgress: (progress) => {
    const previous = get().progress;
    set({ progress });
    if (progress.complete && !progress.scanning && (!previous || previous.scanning || !previous.complete)) {
      void Promise.all([commands.getCategorySummary(), commands.getDriveStorage()])
        .then(([categories, drives]) => set({ categories, drives }))
        .catch((error: unknown) => set({ error: errorText(error) }));
    }
  },
  loadDirectory: async (path, sort = 'name', descending = false, offset = 0) => {
    const sequence = ++searchSequence;
    set({ loading: offset === 0, loadingMore: offset > 0, searching: false, error: null, activePath: path, ...(offset === 0 ? { entries: [], hasMore: false } : {}) });
    try {
      const entries = await commands.listIndexDirectory(path, sort, descending, 1_000, offset);
      if (sequence === searchSequence) set((state) => ({ entries: offset === 0 ? entries : [...state.entries, ...entries], hasMore: entries.length === 1_000 }));
    } catch (error: unknown) {
      if (sequence === searchSequence) set({ ...(offset === 0 ? { entries: [] } : {}), error: errorText(error) });
    } finally {
      if (sequence === searchSequence) set({ loading: false, loadingMore: false });
    }
  },
  search: async (filter) => {
    const sequence = ++searchSequence;
    set({ searching: true, loading: false, loadingMore: false, error: null });
    try {
      const entries = await commands.searchIndex(filter);
      if (sequence === searchSequence) set({ entries, searching: false, hasMore: false });
    } catch (error: unknown) {
      if (sequence === searchSequence) set({ error: errorText(error), searching: false });
    }
  },
  clearSearch: () => {
    searchSequence += 1;
    set({ searching: false, entries: [], error: null });
  },
}));
