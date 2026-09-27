import { useEffect, useState } from 'react';
import { Star, X } from 'lucide-react';
import { commands, type FileProperties } from '../lib/bindings';
import { readableSize } from './FileList';

interface PropertiesDialogProps {
  path: string;
  favorite: boolean;
  onToggleFavorite: (path: string, name: string, isDirectory: boolean) => void;
  onClose: () => void;
}

function when(timestamp: number | null): string {
  if (timestamp === null) return '—';
  return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(timestamp * 1000);
}

/** Metadata sheet gathered in Rust; no file contents are read. */
export function PropertiesDialog({ path, favorite, onToggleFavorite, onClose }: PropertiesDialogProps) {
  const [properties, setProperties] = useState<FileProperties | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    setProperties(null);
    setError(null);
    commands
      .describePath(path)
      .then((value) => {
        if (active) setProperties(value);
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setError(failure instanceof Error ? failure.message : typeof failure === 'string' ? failure : 'Properties are unavailable.');
      });
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      active = false;
      window.removeEventListener('keydown', onKey);
    };
  }, [onClose, path]);

  const rows: Array<[string, string]> = properties
    ? [
        ['Type', properties.kind],
        ['Location', properties.parent || '—'],
        ['Size', properties.isDirectory ? `${properties.childCount?.toLocaleString() ?? 0} items` : readableSize(properties.size)],
        ['Modified', when(properties.modifiedUnix)],
        ['Created', when(properties.createdUnix)],
        ['Accessed', when(properties.accessedUnix)],
        ['Drive', properties.drive],
        ['Attributes', properties.attributeLabels.join(', ')],
      ]
    : [];

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="modal-card properties-card" role="dialog" aria-modal="true" aria-labelledby="properties-title">
        <header className="modal-head">
          <div>
            <span className="section-overline">PROPERTIES</span>
            <h2 id="properties-title">{properties?.name ?? path.split(/[\\/]/).filter(Boolean).pop() ?? 'Item'}</h2>
            <p>{properties?.path ?? path}</p>
          </div>
          <button type="button" className="icon-button" aria-label="Close" onClick={onClose}><X size={16} /></button>
        </header>

        {error ? <p className="rename-error" role="alert">{error}</p> : (
          <dl className="properties-grid">
            {rows.map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd title={value}>{value}</dd>
              </div>
            ))}
          </dl>
        )}

        <footer className="modal-foot">
          {properties && (
            <button
              type="button"
              className={`secondary-button${favorite ? ' pinned' : ''}`}
              onClick={() => onToggleFavorite(properties.path, properties.name, properties.isDirectory)}
            >
              <Star size={15} fill={favorite ? 'currentColor' : 'none'} />
              {favorite ? 'Remove from Favorites' : 'Add to Favorites'}
            </button>
          )}
          <button type="button" className="primary-button" onClick={onClose}>Close</button>
        </footer>
      </div>
    </div>
  );
}
