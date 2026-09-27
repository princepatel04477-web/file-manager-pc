import { useRef, type MouseEvent as ReactMouseEvent } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Archive, AudioLines, Cloud, File, FileImage, FileText, Folder, Film, HardDrive } from 'lucide-react';
import type { FileEntry } from '../lib/bindings';
import { useMarquee } from '../hooks/useMarquee';
import { useThumbnail } from './Thumbnail';

export interface SelectionModifiers {
  additive: boolean;
  range: boolean;
}

interface FileListProps {
  entries: FileEntry[];
  selectedPaths: string[];
  onSelect: (path: string, modifiers: SelectionModifiers) => void;
  onSelectMany?: (paths: string[], additive: boolean) => void;
  onOpen: (entry: FileEntry) => void;
  onContextMenu?: (position: { x: number; y: number }, entry: FileEntry | null) => void;
  onRenameRequest?: (entry: FileEntry) => void;
  showThumbnails?: boolean;
  emptyTitle?: string;
  emptyMessage?: string;
  heightClass?: string;
}

function readableSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[unit]}`;
}

function FileGlyph({ entry }: { entry: FileEntry }) {
  const className = `file-glyph glyph-${entry.kind}`;
  if (entry.isDirectory) return <span className={className}><Folder size={19} strokeWidth={1.8} /></span>;
  if (entry.isCloudPlaceholder) return <span className={className}><Cloud size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'image') return <span className={className}><FileImage size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'video') return <span className={className}><Film size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'audio') return <span className={className}><AudioLines size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'archive') return <span className={className}><Archive size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'document') return <span className={className}><FileText size={18} strokeWidth={1.8} /></span>;
  if (entry.kind === 'drive') return <span className={className}><HardDrive size={18} strokeWidth={1.8} /></span>;
  return <span className={className}><File size={18} strokeWidth={1.8} /></span>;
}

/** Shell thumbnail when one exists, otherwise the usual type glyph. */
function EntryThumb({ entry, enabled }: { entry: FileEntry; enabled: boolean }) {
  const source = useThumbnail(entry, enabled);
  if (!source) return <FileGlyph entry={entry} />;
  return <span className={`file-glyph glyph-thumb${entry.kind === 'video' ? ' has-video-badge' : ''}`}><img src={source} alt="" loading="lazy" /></span>;
}

function modifiedLabel(timestamp: number | null): string {
  if (timestamp === null) return '—';
  const date = new Date(timestamp * 1000);
  const today = new Date();
  const isToday = date.toDateString() === today.toDateString();
  return new Intl.DateTimeFormat(undefined, isToday ? { hour: 'numeric', minute: '2-digit' } : { month: 'short', day: 'numeric', year: date.getFullYear() === today.getFullYear() ? undefined : 'numeric' }).format(date);
}

export function FileList({
  entries,
  selectedPaths,
  onSelect,
  onSelectMany,
  onOpen,
  onContextMenu,
  onRenameRequest,
  showThumbnails = false,
  emptyTitle = 'Nothing here yet',
  emptyMessage = 'Files in this folder will appear here.',
  heightClass = 'file-list-scroll',
}: FileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 66,
    overscan: 10,
  });
  const virtualRows = virtualizer.getVirtualItems();
  const marquee = useMarquee(
    scrollRef,
    (paths, additive) => onSelectMany?.(paths, additive),
    () => onSelectMany?.([], false),
  );

  return (
    <div className="file-list-shell">
      <div className="file-list-head" role="row">
        <span className="head-name">Name</span>
        <span className="head-type">Type</span>
        <span className="head-date">Date modified</span>
        <span className="head-size">Size</span>
      </div>
      {entries.length === 0 ? (
        <div className="list-empty">
          <div className="empty-icon"><Folder size={23} strokeWidth={1.6} /></div>
          <h3>{emptyTitle}</h3>
          <p>{emptyMessage}</p>
        </div>
      ) : (
        <div ref={scrollRef} className={heightClass}>
          <div className="virtual-spacer" style={{ height: `${virtualizer.getTotalSize()}px` }}>
            {virtualRows.map((virtualRow) => {
              const entry = entries[virtualRow.index];
              if (!entry) return null;
              const selected = selectedPaths.includes(entry.path);
              return (
                <div
                  key={entry.path}
                  className={`file-row${selected ? ' is-selected' : ''}`}
                  role="row"
                  tabIndex={0}
                  data-file-index={virtualRow.index}
                  data-file-path={entry.path}
                  aria-selected={selected}
                  aria-label={`${entry.isDirectory ? 'Folder' : 'File'} ${entry.name}${entry.isCloudPlaceholder ? ', online only' : ''}`}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') { event.preventDefault(); onOpen(entry); }
                    else if (event.key === ' ') { event.preventDefault(); onSelect(entry.path, { additive: true, range: false }); }
                    else if (event.key === 'F2') { event.preventDefault(); onRenameRequest?.(entry); }
                    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Home' || event.key === 'End') {
                      event.preventDefault();
                      const next = event.key === 'Home' ? 0 : event.key === 'End' ? entries.length - 1 : Math.max(0, Math.min(entries.length - 1, virtualRow.index + (event.key === 'ArrowDown' ? 1 : -1)));
                      virtualizer.scrollToIndex(next, { align: 'auto' });
                      window.requestAnimationFrame(() => scrollRef.current?.querySelector<HTMLElement>(`[data-file-index="${next}"]`)?.focus());
                    }
                  }}
                  onClick={(event: ReactMouseEvent<HTMLDivElement>) => {
                    const target = event.target;
                    if (target instanceof Element && target.closest('button, input')) return;
                    onSelect(entry.path, {
                      additive: event.ctrlKey || event.metaKey,
                      range: event.shiftKey,
                    });
                  }}
                  onDoubleClick={() => onOpen(entry)}
                  onContextMenu={(event) => {
                    if (!onContextMenu) return;
                    event.preventDefault();
                    onSelect(entry.path, { additive: event.ctrlKey || event.metaKey || selected, range: false });
                    onContextMenu({ x: event.clientX, y: event.clientY }, entry);
                  }}
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  <div className="file-name-cell">
                    <input
                      className="row-checkbox"
                      aria-label={`Select ${entry.name}`}
                      type="checkbox"
                      checked={selected}
                      onChange={() => onSelect(entry.path, { additive: true, range: false })}
                    />
                    <EntryThumb entry={entry} enabled={showThumbnails} />
                    <div className="file-name-wrap">
                      <span className="file-name" title={entry.name}>{entry.name}</span>
                      {entry.isCloudPlaceholder && <span className="cloud-label"><Cloud size={12} /> Online only</span>}
                    </div>
                  </div>
                  <span className="file-type-cell">{entry.isDirectory ? 'Folder' : entry.extension ? `${entry.extension.toUpperCase()} file` : 'File'}</span>
                  <span className="file-date-cell">{modifiedLabel(entry.modifiedUnix)}</span>
                  <span className="file-size-cell">{entry.isDirectory ? '—' : readableSize(entry.size)}</span>
                </div>
              );
            })}
          </div>
          {marquee && <span className="marquee-rect" style={{ left: marquee.left, top: marquee.top, width: marquee.width, height: marquee.height }} aria-hidden="true" />}
        </div>
      )}
    </div>
  );
}

export { EntryThumb, FileGlyph, readableSize };
