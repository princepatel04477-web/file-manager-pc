import { invoke } from '@tauri-apps/api/core';
import type { CleanReport, DirectoryListing, DriveStorage, HomeLocation, IndexProgress, IndexedEntry, SearchFilter, SearchResults, ShareLink, TrashResult, CategorySummary } from './specta-bindings';

export type {
  CategorySummary,
  CleanReport,
  DirectoryListing,
  DriveStorage,
  DuplicateGroup,
  IndexProgress,
  IndexedEntry,
  FileEntry,
  HomeLocation,
  SearchFilter,
  SearchResults,
  ShareLink,
  TrashResult,
} from './specta-bindings';

/** The only frontend boundary to native filesystem and sharing commands. */
export const commands = {
  listHomeLocations: (): Promise<HomeLocation[]> => invoke('list_home_locations'),
  listDirectory: (path: string): Promise<DirectoryListing> => invoke('list_directory', { path }),
  scanStorage: (): Promise<CleanReport> => invoke('scan_storage'),
  searchFiles: (query: string): Promise<SearchResults> => invoke('search_files', { query }),
  trashPaths: (paths: string[]): Promise<TrashResult> => invoke('trash_paths', { paths }),
  openFile: (path: string): Promise<void> => invoke('open_file', { path }),
  startShare: (path: string): Promise<ShareLink> => invoke('start_share', { path }),
  stopShare: (): Promise<void> => invoke('stop_share'),
  getIndexStatus: (): Promise<IndexProgress> => invoke('get_index_status'),
  getCategorySummary: (): Promise<CategorySummary[]> => invoke('get_category_summary'),
  getDriveStorage: (): Promise<DriveStorage[]> => invoke('get_drive_storage'),
  listIndexDirectory: (path: string, sort: string, descending: boolean, limit: number, offset: number): Promise<IndexedEntry[]> =>
    invoke('list_index_directory', { path, sort, descending, limit, offset }),
  searchIndex: (filter: SearchFilter): Promise<IndexedEntry[]> => invoke('search_index', { filter }),
};
