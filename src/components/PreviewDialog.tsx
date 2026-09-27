import { useEffect, useMemo, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ArrowUpRight, Cloud, ExternalLink, FileWarning, X } from 'lucide-react';
import { commands, type FileEntry, type TextPreview } from '../lib/bindings';
import { previewKindFor } from '../lib/preview';
import { readableSize } from './FileList';

interface PreviewDialogProps {
  entry: FileEntry;
  onOpenWithApp: (path: string) => void;
  onClose: () => void;
}

/** Built-in viewer: images, video, and audio over the Tauri asset protocol, PDF in an
 * embedded frame, and text/code read through a capped Rust command. */
export function PreviewDialog({ entry, onOpenWithApp, onClose }: PreviewDialogProps) {
  const kind = useMemo(() => previewKindFor(entry), [entry]);
  const [text, setText] = useState<TextPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const assetUrl = useMemo(() => (entry.path ? convertFileSrc(entry.path) : ''), [entry.path]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  useEffect(() => {
    if (kind !== 'text') {
      setText(null);
      return;
    }
    let active = true;
    setError(null);
    commands
      .readTextPreview(entry.path)
      .then((value) => {
        if (active) setText(value);
      })
      .catch((failure: unknown) => {
        if (!active) return;
        setError(failure instanceof Error ? failure.message : typeof failure === 'string' ? failure : 'This file could not be previewed.');
      });
    // Opening a preview is a real open, so it belongs in Recents.
    void commands.recordRecent(entry.path, entry.name).catch(() => undefined);
    return () => {
      active = false;
    };
  }, [entry.name, entry.path, kind]);

  return (
    <div className="modal-backdrop preview-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className="modal-card preview-card" role="dialog" aria-modal="true" aria-labelledby="preview-title">
        <header className="modal-head">
          <div>
            <span className="section-overline">PREVIEW</span>
            <h2 id="preview-title">{entry.name}</h2>
            <p>{entry.isDirectory ? 'Folder' : readableSize(entry.size)}{entry.isCloudPlaceholder ? ' · online only' : ''}</p>
          </div>
          <div className="preview-head-actions">
            <button type="button" className="secondary-button" onClick={() => onOpenWithApp(entry.path)}>
              <ExternalLink size={14} /> Open in app
            </button>
            <button type="button" className="icon-button" aria-label="Close preview" onClick={onClose}><X size={16} /></button>
          </div>
        </header>

        <div className="preview-stage">
          {entry.isCloudPlaceholder ? (
            <div className="preview-unavailable"><Cloud size={22} /><strong>This file is online-only</strong><p>Sift will not download it for you. Choose “Open in app” after Windows has synced it.</p></div>
          ) : kind === 'image' ? (
            <img src={assetUrl} alt={entry.name} />
          ) : kind === 'video' ? (
            <video src={assetUrl} controls preload="metadata" playsInline />
          ) : kind === 'audio' ? (
            <div className="preview-audio"><audio src={assetUrl} controls preload="metadata" /></div>
          ) : kind === 'pdf' ? (
            <iframe src={assetUrl} title={entry.name} />
          ) : kind === 'text' ? (
            error ? (
              <div className="preview-unavailable"><FileWarning size={22} /><strong>Not previewable</strong><p>{error}</p></div>
            ) : text ? (
              <>
                <pre className="preview-text"><code>{text.content}</code></pre>
                {text.truncated && <p className="preview-truncated">Showing the first {readableSize(text.size > 2 * 1024 * 1024 ? 2 * 1024 * 1024 : text.size)} of {readableSize(text.size)}.</p>}
              </>
            ) : (
              <div className="list-loading" role="status" aria-label="Loading preview"><span /><span /><span /></div>
            )
          ) : (
            <div className="preview-unavailable">
              <ArrowUpRight size={22} />
              <strong>Sift does not preview this file type</strong>
              <p>Open it in its own app; nothing is uploaded or converted.</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
