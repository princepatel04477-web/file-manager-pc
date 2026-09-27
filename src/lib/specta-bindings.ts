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

export interface CleanItem {
  path: string;
  name: string;
  size: number;
  modifiedUnix: number | null;
  daysOld: number | null;
  isCloud: boolean;
}

/** A selectable chunk of the Junk card: one junk location, or the Recycle Bin. */
export interface CleanGroup {
  id: string;
  label: string;
  itemCount: number;
  reclaimableBytes: number;
}

export type CardAction = "deletePaths" | "deleteJunk" | "emptyRecycleBin" | "uninstall" | "unavailable";

export interface CleanCard {
  id: string;
  title: string;
  description: string;
  action: CardAction;
  itemCount: number;
  reclaimableBytes: number;
  groups: CleanGroup[];
  items: CleanItem[];
  truncated: boolean;
  skipped: number;
  ready: boolean;
}

export interface RecycleBinInfo {
  bytes: number;
  items: number;
  available: boolean;
}

export interface UninstallCommand {
  executable: string;
  arguments: string[];
}

export type AppSource = "machine64" | "machine32" | "currentUser";

export interface InstalledApp {
  name: string;
  publisher: string;
  version: string;
  installDate: string | null;
  sizeBytes: number;
  uninstallCommand: UninstallCommand | null;
  perUser: boolean;
  source: AppSource;
}

export interface CleanSummary {
  cards: CleanCard[];
  recycleBin: RecycleBinInfo;
  apps: InstalledApp[];
  appsBytes: number;
  indexedFiles: number;
  generatedAtUnix: number;
}

export interface JunkDeleteResult {
  freedBytes: number;
  deleted: number;
  skipped: number;
  cancelled: boolean;
  errors: string[];
}

export interface CleanDeleteResult {
  freedBytes: number;
  moved: number;
  skipped: number;
  cancelled: boolean;
  errors: string[];
}

export interface DuplicateFile {
  path: string;
  name: string;
  size: number;
  modifiedUnix: number | null;
  /** True for the copy Sift protects by default. */
  original: boolean;
}

export interface DuplicateSet {
  fingerprint: string;
  size: number;
  files: DuplicateFile[];
  reclaimableBytes: number;
}

export interface DuplicateReport {
  sets: DuplicateSet[];
  reclaimableBytes: number;
  candidates: number;
  hashed: number;
  skipped: number;
}

export interface SearchResults {
  entries: FileEntry[];
  scanned: number;
  skipped: number;
  truncated: boolean;
}

export interface FavoriteItem {
  path: string;
  name: string;
  isDirectory: boolean;
  addedAtUnix: number;
}

export interface RecentItem {
  path: string;
  name: string;
  openedAtUnix: number;
}

export interface FileProperties {
  name: string;
  path: string;
  parent: string;
  extension: string;
  kind: string;
  isDirectory: boolean;
  isCloudPlaceholder: boolean;
  isHidden: boolean;
  size: number;
  childCount: number | null;
  modifiedUnix: number | null;
  createdUnix: number | null;
  accessedUnix: number | null;
  drive: string;
  attributeLabels: string[];
  favorite: boolean;
}

export interface TextPreview {
  path: string;
  content: string;
  truncated: boolean;
  size: number;
  encoding: string;
}

export interface Thumbnail {
  dataUrl: string;
  key: string;
  fromCache: boolean;
}

export type ThumbnailKind = "image" | "video" | "document";

export type ConflictAction = "replace" | "skip" | "keepBoth" | "ask";

export interface ConflictDecision {
  source: string;
  action: ConflictAction;
}

export type PlannedAction = "create" | "replace" | "skip" | "conflict";

export interface PlannedItem {
  source: string;
  destination: string;
  name: string;
  isDirectory: boolean;
  size: number;
  action: PlannedAction;
}

export interface PlannedConflict {
  source: string;
  destination: string;
  name: string;
  sourceIsDirectory: boolean;
  sourceSize: number;
  sourceModifiedUnix: number | null;
  existingIsDirectory: boolean;
  existingSize: number;
  existingModifiedUnix: number | null;
}

export type BlockedReason =
  | "outsideUserFiles"
  | "missing"
  | "reparsePoint"
  | "cloudOnly"
  | "sameItem"
  | "insideItself"
  | "invalidName"
  | "destinationUnavailable";

export interface BlockedItem {
  source: string;
  reason: BlockedReason;
}

export interface TransferPlan {
  kind: string;
  destination: string;
  items: PlannedItem[];
  conflicts: PlannedConflict[];
  blocked: BlockedItem[];
  bytesTotal: number;
}

export interface TransferResult {
  completed: number;
  skipped: number;
  failed: number;
  bytes: number;
  cancelled: boolean;
  destinations: string[];
  errors: string[];
}

export interface RenameOutcome {
  previousPath: string;
  newPath: string;
  replaced: boolean;
}

export interface DeleteResult {
  moved: number;
  movedBytes: number;
  skipped: number;
  cancelled: boolean;
  errors: string[];
}

export type OpsKind = "copy" | "move" | "rename" | "delete" | "scan" | "clean";

export type OpsState = "running" | "completed" | "cancelled" | "failed";

export interface OperationProgress {
  jobId: string;
  kind: OpsKind;
  state: OpsState;
  itemsTotal: number;
  itemsDone: number;
  itemsSkipped: number;
  bytesTotal: number;
  bytesDone: number;
  current: string;
  destination: string;
  startedUnix: number;
  finishedUnix: number | null;
  error: string | null;
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

export interface NearbyShare {
  id: string;
  deviceName: string;
  fileName: string;
  fileSize: number;
  host: string;
  port: number;
  token: string;
}

export interface PcShareSession {
  deviceName: string;
  fileName: string;
  fileSize: number;
  pairingCode: string;
  expiresInSeconds: number;
}
