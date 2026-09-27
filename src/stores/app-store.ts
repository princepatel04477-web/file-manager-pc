import { create } from 'zustand';
import { commands, type CleanReport, type DirectoryListing, type HomeLocation, type SearchResults, type TrashResult } from '../lib/bindings';

function messageFrom(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Sift could not complete that action. Please try again.';
}

let searchGeneration = 0;

interface AppState {
  locations: HomeLocation[];
  listing: DirectoryListing | null;
  cleanReport: CleanReport | null;
  searchResults: SearchResults | null;
  selectedPaths: string[];
  currentPath: string | null;
  loading: boolean;
  scanning: boolean;
  searching: boolean;
  error: string | null;
  notice: string | null;
  initialize: () => Promise<void>;
  openDirectory: (path: string) => Promise<void>;
  runScan: () => Promise<void>;
  runSearch: (query: string) => Promise<void>;
  toggleSelected: (path: string) => void;
  clearSelection: () => void;
  moveToRecycleBin: (paths: string[]) => Promise<TrashResult | null>;
  openFile: (path: string) => Promise<void>;
  setNotice: (notice: string | null) => void;
  clearError: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  locations: [],
  listing: null,
  cleanReport: null,
  searchResults: null,
  selectedPaths: [],
  currentPath: null,
  loading: false,
  scanning: false,
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
  runScan: async () => {
    set({ scanning: true, error: null });
    try {
      const cleanReport = await commands.scanStorage();
      set({ cleanReport });
    } catch (error: unknown) {
      set({ error: messageFrom(error) });
    } finally {
      set({ scanning: false });
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
  toggleSelected: (path) => {
    const current = get().selectedPaths;
    set({ selectedPaths: current.includes(path) ? current.filter((item) => item !== path) : [...current, path] });
  },
  clearSelection: () => set({ selectedPaths: [] }),
  moveToRecycleBin: async (paths) => {
    if (paths.length === 0) return null;
    set({ loading: true, error: null });
    try {
      const result = await commands.trashPaths(paths);
      set({ selectedPaths: [], notice: result.moved > 0 ? `${result.moved} item${result.moved === 1 ? '' : 's'} moved to the Recycle Bin.` : null });
      const currentPath = get().currentPath;
      if (currentPath) await get().openDirectory(currentPath);
      const nextReport = get().cleanReport;
      if (nextReport) set({ cleanReport: null });
      return result;
    } catch (error: unknown) {
      set({ error: messageFrom(error) });
      return null;
    } finally {
      set({ loading: false });
    }
  },
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
