import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  Archive, AudioLines, ArrowDownWideNarrow, ArrowUpWideNarrow, Check, ChevronRight, Clipboard, ClipboardPaste,
  Copy, ExternalLink, Eye, FileImage, FileText, Film, Folder, Grid2X2, HardDrive, Info, List, Pencil,
  Scissors, Search, Send, ShieldCheck, Sparkles, Star, Trash2, X,
} from 'lucide-react';
import type { CategorySummary, ConflictAction, ConflictDecision, DriveStorage, FileEntry, IndexedEntry } from '../lib/bindings';
import { commands } from '../lib/bindings';
import { previewKindFor } from '../lib/preview';
import { useAppStore } from '../stores/app-store';
import { useIndexStore } from '../stores/index-store';
import { useOpsStore } from '../stores/ops-store';
import { useMarquee } from '../hooks/useMarquee';
import { ConflictDialog } from '../components/ConflictDialog';
import { ContextMenu, type ContextMenuItem } from '../components/ContextMenu';
import { EntryThumb, FileGlyph, FileList, readableSize, type SelectionModifiers } from '../components/FileList';
import { PreviewDialog } from '../components/PreviewDialog';
import { PropertiesDialog } from '../components/PropertiesDialog';
import { RenameDialog } from '../components/RenameDialog';

const categoryDetails: Record<string, { label: string; color: string; icon: typeof FileImage }> = {
  images: { label: 'Images', color: 'images', icon: FileImage },
  videos: { label: 'Videos', color: 'videos', icon: Film },
  audio: { label: 'Audio', color: 'audio', icon: AudioLines },
  documents: { label: 'Documents', color: 'documents', icon: FileText },
  archives: { label: 'Archives', color: 'archives', icon: Archive },
  installers: { label: 'Installers', color: 'installers', icon: HardDrive },
  other: { label: 'Other', color: 'other', icon: Folder },
};

interface BrowseRouteProps {
  desktopAvailable: boolean;
  query: string;
  onQueryChange: (query: string) => void;
  onShareRequest?: (entry: FileEntry) => void;
  /** Opens the skipped-folder list, so "N skipped" is never a dead end. */
  onOpenSettings?: () => void;
}

interface MenuState {
  x: number;
  y: number;
  entry: FileEntry | null;
}

function toFileEntry(entry: IndexedEntry): FileEntry {
  const kinds: Record<string, string> = {
    images: 'image', videos: 'video', audio: 'audio', documents: 'document', archives: 'archive', installers: 'file', other: 'file',
  };
  return {
    name: entry.name,
    path: entry.path,
    extension: entry.ext,
    kind: entry.isDirectory ? 'folder' : entry.isCloud ? 'cloud' : kinds[entry.category] ?? 'file',
    isDirectory: entry.isDirectory,
    isCloudPlaceholder: entry.isCloud,
    size: entry.size,
    modifiedUnix: entry.mtime,
  };
}

function pathEquals(left: string, right: string): boolean {
  return left.replaceAll('/', '\\').replace(/\\+$/, '').toLowerCase() === right.replaceAll('/', '\\').replace(/\\+$/, '').toLowerCase();
}

function parentOf(path: string): string | null {
  const normalized = path.replaceAll('/', '\\').replace(/\\+$/, '');
  const index = normalized.lastIndexOf('\\');
  if (index < 0) return null;
  if (index === 2 && normalized.length >= 2 && normalized[1] === ':') return `${normalized.slice(0, 2)}\\`;
  return normalized.slice(0, index) || null;
}

function breadcrumbItems(path: string, homePath: string | null): Array<{ label: string; path: string }> {
  if (!homePath || !path.toLowerCase().startsWith(homePath.toLowerCase().replace(/[\\/]+$/, ''))) {
    return [{ label: path.split(/[\\/]/).filter(Boolean).at(-1) ?? 'Folder', path }];
  }
  const root = homePath.replace(/[\\/]+$/, '');
  const relative = path.slice(root.length).replace(/^[\\/]+/, '');
  const parts = relative.split(/[\\/]+/).filter(Boolean);
  const items = [{ label: 'Home', path: homePath }];
  let accumulated = root;
  for (const part of parts) {
    accumulated = `${accumulated}\\${part}`;
    items.push({ label: part, path: accumulated });
  }
  return items;
}

function sizeBytes(value: string): number | null {
  if (!value.trim()) return null;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) return null;
  return Math.round(parsed * 1024 * 1024);
}

function dateBoundary(value: string, end: boolean): number | null {
  if (!value) return null;
  const parsed = Date.parse(`${value}T00:00:00`);
  if (!Number.isFinite(parsed)) return null;
  return Math.floor(parsed / 1000) + (end ? 86_399 : 0);
}

function isTypingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement || target instanceof HTMLTextAreaElement;
}

export function BrowseRoute({ desktopAvailable, query, onQueryChange, onShareRequest, onOpenSettings }: BrowseRouteProps) {
  const index = useIndexStore();
  const loadDirectory = useIndexStore((state) => state.loadDirectory);
  const selectedPaths = useAppStore((state) => state.selectedPaths);
  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  const toggleSelected = useAppStore((state) => state.toggleSelected);
  const clearSelection = useAppStore((state) => state.clearSelection);
  const openFile = useAppStore((state) => state.openFile);
  const setNotice = useAppStore((state) => state.setNotice);

  const clipboard = useOpsStore((state) => state.clipboard);
  const copyToClipboard = useOpsStore((state) => state.copyToClipboard);
  const pasteInto = useOpsStore((state) => state.pasteInto);
  const conflict = useOpsStore((state) => state.conflict);
  const resolveConflict = useOpsStore((state) => state.resolveConflict);
  const dismissConflict = useOpsStore((state) => state.dismissConflict);
  const busy = useOpsStore((state) => state.busy);
  const deletePaths = useOpsStore((state) => state.deletePaths);
  const renamePath = useOpsStore((state) => state.renamePath);
  const toggleFavorite = useOpsStore((state) => state.toggleFavorite);
  const favorites = useOpsStore((state) => state.favorites);
  const recents = useOpsStore((state) => state.recents);
  const loadRecents = useOpsStore((state) => state.loadRecents);
  const clearRecents = useOpsStore((state) => state.clearRecents);
  const setRefresh = useOpsStore((state) => state.setRefresh);

  const [view, setView] = useState<'list' | 'grid'>('list');
  const [sort, setSort] = useState('name');
  const [descending, setDescending] = useState(false);
  const [category, setCategory] = useState('');
  const [minSize, setMinSize] = useState('');
  const [maxSize, setMaxSize] = useState('');
  const [modifiedAfter, setModifiedAfter] = useState('');
  const [modifiedBefore, setModifiedBefore] = useState('');
  const [columns, setColumns] = useState(3);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [renameTarget, setRenameTarget] = useState<FileEntry | null>(null);
  const [propertiesTarget, setPropertiesTarget] = useState<FileEntry | null>(null);
  const [previewTarget, setPreviewTarget] = useState<FileEntry | null>(null);
  const gridScrollRef = useRef<HTMLDivElement>(null);
  const browserRef = useRef<HTMLElement>(null);

  const isFiltered = Boolean(query.trim() || category || minSize || maxSize || modifiedAfter || modifiedBefore);
  const home = index.locations.find((location) => location.id === 'home');
  const currentPath = index.activePath;
  const breadcrumbs = useMemo(() => currentPath ? breadcrumbItems(currentPath, home?.path ?? null) : [], [currentPath, home?.path]);
  const entries = useMemo(() => index.entries.map(toFileEntry), [index.entries]);
  const order = useMemo(() => entries.map((entry) => entry.path), [entries]);
  const selectedEntries = useMemo(() => entries.filter((entry) => selectedPaths.includes(entry.path)), [entries, selectedPaths]);
  const allCategories = useMemo(() => Object.keys(categoryDetails).map((name) => {
    const found = index.categories.find((item) => item.category === name);
    return found ?? { category: name, count: 0, totalSize: 0 } satisfies CategorySummary;
  }), [index.categories]);

  useEffect(() => {
    if (!currentPath || !desktopAvailable) return;
    if (!isFiltered) {
      void loadDirectory(currentPath, sort, descending);
      return;
    }
    const timer = window.setTimeout(() => {
      void index.search({
        query,
        category: category || null,
        minSize: sizeBytes(minSize),
        maxSize: sizeBytes(maxSize),
        modifiedAfter: dateBoundary(modifiedAfter, false),
        modifiedBefore: dateBoundary(modifiedBefore, true),
        limit: 250,
      });
    }, query.trim() ? 180 : 0);
    return () => window.clearTimeout(timer);
  }, [category, currentPath, descending, desktopAvailable, index.search, isFiltered, loadDirectory, maxSize, minSize, modifiedAfter, modifiedBefore, query, sort]);

  useEffect(() => {
    const container = browserRef.current;
    if (!container) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      setColumns(Math.max(1, Math.floor((entry.contentRect.width + 14) / 215)));
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [view, entries.length]);

  useEffect(() => {
    if (!desktopAvailable) return;
    void loadRecents();
  }, [desktopAvailable, loadRecents]);

  // Filesystem changes land here so the listing (and the index behind it) refreshes.
  useEffect(() => {
    if (!currentPath) return;
    setRefresh(() => () => {
      void loadDirectory(currentPath, sort, descending);
    });
    return () => setRefresh(null);
  }, [currentPath, descending, loadDirectory, setRefresh, sort]);

  const handleSelect = useCallback((path: string, modifiers: SelectionModifiers) => {
    toggleSelected(path, { additive: modifiers.additive, range: modifiers.range, order });
  }, [order, toggleSelected]);

  const handleSelectMany = useCallback((paths: string[], additive: boolean) => {
    if (!additive) {
      setSelectedPaths(paths);
      return;
    }
    setSelectedPaths(Array.from(new Set([...selectedPaths, ...paths])));
  }, [selectedPaths, setSelectedPaths]);

  function openEntry(entry: FileEntry) {
    if (entry.isDirectory) {
      void loadDirectory(entry.path, sort, descending);
      clearSelection();
      return;
    }
    if (entry.isCloudPlaceholder) {
      setNotice('This file is online-only. Sift will not hydrate it automatically.');
      return;
    }
    if (previewKindFor(entry) !== 'none') {
      setPreviewTarget(entry);
      return;
    }
    void openFile(entry.path);
  }

  function copySelection(mode: 'copy' | 'cut') {
    if (selectedEntries.length === 0) {
      setNotice('Select something to copy first.');
      return;
    }
    copyToClipboard(selectedEntries.map((entry) => entry.path), selectedEntries.map((entry) => entry.name), mode);
    setNotice(`${mode === 'cut' ? 'Cut' : 'Copied'} ${selectedEntries.length} item${selectedEntries.length === 1 ? '' : 's'}.`);
  }

  async function requestDelete() {
    if (selectedPaths.length === 0) return;
    const confirmed = window.confirm(`Move ${selectedPaths.length} selected item${selectedPaths.length === 1 ? '' : 's'} to the Recycle Bin?`);
    if (!confirmed) return;
    const result = await deletePaths(selectedPaths);
    if (result) clearSelection();
  }

  function openContextMenu(position: { x: number; y: number }, entry: FileEntry | null) {
    setMenu({ x: position.x, y: position.y, entry });
  }

  function shell(command: (path: string) => Promise<void>) {
    return async (path: string) => {
      try {
        await command(path);
      } catch (failure: unknown) {
        setNotice(failure instanceof Error ? failure.message : typeof failure === 'string' ? failure : 'Windows could not run that action.');
      }
    };
  }

  // Recomputed each render so the verbs always see the current sort/selection.
  const menuItems: ContextMenuItem[] = (() => {
    const entry = menu?.entry ?? selectedEntries[0] ?? null;
    const multiple = selectedPaths.length > 1;
    const pinned = entry ? favorites.some((favorite) => favorite.path.toLowerCase() === entry.path.toLowerCase()) : false;
    const previewable = entry ? !entry.isDirectory && !entry.isCloudPlaceholder && previewKindFor(entry) !== 'none' : false;
    const pasteDestination = entry?.isDirectory ? entry.path : currentPath;

    return [
      {
        id: 'open',
        label: multiple ? 'Open each' : 'Open',
        icon: ExternalLink,
        disabled: !entry || entry.isCloudPlaceholder,
        onSelect: () => {
          if (!entry) return;
          if (entry.isDirectory) void loadDirectory(entry.path, sort, descending);
          else void openFile(entry.path);
        },
      },
      {
        id: 'preview',
        label: 'Preview',
        icon: Eye,
        disabled: !previewable,
        onSelect: () => {
          if (entry) setPreviewTarget(entry);
        },
      },
      {
        id: 'open-with',
        label: 'Open with…',
        icon: Grid2X2,
        disabled: !entry || entry.isDirectory || entry.isCloudPlaceholder,
        onSelect: () => {
          if (entry) void shell(commands.openWith)(entry.path);
        },
      },
      {
        id: 'copy',
        label: 'Copy',
        icon: Copy,
        shortcut: 'Ctrl C',
        separatorBefore: true,
        disabled: !entry,
        onSelect: () => copySelection('copy'),
      },
      {
        id: 'cut',
        label: 'Cut',
        icon: Scissors,
        shortcut: 'Ctrl X',
        disabled: !entry,
        onSelect: () => copySelection('cut'),
      },
      {
        id: 'paste',
        label: 'Paste',
        icon: ClipboardPaste,
        shortcut: 'Ctrl V',
        disabled: !clipboard || !pasteDestination,
        onSelect: () => {
          if (pasteDestination) void pasteInto(pasteDestination);
        },
      },
      {
        id: 'rename',
        label: 'Rename',
        icon: Pencil,
        shortcut: 'F2',
        separatorBefore: true,
        disabled: !entry || multiple,
        onSelect: () => {
          if (entry) setRenameTarget(entry);
        },
      },
      {
        id: 'delete',
        label: 'Delete',
        icon: Trash2,
        shortcut: 'Del',
        danger: true,
        disabled: selectedPaths.length === 0,
        onSelect: () => void requestDelete(),
      },
      {
        id: 'favorite',
        label: pinned ? 'Remove from Favorites' : 'Add to Favorites',
        icon: Star,
        separatorBefore: true,
        disabled: !entry,
        onSelect: () => {
          if (entry) void toggleFavorite(entry.path, entry.name, entry.isDirectory);
        },
      },
      {
        id: 'reveal',
        label: 'Reveal in Explorer',
        icon: Folder,
        disabled: !entry,
        onSelect: () => {
          if (entry) void shell(commands.revealInExplorer)(entry.path);
        },
      },
      {
        id: 'properties',
        label: 'Properties',
        icon: Info,
        disabled: !entry,
        onSelect: () => {
          if (entry) setPropertiesTarget(entry);
        },
      },
      {
        id: 'share',
        label: 'Share…',
        icon: Send,
        separatorBefore: true,
        disabled: !entry || entry.isDirectory || entry.isCloudPlaceholder || !onShareRequest,
        onSelect: () => {
          if (entry) onShareRequest?.(entry);
        },
      },
    ];
  })();

  // Global shortcuts for the browser surface. Re-subscribed each render so the
  // handlers always see the current selection, clipboard, and sort order.
  useEffect(() => {
    if (!desktopAvailable) return;
    const onKey = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target)) return;
      const modifier = event.ctrlKey || event.metaKey;
      if (modifier && event.key.toLowerCase() === 'c') {
        event.preventDefault();
        copySelection('copy');
      } else if (modifier && event.key.toLowerCase() === 'x') {
        event.preventDefault();
        copySelection('cut');
      } else if (modifier && event.key.toLowerCase() === 'v') {
        event.preventDefault();
        if (currentPath) void pasteInto(currentPath);
      } else if (modifier && event.key.toLowerCase() === 'a') {
        event.preventDefault();
        setSelectedPaths(order);
      } else if (event.key === 'F2' && selectedEntries.length === 1 && selectedEntries[0]) {
        event.preventDefault();
        setRenameTarget(selectedEntries[0]);
      } else if (event.key === 'Delete') {
        event.preventDefault();
        void requestDelete();
      } else if (event.key === 'Enter' && selectedEntries.length === 1 && selectedEntries[0]) {
        event.preventDefault();
        openEntry(selectedEntries[0]);
      } else if (event.key === 'Escape') {
        clearSelection();
        setMenu(null);
      } else if ((event.altKey && event.key === 'ArrowLeft') || event.key === 'Backspace') {
        const parent = currentPath ? parentOf(currentPath) : null;
        if (parent && home && (pathEquals(parent, home.path) || parent.toLowerCase().startsWith(home.path.toLowerCase()))) {
          event.preventDefault();
          void loadDirectory(parent, sort, descending);
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function openCategory(categoryName: string) {
    setCategory((current) => current === categoryName ? '' : categoryName);
    onQueryChange('');
  }

  return (
    <div className="indexed-browse">
      {!desktopAvailable ? (
        <div className="desktop-required-card"><div className="desktop-illustration"><Folder size={27} /></div><div><strong>Your indexed files appear in the Windows app</strong><p>Sift indexes only the current Windows user's folders. The web preview never reads or imitates local files.</p></div></div>
      ) : (
        <>
          {index.progress?.scanning && <div className="index-progress-card" role="status">
            <span className="index-spinner" /><div className="index-progress-copy"><strong>Building your private index</strong><span>{index.progress.filesScanned.toLocaleString()} files scanned · {index.progress.drive || 'Finding drives'} · {index.progress.currentDir}</span></div>
            {index.progress.skipped > 0 && (
              onOpenSettings ? (
                <button type="button" className="index-progress-skipped" onClick={onOpenSettings} title="See which folders were skipped">
                  {index.progress.skipped.toLocaleString()} skipped · why?
                </button>
              ) : (
                <span className="index-progress-skipped">{index.progress.skipped.toLocaleString()} skipped</span>
              )
            )}
          </div>}
          {index.progress?.error && <div className="index-inline-error" role="alert"><ShieldCheck size={15} />{index.progress.error}</div>}

          <section className="browse-overview-grid">
            <div className="overview-panel categories-panel">
              <div className="overview-heading"><div><span className="section-overline">YOUR FILES, SORTED</span><h2>Categories</h2></div><span>{index.progress?.filesScanned.toLocaleString() ?? '—'} indexed</span></div>
              <div className="category-grid">
                {allCategories.map((item) => {
                  const detail = categoryDetails[item.category] ?? { label: 'Other', color: 'other', icon: Folder };
                  const Icon = detail.icon;
                  return <button type="button" key={item.category} className={`category-card category-${detail.color}${category === item.category ? ' selected' : ''}`} onClick={() => openCategory(item.category)} aria-pressed={category === item.category}>
                    <span className="category-icon"><Icon size={17} /></span><strong>{detail.label}</strong><small>{item.count.toLocaleString()} files · {readableSize(item.totalSize)}</small>
                  </button>;
                })}
              </div>
            </div>
            <div className="overview-panel storage-panel">
              <div className="overview-heading"><div><span className="section-overline">SPACE AT A GLANCE</span><h2>Drives</h2></div><HardDrive size={17} /></div>
              {index.drives.length === 0 ? <div className="drive-empty">Drive information will appear here when Windows reports a mounted fixed or removable drive.</div> : <div className="drive-list">
                {index.drives.map((drive: DriveStorage) => {
                  const usage = drive.total > 0 ? Math.min(100, drive.used / drive.total * 100) : 0;
                  return <div className="drive-item" key={drive.drive}>
                    <div className="drive-caption"><strong>{drive.drive}</strong><span>{readableSize(drive.free)} free of {readableSize(drive.total)}</span></div>
                    <div className="drive-track" role="img" aria-label={`${drive.drive}: ${readableSize(drive.used)} used, ${readableSize(drive.free)} free`}><span style={{ width: `${usage}%` }} /></div>
                    <div className="drive-foot"><span>{readableSize(drive.used)} used</span><span>{readableSize(drive.indexedSize)} indexed for this user</span></div>
                  </div>;
                })}
              </div>}
              <p className="storage-privacy"><ShieldCheck size={13} /> Drive capacity comes from Windows. File indexing stays within your user folders.</p>
            </div>
          </section>

          <section className="browse-shortcuts" aria-label="Favorites and recent files">
            <div className="shortcut-panel">
              <div className="overview-heading"><div><span className="section-overline">PINNED BY YOU</span><h2>Favorites</h2></div><Star size={15} /></div>
              {favorites.length === 0 ? <p className="shortcut-empty">Right-click any file or folder and choose “Add to Favorites”.</p> : (
                <ul className="shortcut-list">
                  {favorites.slice(0, 8).map((favorite) => (
                    <li key={favorite.path}>
                      <button type="button" title={favorite.path} onClick={() => {
                        if (favorite.isDirectory) { void loadDirectory(favorite.path, sort, descending); clearSelection(); }
                        else {
                          const parent = parentOf(favorite.path);
                          if (parent) { void loadDirectory(parent, sort, descending); setSelectedPaths([favorite.path]); }
                        }
                      }}>
                        <Star size={13} fill="currentColor" />
                        <span>{favorite.name}</span>
                      </button>
                      <button type="button" className="shortcut-remove" aria-label={`Remove ${favorite.name} from favorites`} onClick={() => void toggleFavorite(favorite.path, favorite.name, favorite.isDirectory)}><X size={12} /></button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div className="shortcut-panel">
              <div className="overview-heading"><div><span className="section-overline">OPENED LATELY</span><h2>Recent</h2></div>
                {recents.length > 0 && <button type="button" className="text-button" onClick={() => void clearRecents()}>Clear</button>}
              </div>
              {recents.length === 0 ? <p className="shortcut-empty">Files you open or preview in Sift are listed here. Nothing leaves this PC.</p> : (
                <ul className="shortcut-list">
                  {recents.slice(0, 8).map((recent) => (
                    <li key={recent.path}>
                      <button type="button" title={recent.path} onClick={() => void openFile(recent.path)}>
                        <FileGlyph entry={{ name: recent.name, path: recent.path, extension: recent.name.includes('.') ? recent.name.split('.').pop() ?? '' : '', kind: 'file', isDirectory: false, isCloudPlaceholder: false, size: 0, modifiedUnix: recent.openedAtUnix }} />
                        <span>{recent.name}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>

          <section className="indexed-folder-browser" ref={browserRef}>
            <div className="browser-heading-row"><div><span className="section-overline">BROWSE YOUR INDEX</span><h2>{isFiltered ? 'Search results' : breadcrumbs.at(-1)?.label ?? 'Home'}</h2><p>{isFiltered ? `${entries.length.toLocaleString()} matching indexed files` : 'Right-click for copy, paste, rename, and more.'}</p></div>
              <div className="browser-controls">
                <label className="compact-select"><span>Sort</span><select value={sort} onChange={(event) => setSort(event.target.value)} aria-label="Sort indexed files"><option value="name">Name</option><option value="date">Date</option><option value="size">Size</option><option value="type">Type</option></select></label>
                <button className="icon-button" type="button" onClick={() => setDescending((value) => !value)} aria-label={descending ? 'Sort ascending' : 'Sort descending'} title={descending ? 'Descending' : 'Ascending'}>{descending ? <ArrowDownWideNarrow size={16} /> : <ArrowUpWideNarrow size={16} />}</button>
                <button className={`icon-button${view === 'list' ? ' selected' : ''}`} type="button" onClick={() => setView('list')} aria-label="List view" title="List view"><List size={16} /></button>
                <button className={`icon-button${view === 'grid' ? ' selected' : ''}`} type="button" onClick={() => setView('grid')} aria-label="Grid view" title="Grid view"><Grid2X2 size={16} /></button>
              </div>
            </div>

            <div className="file-action-bar" role="toolbar" aria-label="File actions">
              <button type="button" className="file-action" onClick={() => copySelection('copy')} disabled={selectedPaths.length === 0}><Copy size={14} /> Copy</button>
              <button type="button" className="file-action" onClick={() => copySelection('cut')} disabled={selectedPaths.length === 0}><Scissors size={14} /> Cut</button>
              <button type="button" className="file-action" onClick={() => { if (currentPath) void pasteInto(currentPath); }} disabled={!clipboard || !currentPath}>
                {clipboard?.mode === 'cut' ? <Clipboard size={14} /> : <ClipboardPaste size={14} />} Paste{clipboard ? ` (${clipboard.paths.length})` : ''}
              </button>
              <button type="button" className="file-action" onClick={() => { const only = selectedEntries[0]; if (only) setRenameTarget(only); }} disabled={selectedEntries.length !== 1}><Pencil size={14} /> Rename</button>
              <button type="button" className="file-action" onClick={() => { const only = selectedEntries[0]; if (only) void toggleFavorite(only.path, only.name, only.isDirectory); }} disabled={selectedEntries.length !== 1}><Star size={14} /> Pin</button>
              <button type="button" className="file-action danger" onClick={() => void requestDelete()} disabled={selectedPaths.length === 0}><Trash2 size={14} /> Delete</button>
              <span className="file-action-hint">
                {selectedPaths.length > 0
                  ? `${selectedPaths.length} selected · Ctrl C / X / V · F2 · Del`
                  : 'Ctrl-click, shift-click, or drag to select · right-click for more'}
              </span>
            </div>

            <div className="index-breadcrumbs" aria-label="Folder breadcrumb">
              {breadcrumbs.map((crumb, crumbIndex) => <span key={`${crumb.path}:${crumbIndex}`}><button type="button" disabled={pathEquals(crumb.path, currentPath ?? '')} onClick={() => void loadDirectory(crumb.path, sort, descending)}>{crumb.label}</button>{crumbIndex < breadcrumbs.length - 1 && <ChevronRight size={13} />}</span>)}
              {!isFiltered && index.activePath && <span className="keyboard-hint">Alt + ← to go up</span>}
            </div>

            <div className="filter-row">
              <label className="filter-search"><Search size={15} /><input value={query} onChange={(event) => onQueryChange(event.target.value)} placeholder="Search names (prefix match)" aria-label="Search indexed filenames" /></label>
              <label className="filter-control"><span>Min MB</span><input inputMode="decimal" value={minSize} onChange={(event) => setMinSize(event.target.value)} aria-label="Minimum size in megabytes" placeholder="—" /></label>
              <label className="filter-control"><span>Max MB</span><input inputMode="decimal" value={maxSize} onChange={(event) => setMaxSize(event.target.value)} aria-label="Maximum size in megabytes" placeholder="—" /></label>
              <label className="filter-date"><span>Modified after</span><input type="date" value={modifiedAfter} onChange={(event) => setModifiedAfter(event.target.value)} aria-label="Modified after date" /></label>
              <label className="filter-date"><span>Before</span><input type="date" value={modifiedBefore} onChange={(event) => setModifiedBefore(event.target.value)} aria-label="Before date" /></label>
              {isFiltered && <button type="button" className="clear-filters" onClick={() => { onQueryChange(''); setCategory(''); setMinSize(''); setMaxSize(''); setModifiedAfter(''); setModifiedBefore(''); }}>Clear</button>}
            </div>

            {index.loading || index.searching ? <div className="list-loading" role="status" aria-label="Loading indexed files"><span /><span /><span /></div> : index.error ? <div className="indexed-error-state"><ShieldCheck size={20} /><strong>Index unavailable</strong><p>{index.error}</p></div> : view === 'list' ? (
              <FileList
                entries={entries}
                selectedPaths={selectedPaths}
                onSelect={handleSelect}
                onSelectMany={handleSelectMany}
                onOpen={openEntry}
                onContextMenu={openContextMenu}
                onRenameRequest={setRenameTarget}
                showThumbnails
                emptyTitle={isFiltered ? 'No matching files' : 'This folder is empty'}
                emptyMessage={isFiltered ? 'Try fewer filters or a shorter filename prefix.' : index.progress?.scanning ? 'This folder is still being indexed.' : 'Files in this folder will appear here when indexed.'}
              />
            ) : (
              <VirtualGrid
                entries={entries}
                columns={columns}
                scrollRef={gridScrollRef}
                selectedPaths={selectedPaths}
                onSelect={handleSelect}
                onSelectMany={handleSelectMany}
                onOpen={openEntry}
                onContextMenu={openContextMenu}
                onRenameRequest={setRenameTarget}
                empty={isFiltered ? 'No matching files' : 'This folder is empty'}
              />
            )}
            {!isFiltered && index.hasMore && currentPath && <button type="button" className="load-more-button" onClick={() => void loadDirectory(currentPath, sort, descending, index.entries.length)} disabled={index.loadingMore}>{index.loadingMore ? 'Loading more…' : 'Load more files'}</button>}
          </section>
        </>
      )}

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems} title={menu.entry?.name} onClose={() => setMenu(null)} />}

      {conflict && (
        <ConflictDialog
          request={conflict}
          busy={busy}
          onResolve={(decisions: ConflictDecision[], defaultAction: ConflictAction) => void resolveConflict(decisions, defaultAction)}
          onCancel={dismissConflict}
        />
      )}

      {renameTarget && (
        <RenameDialog
          entry={renameTarget}
          busy={busy}
          onClose={() => setRenameTarget(null)}
          onSubmit={(name) => {
            void renamePath(renameTarget.path, name).then((done) => {
              if (done) setRenameTarget(null);
            });
          }}
        />
      )}

      {propertiesTarget && (
        <PropertiesDialog
          path={propertiesTarget.path}
          favorite={favorites.some((favorite) => favorite.path.toLowerCase() === propertiesTarget.path.toLowerCase())}
          onToggleFavorite={(path, name, isDirectory) => void toggleFavorite(path, name, isDirectory)}
          onClose={() => setPropertiesTarget(null)}
        />
      )}

      {previewTarget && (
        <PreviewDialog
          entry={previewTarget}
          onOpenWithApp={(path) => void openFile(path)}
          onClose={() => setPreviewTarget(null)}
        />
      )}
    </div>
  );
}

interface VirtualGridProps {
  entries: FileEntry[];
  columns: number;
  scrollRef: RefObject<HTMLDivElement>;
  selectedPaths: string[];
  onSelect: (path: string, modifiers: SelectionModifiers) => void;
  onSelectMany: (paths: string[], additive: boolean) => void;
  onOpen: (entry: FileEntry) => void;
  onContextMenu: (position: { x: number; y: number }, entry: FileEntry | null) => void;
  onRenameRequest: (entry: FileEntry) => void;
  empty: string;
}

function VirtualGrid({ entries, columns, scrollRef, selectedPaths, onSelect, onSelectMany, onOpen, onContextMenu, onRenameRequest, empty }: VirtualGridProps) {
  const rowCount = Math.ceil(entries.length / columns);
  const virtualizer = useVirtualizer({ count: rowCount, getScrollElement: () => scrollRef.current, estimateSize: () => 104, overscan: 6 });
  const marquee = useMarquee(scrollRef, onSelectMany, () => onSelectMany([], false));
  if (entries.length === 0) return <div className="grid-empty"><Sparkles size={20} /><strong>{empty}</strong><span>Try a different folder or search.</span></div>;
  return <div ref={scrollRef} className="indexed-grid-scroll"><div className="indexed-grid-inner" style={{ height: virtualizer.getTotalSize() }}>
    {virtualizer.getVirtualItems().map((row) => <div className="indexed-grid-row" key={row.key} style={{ transform: `translateY(${row.start}px)`, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
      {entries.slice(row.index * columns, (row.index + 1) * columns).map((entry, columnIndex) => {
        const entryIndex = row.index * columns + columnIndex;
        const checked = selectedPaths.includes(entry.path);
        return <div
          key={entry.path}
          role="button"
          tabIndex={0}
          data-grid-index={entryIndex}
          data-file-path={entry.path}
          aria-pressed={checked}
          aria-label={`${entry.isDirectory ? 'Folder' : 'File'} ${entry.name}${entry.isCloudPlaceholder ? ', online only' : ''}`}
          className={`indexed-grid-card${checked ? ' selected' : ''}`}
          onKeyDown={(event) => {
            if (event.key === 'Enter') { event.preventDefault(); onOpen(entry); return; }
            if (event.key === 'F2') { event.preventDefault(); onRenameRequest(entry); return; }
            if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
            event.preventDefault();
            const delta = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowUp' ? -columns : event.key === 'ArrowDown' ? columns : 0;
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? entries.length - 1 : Math.max(0, Math.min(entries.length - 1, entryIndex + delta));
            virtualizer.scrollToIndex(Math.floor(next / columns), { align: 'auto' });
            window.requestAnimationFrame(() => scrollRef.current?.querySelector<HTMLElement>(`[data-grid-index="${next}"]`)?.focus());
          }}
          onClick={(event) => onSelect(entry.path, { additive: event.ctrlKey || event.metaKey, range: event.shiftKey })}
          onDoubleClick={() => onOpen(entry)}
          onContextMenu={(event) => {
            event.preventDefault();
            onSelect(entry.path, { additive: event.ctrlKey || event.metaKey || checked, range: false });
            onContextMenu({ x: event.clientX, y: event.clientY }, entry);
          }}
        >
          <span className="grid-card-top"><EntryThumb entry={entry} enabled /><span className="grid-card-check" aria-hidden="true">{checked && <Check size={12} />}</span></span>
          <strong title={entry.name}>{entry.name}</strong><span>{entry.isDirectory ? 'Folder' : entry.isCloudPlaceholder ? 'Online only' : entry.extension.toUpperCase() || 'File'} · {entry.isDirectory ? '—' : readableSize(entry.size)}</span>
        </div>;
      })}
    </div>)}
    {marquee && <span className="marquee-rect" style={{ left: marquee.left, top: marquee.top, width: marquee.width, height: marquee.height }} aria-hidden="true" />}
  </div></div>;
}
