import { Ban, Check, Copy, FolderInput, Scissors, Trash2, XCircle } from 'lucide-react';
import type { OperationProgress, OpsKind } from '../lib/bindings';
import { readableSize } from './FileList';

interface OperationProgressCardProps {
  operations: OperationProgress[];
  onCancel: (jobId: string) => void;
}

const kindCopy: Record<OpsKind, { label: string; icon: typeof Copy }> = {
  copy: { label: 'Copying', icon: Copy },
  move: { label: 'Moving', icon: FolderInput },
  rename: { label: 'Renaming', icon: Scissors },
  delete: { label: 'Moving to Recycle Bin', icon: Trash2 },
};

function percent(operation: OperationProgress): number {
  if (operation.bytesTotal > 0) return Math.min(100, Math.round((operation.bytesDone / operation.bytesTotal) * 100));
  if (operation.itemsTotal > 0) return Math.min(100, Math.round((operation.itemsDone / operation.itemsTotal) * 100));
  return 0;
}

/** Floating card for in-flight copy/move/delete jobs, with a working Cancel button. */
export function OperationProgressCard({ operations, onCancel }: OperationProgressCardProps) {
  if (operations.length === 0) return null;
  return (
    <div className="operation-card-stack" role="status" aria-live="polite">
      {operations.map((operation) => {
        const detail = kindCopy[operation.kind] ?? kindCopy.copy;
        const Icon = detail.icon;
        const done = percent(operation);
        return (
          <div className="operation-card" key={operation.jobId}>
            <div className="operation-card-head">
              <span className="operation-icon"><Icon size={14} strokeWidth={1.8} /></span>
              <strong>{detail.label}</strong>
              <span className="operation-count">{operation.itemsDone.toLocaleString()}/{operation.itemsTotal.toLocaleString()}</span>
              <button type="button" className="operation-cancel" aria-label={`Cancel ${detail.label.toLowerCase()}`} title="Cancel" onClick={() => onCancel(operation.jobId)}>
                <Ban size={14} />
              </button>
            </div>
            <div className="operation-track" role="img" aria-label={`${done}% complete`}><span style={{ width: `${done}%` }} /></div>
            <div className="operation-card-foot">
              <span title={operation.current || operation.destination}>
                {operation.current || operation.destination || 'Working…'}
              </span>
              {operation.bytesTotal > 0 && <span>{readableSize(operation.bytesDone)} of {readableSize(operation.bytesTotal)}</span>}
            </div>
          </div>
        );
      })}
      <p className="operation-hint"><Check size={11} /> Cancelling stops after the current file; finished items stay.</p>
    </div>
  );
}

export function OperationFailedNote({ message, onDismiss }: { message: string; onDismiss: () => void }) {
  return (
    <div className="operation-failed" role="alert">
      <XCircle size={14} />
      <span>{message}</span>
      <button type="button" aria-label="Dismiss" onClick={onDismiss}><XCircle size={13} /></button>
    </div>
  );
}
