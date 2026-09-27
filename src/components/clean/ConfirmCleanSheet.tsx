import { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, Copy, ShieldCheck, Trash2, X } from 'lucide-react';
import type { CleanCard, CleanItem, DuplicateSet, InstalledApp } from '../../lib/bindings';
import { readableSize } from '../FileList';
import {
  defaultDuplicateSelection,
  groupBytes,
  groupItems,
  keepsOneCopyPerSet,
  sumBytes,
} from './selection';

export type ConfirmRequest =
  | { kind: 'junk'; card: CleanCard }
  | { kind: 'files'; card: CleanCard }
  | { kind: 'duplicates'; card: CleanCard; sets: DuplicateSet[] }
  | { kind: 'apps'; card: CleanCard; apps: InstalledApp[] };

interface ConfirmCleanSheetProps {
  request: ConfirmRequest;
  busy: boolean;
  desktopAvailable: boolean;
  onClose: () => void;
  onConfirmPaths: (paths: string[], label: string) => void;
  onConfirmJunk: (groupIds: string[], expectedBytes: number) => void;
  onUninstall: (app: InstalledApp) => void;
}

function toggle<T>(values: T[], value: T): T[] {
  return values.includes(value) ? values.filter((item) => item !== value) : [...values, value];
}

function ageLabel(item: CleanItem): string {
  if (item.daysOld === null) return '';
  if (item.daysOld < 1) return 'today';
  return `${item.daysOld.toLocaleString()} days old`;
}

/**
 * The step every Clean action goes through: what will be removed, how much space it
 * gives back, and one button to do it. Nothing here deletes permanently.
 */
export function ConfirmCleanSheet({
  request,
  busy,
  desktopAvailable,
  onClose,
  onConfirmPaths,
  onConfirmJunk,
  onUninstall,
}: ConfirmCleanSheetProps) {
  const initialPaths = useMemo(() => {
    if (request.kind === 'duplicates') return defaultDuplicateSelection(request.sets);
    if (request.kind === 'files') return request.card.items.map((item) => item.path);
    return [] as string[];
  }, [request]);
  const initialGroups = useMemo(
    () => (request.kind === 'junk' ? request.card.groups.map((group) => group.id) : []),
    [request],
  );
  const [selectedPaths, setSelectedPaths] = useState<string[]>(initialPaths);
  const [selectedGroups, setSelectedGroups] = useState<string[]>(initialGroups);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  if (request.kind === 'apps') {
    return (
      <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
        <div className="modal-card clean-sheet" role="dialog" aria-modal="true" aria-labelledby="clean-sheet-title">
          <header className="modal-head">
            <div>
              <span className="section-overline">UNUSED APPS</span>
              <h2 id="clean-sheet-title">{request.card.title}</h2>
            </div>
            <button type="button" className="icon-button" aria-label="Close" onClick={onClose}><X size={16} /></button>
          </header>
          <p className="clean-sheet-note">Uninstalling opens the app's own uninstaller. Sift never removes a program by itself.</p>
          <ul className="clean-sheet-list app-list">
            {request.apps.map((app) => (
              <li key={`${app.source}:${app.name}`} className="clean-sheet-row">
                <div className="row-main">
                  <strong>{app.name}</strong>
                  <span>
                    {app.sizeBytes > 0 ? readableSize(app.sizeBytes) : 'Size unknown'}
                    {app.installDate ? ` · installed ${app.installDate}` : ''}
                    {app.publisher ? ` · ${app.publisher}` : ''}
                  </span>
                </div>
                <button
                  type="button"
                  className="secondary-button row-action"
                  disabled={!desktopAvailable || busy || !app.uninstallCommand}
                  onClick={() => onUninstall(app)}
                >
                  Uninstall
                </button>
              </li>
            ))}
            {request.apps.length === 0 && <li className="clean-sheet-empty">No installed programs were found.</li>}
          </ul>
          <footer className="modal-foot">
            <button type="button" className="secondary-button" onClick={onClose}>Close</button>
          </footer>
        </div>
      </div>
    );
  }

  const isJunk = request.kind === 'junk';
  const groups = isJunk ? request.card.groups : [];
  const onlyRecycleBin = isJunk && selectedGroups.length > 0 && selectedGroups.every((id) => id === 'recycle-bin');
  const bytes = isJunk
    ? groupBytes(groups, selectedGroups)
    : sumBytes(request.card.items, selectedPaths);
  const items = isJunk ? groupItems(groups, selectedGroups) : selectedPaths.length;
  const sets = request.kind === 'duplicates' ? request.sets : [];
  const wouldEmptyASet = request.kind === 'duplicates' && !keepsOneCopyPerSet(sets, selectedPaths);
  const canConfirm = !busy && desktopAvailable && items > 0 && !wouldEmptyASet;

  function confirm() {
    if (!canConfirm) return;
    if (isJunk) {
      onConfirmJunk(selectedGroups, bytes);
      return;
    }
    onConfirmPaths(selectedPaths, request.card.title);
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="modal-card clean-sheet" role="dialog" aria-modal="true" aria-labelledby="clean-sheet-title">
        <header className="modal-head">
          <div>
            <span className="section-overline">REVIEW BEFORE YOU REMOVE</span>
            <h2 id="clean-sheet-title">{request.card.title}</h2>
          </div>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}><X size={16} /></button>
        </header>

        {isJunk ? (
          <>
            <p className="clean-sheet-note">
              {onlyRecycleBin
                ? 'Emptying the Recycle Bin removes those files for good. Everything else here is moved to the Recycle Bin first.'
                : 'Junk is moved to the Recycle Bin, so you can bring any of it back. Files Windows still has open are skipped.'}
            </p>
            <ul className="clean-sheet-list">
              {groups.map((group) => {
                const checked = selectedGroups.includes(group.id);
                return (
                  <li key={group.id}>
                    <label className={`clean-sheet-row${checked ? ' selected' : ''}`}>
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={() => setSelectedGroups((current) => toggle(current, group.id))}
                      />
                      <div className="row-main">
                        <strong>{group.label}</strong>
                        <span>{group.itemCount.toLocaleString()} item{group.itemCount === 1 ? '' : 's'}</span>
                      </div>
                      <span className="row-size">{readableSize(group.reclaimableBytes)}</span>
                    </label>
                  </li>
                );
              })}
            </ul>
          </>
        ) : request.kind === 'duplicates' ? (
          <>
            <p className="clean-sheet-note">
              One copy of every set stays put. Uncheck anything you would rather keep — Sift will not
              remove the last copy of a file.
            </p>
            <ul className="clean-sheet-list duplicate-list">
              {sets.map((set) => (
                <li key={set.fingerprint} className="duplicate-set">
                  <div className="duplicate-set-head">
                    <Copy size={14} />
                    <span>{readableSize(set.size)} each · {set.files.length} copies · {readableSize(set.reclaimableBytes)} reclaimable</span>
                  </div>
                  {set.files.map((file) => {
                    const checked = selectedPaths.includes(file.path);
                    return (
                      <label key={file.path} className={`clean-sheet-row${checked ? ' selected' : ''}${file.original ? ' protected' : ''}`}>
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() => setSelectedPaths((current) => toggle(current, file.path))}
                        />
                        <div className="row-main">
                          <strong>{file.name}</strong>
                          <span title={file.path}>{file.path}</span>
                        </div>
                        <span className="row-size">{file.original ? 'Kept' : readableSize(file.size)}</span>
                      </label>
                    );
                  })}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <>
            <p className="clean-sheet-note">{request.card.description} Removed files go to the Recycle Bin.</p>
            <ul className="clean-sheet-list">
              {request.card.items.map((item) => {
                const checked = selectedPaths.includes(item.path);
                return (
                  <li key={item.path}>
                    <label className={`clean-sheet-row${checked ? ' selected' : ''}`}>
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={() => setSelectedPaths((current) => toggle(current, item.path))}
                      />
                      <div className="row-main">
                        <strong>{item.name}</strong>
                        <span title={item.path}>{item.path}</span>
                      </div>
                      <span className="row-size">
                        {readableSize(item.size)}
                        {ageLabel(item) ? <em>{ageLabel(item)}</em> : null}
                      </span>
                    </label>
                  </li>
                );
              })}
              {request.card.items.length === 0 && <li className="clean-sheet-empty">Nothing to show here right now.</li>}
            </ul>
          </>
        )}

        {request.card.truncated && (
          <p className="clean-sheet-footnote">Showing the first {request.card.items.length.toLocaleString()} of {request.card.itemCount.toLocaleString()} items.</p>
        )}
        {request.card.skipped > 0 && (
          <p className="clean-sheet-footnote">{request.card.skipped.toLocaleString()} unreadable or protected entries were skipped.</p>
        )}
        {wouldEmptyASet && (
          <p className="clean-sheet-warning" role="alert"><AlertTriangle size={14} /> Every copy in at least one set is selected. Leave one unchecked.</p>
        )}
        <footer className="modal-foot clean-sheet-foot">
          <span className="clean-sheet-total">
            <ShieldCheck size={15} />
            {items === 0
              ? 'Nothing selected'
              : `${items.toLocaleString()} item${items === 1 ? '' : 's'} · free up ${readableSize(bytes)}`}
          </span>
          <div className="clean-sheet-actions">
            <button type="button" className="secondary-button" onClick={onClose} disabled={busy}>Cancel</button>
            <button type="button" className="primary-button danger-primary" onClick={confirm} disabled={!canConfirm}>
              {busy ? <span className="button-spinner" /> : <Trash2 size={16} />}
              {onlyRecycleBin ? 'Empty Recycle Bin' : `Move ${items.toLocaleString()} to Recycle Bin`}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
