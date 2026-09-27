import { useEffect, useRef, useState } from 'react';
import { X } from 'lucide-react';
import type { FileEntry } from '../lib/bindings';
import { stemOf, validateFileName } from '../lib/preview';

interface RenameDialogProps {
  entry: FileEntry;
  busy: boolean;
  onSubmit: (name: string) => void;
  onClose: () => void;
}

/** Inline rename triggered by F2 or the context menu. */
export function RenameDialog({ entry, busy, onSubmit, onClose }: RenameDialogProps) {
  const [name, setName] = useState(entry.name);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const node = inputRef.current;
    if (!node) return;
    node.focus();
    node.setSelectionRange(0, stemOf(entry.name));
  }, [entry.name]);

  function submit() {
    const next = name.trim();
    const problem = validateFileName(next);
    if (problem) {
      setError(problem);
      return;
    }
    if (next === entry.name) {
      onClose();
      return;
    }
    setError(null);
    onSubmit(next);
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="modal-card rename-card" role="dialog" aria-modal="true" aria-labelledby="rename-title">
        <header className="modal-head">
          <div>
            <span className="section-overline">{entry.isDirectory ? 'RENAME FOLDER' : 'RENAME FILE'}</span>
            <h2 id="rename-title">Choose a new name</h2>
          </div>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}><X size={16} /></button>
        </header>
        <label className="rename-field">
          <span className="sr-only">New name</span>
          <input
            ref={inputRef}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setError(validateFileName(event.target.value.trim()));
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                submit();
              } else if (event.key === 'Escape') {
                event.preventDefault();
                onClose();
              }
            }}
          />
        </label>
        {error && <p className="rename-error" role="alert">{error}</p>}
        <footer className="modal-foot">
          <button type="button" className="secondary-button" onClick={onClose} disabled={busy}>Cancel</button>
          <button type="button" className="primary-button" onClick={submit} disabled={busy || Boolean(error)}>
            {busy ? <span className="button-spinner" /> : null}Rename
          </button>
        </footer>
      </div>
    </div>
  );
}
