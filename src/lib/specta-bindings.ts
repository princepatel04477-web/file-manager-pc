// Generated from src-tauri/src/lib.rs with tauri-specta. This checked-in export keeps
// strict frontend builds available before the first Windows desktop startup.
export interface HomeLocation {
  id: string;
  label: string;
  path: string;
  category: string;
}

export interface FileEntry {
  name: string;
  path: string;
  extension: string;
  kind: string;
  isDirectory: boolean;
  isCloudPlaceholder: boolean;
  size: number;
  modifiedUnix: number | null;
}

export interface DirectoryListing {
  path: string;
  label: string;
  parentPath: string | null;
  entries: FileEntry[];
  skipped: number;
}

export interface DuplicateGroup {
  fingerprint: string;
  size: number;
  files: FileEntry[];
  reclaimableBytes: number;
}

export interface CleanReport {
  scannedFiles: number;
  skipped: number;
  largeFiles: FileEntry[];
  duplicateGroups: DuplicateGroup[];
  reclaimableBytes: number;
  scannedAtUnix: number;
}

export interface SearchResults {
  entries: FileEntry[];
  scanned: number;
  skipped: number;
  truncated: boolean;
}

export interface TrashResult {
  moved: number;
  skipped: number;
}

export interface IndexedEntry {
  id: number;
  path: string;
  parentPath: string;
  name: string;
  ext: string;
  category: string;
  size: number;
  mtime: number | null;
  ctime: number | null;
  isHidden: boolean;
  isCloud: boolean;
  isDirectory: boolean;
  drive: string;
}

export interface CategorySummary {
  category: string;
  count: number;
  totalSize: number;
}

export interface IndexCounts {
  indexedFiles: number;
  indexedDirectories: number;
}

export interface SearchFilter {
  query: string;
  category: string | null;
  minSize: number | null;
  maxSize: number | null;
  modifiedAfter: number | null;
  modifiedBefore: number | null;
  limit: number | null;
}

export interface IndexProgress {
  filesScanned: number;
  skipped: number;
  currentDir: string;
  drive: string;
  drives: string[];
  scanning: boolean;
  complete: boolean;
  watching: boolean;
  error: string | null;
}

export interface DriveStorage {
  drive: string;
  root: string;
  used: number;
  free: number;
  total: number;
  indexedSize: number;
}

export interface ShareLink {
  url: string;
  qrSvg: string;
  fileName: string;
  expiresInSeconds: number;
}
