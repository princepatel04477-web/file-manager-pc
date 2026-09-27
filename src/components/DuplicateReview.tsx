import { useMemo, useRef } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Check, Copy, ShieldCheck } from 'lucide-react';
import type { DuplicateGroup, FileEntry } from '../lib/bindings';
import { FileGlyph, readableSize } from './FileList';

interface DuplicateReviewProps {
  groups: DuplicateGroup[];
  selectedPaths: string[];
  onToggle: (path: string) => void;
}

interface DuplicateFile {
  entry: FileEntry;
  groupNumber: number;
  copyCount: number;
  keep: boolean;
}

export function DuplicateReview({ groups, selectedPaths, onToggle }: DuplicateReviewProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const duplicates = useMemo<DuplicateFile[]>(() => groups.flatMap((group, groupIndex) =>
    group.files.map((entry, fileIndex) => ({
      entry,
      groupNumber: groupIndex + 1,
      copyCount: group.files.length,
      keep: fileIndex === 0,
    }))), [groups]);
  const virtualizer = useVirtualizer({
    count: duplicates.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 62,
    overscan: 8,
  });

  if (duplicates.length === 0) {
    return (
      <div className="clean-empty-state">
        <div className="success-mark"><Check size={21} /></div>
        <div><strong>No identical files found</strong><p>Your personal folders are looking tidy.</p></div>
      </div>
    );
  }

  return (
    <div className="duplicate-review">
      <div className="duplicate-notice"><ShieldCheck size={16} /><span>One copy in every matching set is protected. Select only the extra copies you want to recycle.</span></div>
      <div className="duplicate-table-head"><span>File</span><span>Match set</span><span>Size</span><span>Keep</span></div>
      <div ref={scrollRef} className="duplicate-scroll">
        <div className="virtual-spacer" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((row) => {
            const duplicate = duplicates[row.index];
            if (!duplicate) return null;
            const { entry } = duplicate;
            const checked = selectedPaths.includes(entry.path);
            return (
              <div className={`duplicate-row${checked ? ' is-selected' : ''}`} key={`${duplicate.groupNumber}:${entry.path}`} style={{ transform: `translateY(${row.start}px)` }}>
                <div className="duplicate-file-name"><FileGlyph entry={entry} /><div><strong title={entry.name}>{entry.name}</strong><small title={entry.path}>{entry.path}</small></div></div>
                <span className="match-pill"><Copy size={12} /> Set {duplicate.groupNumber} · {duplicate.copyCount} copies</span>
                <span className="duplicate-size">{readableSize(entry.size)}</span>
                {duplicate.keep ? (
                  <span className="keep-badge"><ShieldCheck size={14} /> Keep</span>
                ) : (
                  <label className="duplicate-select"><input type="checkbox" checked={checked} onChange={() => onToggle(entry.path)} aria-label={`Select extra copy ${entry.name}`} /><span>Select</span></label>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
