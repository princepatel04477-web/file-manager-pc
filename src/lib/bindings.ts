import { invoke } from '@tauri-apps/api/core';
import type {
  CategorySummary,
  CleanReport,
  ConflictAction,
  ConflictDecision,
  DeleteResult,
  DirectoryListing,
  DriveStorage,
  FavoriteItem,
  FileProperties,
  HomeLocation,
  IndexedEntry,
  IndexProgress,
  OperationProgress,
  RecentItem,
  RenameOutcome,
  SearchFilter,
  SearchResults,
  ShareLink,
  TextPreview,
  Thumbnail,
  TransferPlan,
  TransferResult,
} from './specta-bindings';

export type {
  BlockedItem,
  BlockedReason,
  CategorySummary,
  CleanReport,
  ConflictAction,
  ConflictDecision,
  DeleteResult,
  DirectoryListing,
  DriveStorage,
  DuplicateGroup,
  FavoriteItem,
  FileEntry,
  FileProperties,
  HomeLocation,
  IndexedEntry,
  IndexCounts,
  IndexProgress,
  OperationProgress,
  OpsKind,
  OpsState,
  PlannedAction,
  PlannedConflict,
  PlannedItem,
  RecentItem,
  RenameOutcome,
  SearchFilter,
  SearchResults,
  ShareLink,
  TextPreview,
  Thumbnail,
  ThumbnailKind,
  TransferPlan,
  TransferResult,
} from './specta-bindings';

/** The only frontend boundary to native filesystem, thumbnail, and sharing commands. */
export const commands = {
  listHomeLocations: (): Promise<HomeLocation[]> => invoke('list_home_locations'),
  listDirectory: (path: string): Promise<DirectoryListing> => invoke('list_directory', { path }),
  scanStorage: (): Promise<CleanReport> => invoke('scan_storage'),
  searchFiles: (query: string): Promise<SearchResults> => invoke('search_files', { query }),
  openFile: (path: string): Promise<void> => invoke('open_file', { path }),
  startShare: (path: string): Promise<ShareLink> => invoke('start_share', { path }),
  stopShare: (): Promise<void> => invoke('stop_share'),
  getIndexStatus: (): Promise<IndexProgress> => invoke('get_index_status'),
  getCategorySummary: (): Promise<CategorySummary[]> => invoke('get_category_summary'),
  getDriveStorage: (): Promise<DriveStorage[]> => invoke('get_drive_storage'),
  listIndexDirectory: (path: string, sort: string, descending: boolean, limit: number, offset: number): Promise<IndexedEntry[]> =>
    invoke('list_index_directory', { path, sort, descending, limit, offset }),
  searchIndex: (filter: SearchFilter): Promise<IndexedEntry[]> => invoke('search_index', { filter }),

  /** Ask what a copy or move would do, including every name collision. */
  planTransfer: (paths: string[], destination: string, kind: 'copy' | 'move'): Promise<TransferPlan> =>
    invoke('plan_transfer', { paths, destination, kind }),
  copyPaths: (paths: string[], destination: string, decisions: ConflictDecision[], defaultAction: ConflictAction): Promise<TransferResult> =>
    invoke('copy_paths', { paths, destination, decisions, defaultAction }),
  movePaths: (paths: string[], destination: string, decisions: ConflictDecision[], defaultAction: ConflictAction): Promise<TransferResult> =>
    invoke('move_paths', { paths, destination, decisions, defaultAction }),
  renamePath: (path: string, newName: string, replace = false): Promise<RenameOutcome> =>
    invoke('rename_path', { path, newName, replace }),
  deletePaths: (paths: string[]): Promise<DeleteResult> => invoke('delete_paths', { paths }),
  cancelOperation: (jobId: string): Promise<boolean> => invoke('cancel_operation', { jobId }),
  listOperations: (): Promise<OperationProgress[]> => invoke('list_operations'),

  revealInExplorer: (path: string): Promise<void> => invoke('reveal_in_explorer', { path }),
  openWith: (path: string): Promise<void> => invoke('open_with', { path }),
  showProperties: (path: string): Promise<void> => invoke('show_properties', { path }),

  getThumbnail: (path: string): Promise<Thumbnail> => invoke('get_thumbnail', { path }),
  describePath: (path: string): Promise<FileProperties> => invoke('describe_path', { path }),
  readTextPreview: (path: string): Promise<TextPreview> => invoke('read_text_preview', { path }),
  recordRecent: (path: string, name: string): Promise<void> => invoke('record_recent', { path, name }),

  listFavorites: (): Promise<FavoriteItem[]> => invoke('list_favorites'),
  addFavorite: (path: string, name: string, isDirectory: boolean): Promise<FavoriteItem[]> =>
    invoke('add_favorite', { path, name, isDirectory }),
  removeFavorite: (path: string): Promise<FavoriteItem[]> => invoke('remove_favorite', { path }),
  listRecents: (limit = 20): Promise<RecentItem[]> => invoke('list_recents', { limit }),
  clearRecents: (): Promise<void> => invoke('clear_recents'),
};
