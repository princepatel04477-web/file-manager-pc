import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Archive, AudioLines, ArrowDownWideNarrow, ArrowUpWideNarrow, Check, ChevronRight, FileImage, FileText, Film, Folder, HardDrive, List, Search, ShieldCheck, Sparkles, Grid2X2 } from 'lucide-react';
import type { CategorySummary, DriveStorage, FileEntry, IndexedEntry } from '../lib/bindings';
import { useAppStore } from '../stores/app-store';
import { useIndexStore } from '../stores/index-store';
import { FileGlyph, FileList, readableSize } from '../components/FileList';

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

export function BrowseRoute({ desktopAvailable, query, onQueryChange }: BrowseRouteProps) {
  const index = useIndexStore();
  const selectedPaths = useAppStore((state) => state.selectedPaths);
  const toggleSelected = useAppStore((state) => state.toggleSelected);
  const openFile = useAppStore((state) => state.openFile);
  const setNotice = useAppStore((state) => state.setNotice);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<'list' | 'grid'>('list');
  const [sort, setSort] = useState('name');
  const [descending, setDescending] = useState(false);
  const [category, setCategory] = useState('');
  const [minSize, setMinSize] = useState('');
  const [maxSize, setMaxSize] = useState('');
  const [modifiedAfter, setModifiedAfter] = useState('');
  const [modifiedBefore, setModifiedBefore] = useState('');
  const [columns, setColumns] = useState(3);
  const isFiltered = Boolean(query.trim() || category || minSize || maxSize || modifiedAfter || modifiedBefore);
  const home = index.locations.find((location) => location.id === 'home');
  const currentPath = index.activePath;
  const breadcrumbs = useMemo(() => currentPath ? breadcrumbItems(currentPath, home?.path ?? null) : [], [currentPath, home?.path]);
  const entries = useMemo(() => index.entries.map(toFileEntry), [index.entries]);
  const allCategories = useMemo(() => Object.keys(categoryDetails).map((name) => {
    const found = index.categories.find((item) => item.category === name);
    return found ?? { category: name, count: 0, totalSize: 0 } satisfies CategorySummary;
  }), [index.categories]);

  useEffect(() => {
    if (!currentPath || !desktopAvailable) return;
    if (!isFiltered) {
      void index.loadDirectory(currentPath, sort, descending);
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
  }, [category, currentPath, descending, desktopAvailable, index.loadDirectory, index.search, isFiltered, maxSize, minSize, modifiedAfter, modifiedBefore, query, sort]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      setColumns(Math.max(1, Math.floor((entry.contentRect.width + 14) / 215)));
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [view, entries.length]);

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
      if ((event.altKey && event.key === 'ArrowLeft') || event.key === 'Backspace') {
        const parent = currentPath ? parentOf(currentPath) : null;
        if (parent && home && (pathEquals(parent, home.path) || parent.toLowerCase().startsWith(home.path.toLowerCase()))) {
          event.preventDefault();
          void index.loadDirectory(parent, sort, descending);
        }
      }
    };
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [currentPath, descending, home, index.loadDirectory, sort]);

  function openEntry(entry: FileEntry) {
    if (entry.isDirectory) void index.loadDirectory(entry.path, sort, descending);
    else if (entry.isCloudPlaceholder) setNotice('This file is online-only. Sift will not hydrate it automatically.');
    else void openFile(entry.path);
  }

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
            <span className="index-progress-skipped">{index.progress.skipped.toLocaleString()} skipped</span>
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

          <section className="indexed-folder-browser">
            <div className="browser-heading-row"><div><span className="section-overline">BROWSE YOUR INDEX</span><h2>{isFiltered ? 'Search results' : breadcrumbs.at(-1)?.label ?? 'Home'}</h2><p>{isFiltered ? `${entries.length.toLocaleString()} matching indexed files` : 'Search and browse the metadata index on this device.'}</p></div>
              <div className="browser-controls">
                <label className="compact-select"><span>Sort</span><select value={sort} onChange={(event) => setSort(event.target.value)} aria-label="Sort indexed files"><option value="name">Name</option><option value="date">Date</option><option value="size">Size</option><option value="type">Type</option></select></label>
                <button className="icon-button" type="button" onClick={() => setDescending((value) => !value)} aria-label={descending ? 'Sort ascending' : 'Sort descending'} title={descending ? 'Descending' : 'Ascending'}>{descending ? <ArrowDownWideNarrow size={16} /> : <ArrowUpWideNarrow size={16} />}</button>
                <button className={`icon-button${view === 'list' ? ' selected' : ''}`} type="button" onClick={() => setView('list')} aria-label="List view" title="List view"><List size={16} /></button>
                <button className={`icon-button${view === 'grid' ? ' selected' : ''}`} type="button" onClick={() => setView('grid')} aria-label="Grid view" title="Grid view"><Grid2X2 size={16} /></button>
              </div>
            </div>
            <div className="index-breadcrumbs" aria-label="Folder breadcrumb">
              {breadcrumbs.map((crumb, crumbIndex) => <span key={`${crumb.path}:${crumbIndex}`}><button type="button" disabled={pathEquals(crumb.path, currentPath ?? '')} onClick={() => void index.loadDirectory(crumb.path, sort, descending)}>{crumb.label}</button>{crumbIndex < breadcrumbs.length - 1 && <ChevronRight size={13} />}</span>)}
              {!isFiltered && index.activePath && <span className="keyboard-hint">Alt + ← to go up</span>}
            </div>

            <div className="filter-row">
              <label className="filter-search"><Search size={15} /><input value={query} onChange={(event) => onQueryChange(event.target.value)} placeholder="Search names (prefix match)" aria-label="Search indexed filenames" /></label>
              <label className="filter-control"><span>Min MB</span><input inputMode="decimal" value={minSize} onChange={(event) => setMinSize(event.target.value)} aria-label="Minimum size in megabytes" placeholder="—" /></label>
              <label className="filter-control"><span>Max MB</span><input inputMode="decimal" value={maxSize} onChange={(event) => setMaxSize(event.target.value)} aria-label="Maximum size in megabytes" placeholder="—" /></label>
              <label className="filter-date"><span>Modified after</span><input type="date" value={modifiedAfter} onChange={(event) => setModifiedAfter(event.target.value)} aria-label="Modified after date" /></label>
              <label className="filter-date"><span>Before</span><input type="date" value={modifiedBefore} onChange={(event) => setModifiedBefore(event.target.value)} aria-label="Modified before date" /></label>
              {isFiltered && <button type="button" className="clear-filters" onClick={() => { onQueryChange(''); setCategory(''); setMinSize(''); setMaxSize(''); setModifiedAfter(''); setModifiedBefore(''); }}>Clear</button>}
            </div>

            {index.loading || index.searching ? <div className="list-loading" role="status" aria-label="Loading indexed files"><span /><span /><span /></div> : index.error ? <div className="indexed-error-state"><ShieldCheck size={20} /><strong>Index unavailable</strong><p>{index.error}</p></div> : view === 'list' ? (
              <FileList entries={entries} selectedPaths={selectedPaths} onToggle={toggleSelected} onOpen={openEntry} emptyTitle={isFiltered ? 'No matching files' : 'This folder is empty'} emptyMessage={isFiltered ? 'Try fewer filters or a shorter filename prefix.' : index.progress?.scanning ? 'This folder is still being indexed.' : 'Files in this folder will appear here when indexed.'} />
            ) : (
              <VirtualGrid entries={entries} columns={columns} scrollRef={scrollRef} selectedPaths={selectedPaths} onToggle={toggleSelected} onOpen={openEntry} empty={isFiltered ? 'No matching files' : 'This folder is empty'} />
            )}
            {!isFiltered && index.hasMore && currentPath && <button type="button" className="load-more-button" onClick={() => void index.loadDirectory(currentPath, sort, descending, index.entries.length)} disabled={index.loadingMore}>{index.loadingMore ? 'Loading more…' : 'Load more files'}</button>}
          </section>
        </>
      )}
    </div>
  );
}

interface VirtualGridProps {
  entries: FileEntry[];
  columns: number;
  scrollRef: RefObject<HTMLDivElement>;
  selectedPaths: string[];
  onToggle: (path: string) => void;
  onOpen: (entry: FileEntry) => void;
  empty: string;
}

function VirtualGrid({ entries, columns, scrollRef, selectedPaths, onToggle, onOpen, empty }: VirtualGridProps) {
  const rowCount = Math.ceil(entries.length / columns);
  const virtualizer = useVirtualizer({ count: rowCount, getScrollElement: () => scrollRef.current, estimateSize: () => 88, overscan: 6 });
  if (entries.length === 0) return <div className="grid-empty"><Sparkles size={20} /><strong>{empty}</strong><span>Try a different folder or search.</span></div>;
  return <div ref={scrollRef} className="indexed-grid-scroll"><div className="indexed-grid-inner" style={{ height: virtualizer.getTotalSize() }}>
    {virtualizer.getVirtualItems().map((row) => <div className="indexed-grid-row" key={row.key} style={{ transform: `translateY(${row.start}px)`, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
      {entries.slice(row.index * columns, (row.index + 1) * columns).map((entry, columnIndex) => {
        const entryIndex = row.index * columns + columnIndex;
        const checked = selectedPaths.includes(entry.path);
        return <button type="button" key={entry.path} data-grid-index={entryIndex} className={`indexed-grid-card${checked ? ' selected' : ''}`} onKeyDown={(event) => {
          if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
          event.preventDefault();
          const delta = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowUp' ? -columns : event.key === 'ArrowDown' ? columns : 0;
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? entries.length - 1 : Math.max(0, Math.min(entries.length - 1, entryIndex + delta));
          virtualizer.scrollToIndex(Math.floor(next / columns), { align: 'auto' });
          window.requestAnimationFrame(() => scrollRef.current?.querySelector<HTMLElement>(`[data-grid-index="${next}"]`)?.focus());
        }} onClick={() => onToggle(entry.path)} onDoubleClick={() => onOpen(entry)} aria-pressed={checked} aria-label={`${entry.isDirectory ? 'Folder' : 'File'} ${entry.name}${entry.isCloudPlaceholder ? ', online only' : ''}`}>
          <span className="grid-card-top"><FileGlyph entry={entry} /><span className="grid-card-check" aria-hidden="true">{checked && <Check size={12} />}</span></span>
          <strong title={entry.name}>{entry.name}</strong><span>{entry.isDirectory ? 'Folder' : entry.isCloudPlaceholder ? 'Online only' : entry.extension.toUpperCase() || 'File'} · {entry.isDirectory ? '—' : readableSize(entry.size)}</span>
        </button>;
      })}
    </div>)}
  </div></div>;
}
