import { useState } from 'react';
import { AlertTriangle, ArrowRightLeft, Copy, MinusCircle, PlusCircle, X } from 'lucide-react';
import type { ConflictAction, ConflictDecision } from '../lib/bindings';
import type { ConflictRequest } from '../stores/ops-store';
import { readableSize } from './FileList';

interface ConflictDialogProps {
  request: ConflictRequest;
  busy: boolean;
  onResolve: (decisions: ConflictDecision[], defaultAction: ConflictAction) => void;
  onCancel: () => void;
}

const blockedCopy: Record<string, string> = {
  outsideUserFiles: 'outside your user folders',
  missing: 'no longer available',
  reparsePoint: 'a Windows link',
  cloudOnly: 'online-only',
  sameItem: 'already in this folder',
  insideItself: 'inside itself',
  invalidName: 'not a valid name',
  destinationUnavailable: 'destination unavailable',
};

function when(timestamp: number | null): string {
  if (timestamp === null) return 'unknown date';
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' }).format(timestamp * 1000);
}

function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/** Replace / skip / keep both, per item or for the whole batch. */
export function ConflictDialog({ request, busy, onResolve, onCancel }: ConflictDialogProps) {
  const [defaultAction, setDefaultAction] = useState<ConflictAction>('keepBoth');
  const [overrides, setOverrides] = useState<Record<string, ConflictAction>>({});
  const choices: Array<{ value: ConflictAction; label: string; hint: string; icon: typeof Copy }> = [
    { value: 'replace', label: 'Replace', hint: 'Overwrite the existing item', icon: ArrowRightLeft },
    { value: 'skip', label: 'Skip', hint: 'Leave the existing item alone', icon: MinusCircle },
    { value: 'keepBoth', label: 'Keep both', hint: 'Save the new one as “name (1)”', icon: PlusCircle },
  ];

  function submit() {
    const decisions = Object.entries(overrides)
      .filter(([, action]) => action !== defaultAction)
      .map(([source, action]) => ({ source, action }) satisfies ConflictDecision);
    onResolve(decisions, defaultAction);
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }}>
      <div className="modal-card conflict-card" role="dialog" aria-modal="true" aria-labelledby="conflict-title">
        <header className="modal-head">
          <div>
            <span className="section-overline">DESTINATION ALREADY HAS THESE</span>
            <h2 id="conflict-title">{request.kind === 'copy' ? 'Copy' : 'Move'} {request.plan.conflicts.length} conflicting item{request.plan.conflicts.length === 1 ? '' : 's'}</h2>
            <p>Into <strong>{folderName(request.destination)}</strong></p>
          </div>
          <button type="button" className="icon-button" aria-label="Close" onClick={onCancel}><X size={16} /></button>
        </header>

        <div className="conflict-choices" role="radiogroup" aria-label="Apply to all">
          {choices.map((choice) => {
            const Icon = choice.icon;
            return (
              <button
                type="button"
                key={choice.value}
                role="radio"
                aria-checked={defaultAction === choice.value}
                className={`conflict-choice${defaultAction === choice.value ? ' selected' : ''}`}
                onClick={() => setDefaultAction(choice.value)}
              >
                <Icon size={16} strokeWidth={1.8} />
                <strong>{choice.label}</strong>
                <small>{choice.hint}</small>
              </button>
            );
          })}
        </div>

        <ul className="conflict-list">
          {request.plan.conflicts.map((conflict) => {
            const action = overrides[conflict.source] ?? defaultAction;
            return (
              <li key={conflict.source}>
                <div className="conflict-item-copy">
                  <strong title={conflict.name}>{conflict.name}</strong>
                  <span>Incoming: {conflict.sourceIsDirectory ? 'folder' : readableSize(conflict.sourceSize)} · {when(conflict.sourceModifiedUnix)}</span>
                  <span>Existing: {conflict.existingIsDirectory ? 'folder' : readableSize(conflict.existingSize)} · {when(conflict.existingModifiedUnix)}</span>
                </div>
                <label className="compact-select">
                  <span className="sr-only">Action for {conflict.name}</span>
                  <select
                    value={action}
                    onChange={(event) => setOverrides((current) => ({ ...current, [conflict.source]: event.target.value as ConflictAction }))}
                  >
                    <option value="replace">Replace</option>
                    <option value="skip">Skip</option>
                    <option value="keepBoth">Keep both</option>
                  </select>
                </label>
              </li>
            );
          })}
        </ul>

        {request.plan.blocked.length > 0 && (
          <div className="conflict-blocked" role="status">
            <AlertTriangle size={14} />
            <span>{request.plan.blocked.length} item{request.plan.blocked.length === 1 ? '' : 's'} will not be {request.kind === 'copy' ? 'copied' : 'moved'} ({blockedCopy[request.plan.blocked[0]?.reason ?? ''] ?? 'not available'})</span>
          </div>
        )}

        <footer className="modal-foot">
          <button type="button" className="secondary-button" onClick={onCancel} disabled={busy}>Cancel</button>
          <button type="button" className="primary-button" onClick={submit} disabled={busy}>
            {busy ? <span className="button-spinner" /> : null}
            {request.kind === 'copy' ? 'Copy' : 'Move'} {request.plan.conflicts.length} item{request.plan.conflicts.length === 1 ? '' : 's'}
          </button>
        </footer>
      </div>
    </div>
  );
}
