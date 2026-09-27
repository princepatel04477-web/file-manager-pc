import { ArrowDownToLine, ArrowUpRight, Camera, Copy, HardDrive, LayoutGrid, ScanSearch, Trash2 } from 'lucide-react';
import type { CleanCard, CleanSummary, DuplicateReport } from '../../lib/bindings';
import { readableSize } from '../FileList';
import type { ConfirmRequest } from './ConfirmCleanSheet';
import { cardIsActionable } from './selection';

const cardIcons = {
  junk: Trash2,
  duplicates: Copy,
  'large-files': HardDrive,
  'old-downloads': ArrowDownToLine,
  'old-screenshots': Camera,
  'unused-apps': LayoutGrid,
} as const;

const actionLabels: Record<string, string> = {
  junk: 'Review junk',
  duplicates: 'Review copies',
  'large-files': 'Choose files',
  'old-downloads': 'Choose files',
  'old-screenshots': 'Choose files',
  'unused-apps': 'Manage apps',
};

interface CleanCardsProps {
  summary: CleanSummary;
  duplicates: DuplicateReport | null;
  scanning: boolean;
  desktopAvailable: boolean;
  onReview: (request: ConfirmRequest) => void;
  onScanDuplicates: () => void;
}

/** The six cards, Files by Google style: each one states how much it can give back. */
export function CleanCards({ summary, duplicates, scanning, desktopAvailable, onReview, onScanDuplicates }: CleanCardsProps) {
  return (
    <div className="clean-card-grid">
      {summary.cards.map((card: CleanCard) => {
        const Icon = cardIcons[card.id as keyof typeof cardIcons] ?? Trash2;
        const isDuplicates = card.id === 'duplicates';
        const scanned = isDuplicates && duplicates !== null;
        const bytes = scanned && duplicates ? duplicates.reclaimableBytes : card.reclaimableBytes;
        const count = scanned && duplicates ? duplicates.sets.length : card.itemCount;
        const countLabel = isDuplicates
          ? scanned && duplicates
            ? `${duplicates.sets.length} duplicate set${duplicates.sets.length === 1 ? '' : 's'}`
            : 'Not scanned yet'
          : `${count.toLocaleString()} item${count === 1 ? '' : 's'}${card.truncated ? '+' : ''}`;
        const actionable = cardIsActionable(card, duplicates !== null) && desktopAvailable;

        return (
          <article key={card.id} className={`clean-card${actionable ? '' : ' is-quiet'}`}>
            <header className="clean-card-head">
              <span className={`clean-card-icon tone-${card.id}`}><Icon size={17} /></span>
              <div>
                <h3>{card.title}</h3>
                <p>{card.description}</p>
              </div>
            </header>
            <div className="clean-card-metric">
              <strong>
                {bytes > 0
                  ? `Free up ${readableSize(bytes)}`
                  : isDuplicates && !scanned
                    ? 'Scan to find out'
                    // Apps with no EstimatedSize have items but nothing measurable to claim.
                    : count > 0
                      ? `${count.toLocaleString()} to review`
                      : 'Nothing to clear'}
              </strong>
              <span>{countLabel}</span>
            </div>
            {isDuplicates && !scanned ? (
              <button type="button" className="clean-card-action" onClick={onScanDuplicates} disabled={!desktopAvailable || scanning}>
                {scanning ? <span className="button-spinner" /> : <ScanSearch size={15} />}
                {scanning ? 'Comparing file contents…' : 'Scan for duplicates'}
              </button>
            ) : (
              <button
                type="button"
                className="clean-card-action"
                disabled={!actionable}
                onClick={() => {
                  if (isDuplicates && duplicates) onReview({ kind: 'duplicates', card, sets: duplicates.sets });
                  else if (card.action === 'deleteJunk') onReview({ kind: 'junk', card });
                  else if (card.action === 'uninstall') onReview({ kind: 'apps', card, apps: summary.apps });
                  else onReview({ kind: 'files', card });
                }}
              >
                {actionLabels[card.id] ?? 'Review'}
                <ArrowUpRight size={14} />
              </button>
            )}
            {card.skipped > 0 && <p className="clean-card-skipped">{card.skipped.toLocaleString()} entries skipped</p>}
          </article>
        );
      })}
    </div>
  );
}
