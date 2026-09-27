import { create } from 'zustand';
import { commands, type DirectoryListing, type HomeLocation, type SearchResults } from '../lib/bindings';

function messageFrom(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Sift could not complete that action. Please try again.';
}

let searchGeneration = 0;

export interface SelectionOptions {
  /** Ctrl/Cmd-click: toggle this path without disturbing the rest. */
  additive?: boolean;
  /** Shift-click: select everything between the anchor and this path. */
  range?: boolean;
  /** The current listing order, used to expand a shift-click range. */
  order?: string[];
}

interface AppState {
  locations: HomeLocation[];
  listing: DirectoryListing | null;
  searchResults: SearchResults | null;
  selectedPaths: string[];
  anchorPath: string | null;
  currentPath: string | null;
  loading: boolean;
  searching: boolean;
  error: string | null;
  notice: string | null;
  initialize: () => Promise<void>;
  openDirectory: (path: string) => Promise<void>;
  runSearch: (query: string) => Promise<void>;
  toggleSelected: (path: string, options?: SelectionOptions) => void;
  setSelectedPaths: (paths: string[]) => void;
  clearSelection: () => void;
  openFile: (path: string) => Promise<void>;
  setNotice: (notice: string | null) => void;
  clearError: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  locations: [],
  listing: null,
  searchResults: null,
  selectedPaths: [],
  anchorPath: null,
  currentPath: null,
  loading: false,
  searching: false,
  error: null,
  notice: null,
  initialize: async () => {
    set({ loading: true, error: null });
    try {
      const locations = await commands.listHomeLocations();
      set({ locations });
      const home = locations.find((location) => location.id === 'home');
      if (home) await get().openDirectory(home.path);
    } catch (error: unknown) {
      set({ error: messageFrom(error) });
    } finally {
      set({ loading: false });
    }
  },
  openDirectory: async (path) => {
    set({ loading: true, error: null, listing: null, searchResults: null, selectedPaths: [] });
    try {
      const listing = await commands.listDirectory(path);
      set({ listing, currentPath: listing.path });
    } catch (error: unknown) {
      set({ listing: null, error: messageFrom(error) });
    } finally {
      set({ loading: false });
    }
  },
  runSearch: async (query) => {
    const generation = ++searchGeneration;
    if (!query.trim()) {
      set({ searchResults: null, error: null, searching: false });
      return;
    }
    set({ error: null, searchResults: null, searching: true });
    try {
      const searchResults = await commands.searchFiles(query);
      if (generation === searchGeneration) set({ searchResults, searching: false });
    } catch (error: unknown) {
      if (generation === searchGeneration) set({ error: messageFrom(error), searching: false });
    }
  },
  toggleSelected: (path, options) => {
    const current = get().selectedPaths;
    const anchor = get().anchorPath;
    if (options?.range && anchor && options.order) {
      const from = options.order.indexOf(anchor);
      const to = options.order.indexOf(path);
      if (from >= 0 && to >= 0) {
        const [start, end] = from <= to ? [from, to] : [to, from];
        set({ selectedPaths: options.order.slice(start, end + 1) });
        return;
      }
    }
    if (options?.additive) {
      set({
        selectedPaths: current.includes(path) ? current.filter((item) => item !== path) : [...current, path],
        anchorPath: path,
      });
      return;
    }
    set({ selectedPaths: [path], anchorPath: path });
  },
  setSelectedPaths: (paths) => set({ selectedPaths: paths }),
  clearSelection: () => set({ selectedPaths: [], anchorPath: null }),
  openFile: async (path) => {
    set({ error: null });
    try {
      await commands.openFile(path);
    } catch (error: unknown) {
      set({ error: messageFrom(error) });
    }
  },
  setNotice: (notice) => set({ notice }),
  clearError: () => set({ error: null }),
}));
