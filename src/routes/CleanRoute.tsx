import { useEffect, useState } from 'react';
import { ArrowUpRight, Check, FolderX, LockKeyhole, RefreshCcw, Sparkles, X, XCircle } from 'lucide-react';
import { CleanCards } from '../components/clean/CleanCards';
import { ConfirmCleanSheet, type ConfirmRequest } from '../components/clean/ConfirmCleanSheet';
import { FreedResult } from '../components/clean/FreedResult';
import { readableSize } from '../components/FileList';
import { totalReclaimable, useCleanStore } from '../stores/clean-store';

interface CleanRouteProps {
  desktopAvailable: boolean;
  /** Takes the user to the skipped-folder list, where the exclusions live too. */
  onOpenSettings: () => void;
}

/** The Clean tab: six cards, a confirm sheet for every removal, and the result banner. */
export function CleanRoute({ desktopAvailable, onOpenSettings }: CleanRouteProps) {
  const summary = useCleanStore((state) => state.summary);
  const loading = useCleanStore((state) => state.loading);
  const scanning = useCleanStore((state) => state.scanning);
  const busy = useCleanStore((state) => state.busy);
  const error = useCleanStore((state) => state.error);
  const notice = useCleanStore((state) => state.notice);
  const duplicates = useCleanStore((state) => state.duplicates);
  const result = useCleanStore((state) => state.result);
  const load = useCleanStore((state) => state.load);
  const scanDuplicates = useCleanStore((state) => state.scanDuplicates);
  const cleanPaths = useCleanStore((state) => state.cleanPaths);
  const cleanJunk = useCleanStore((state) => state.cleanJunk);
  const uninstall = useCleanStore((state) => state.uninstall);
  const clearResult = useCleanStore((state) => state.clearResult);
  const setNotice = useCleanStore((state) => state.setNotice);
  const clearError = useCleanStore((state) => state.clearError);
  const [request, setRequest] = useState<ConfirmRequest | null>(null);

  useEffect(() => {
    if (desktopAvailable) void load();
  }, [desktopAvailable, load]);

  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(null), 4200);
    return () => window.clearTimeout(timeout);
  }, [notice, setNotice]);

  const reclaimable = totalReclaimable(summary, duplicates);
  const reviewed = summary?.indexedFiles ?? 0;
  // Entries the walk could not read at all. Windows keeps these away from the
  // current user more often than anything else, so say so rather than hiding it.
  const unreadable = summary?.cards.reduce((total, card) => total + card.skipped, 0) ?? 0;

  return (
    <div className="clean-page">
      {result && <FreedResult result={result} onDismiss={clearResult} />}

      <section className="clean-hero">
        <div className="hero-copy">
          <span className="hero-kicker"><Sparkles size={14} /> A FRESH START</span>
          <h2>Make space for<br /><em>what's next.</em></h2>
          <p>
            {summary
              ? `Sift reviewed ${reviewed.toLocaleString()} indexed files and found ${readableSize(reclaimable)} you could take back. Nothing is removed until you confirm it.`
              : 'Sift looks through temporary files, browser caches, duplicates, large files and unused apps — then lets you decide what goes.'}
          </p>
          <button
            className="primary-button scan-button"
            type="button"
            disabled={!desktopAvailable || loading || scanning}
            onClick={() => void load()}
          >
            {loading ? <span className="button-spinner" /> : <RefreshCcw size={15} />}
            {loading ? 'Looking through your folders…' : summary ? 'Check again' : 'Review my storage'}
            {!loading && <ArrowUpRight size={16} />}
          </button>
          <div className="hero-footnote"><LockKeyhole size={13} /> Runs privately on this PC · Everything you remove goes to the Recycle Bin</div>
        </div>
        <div className="hero-art" aria-hidden="true">
          <div className="orb orb-one" /><div className="orb orb-two" />
          <div className="art-paper paper-back"><span /><span /><span /></div>
          <div className="art-paper paper-front"><div className="paper-check"><Check size={22} strokeWidth={2.3} /></div><b>Looking good</b><span>YOUR FILES, YOUR WAY</span></div>
          <div className="art-sparkle sparkle-one">✦</div><div className="art-sparkle sparkle-two">✳</div>
        </div>
      </section>

      {error && (
        <div className="alert-banner error-banner" role="alert">
          <XCircle size={18} /><span>{error}</span>
          <button type="button" aria-label="Dismiss error" onClick={clearError}><X size={15} /></button>
        </div>
      )}
      {notice && (
        <div className="alert-banner notice-banner" role="status">
          <Check size={18} /><span>{notice}</span>
          <button type="button" aria-label="Dismiss notification" onClick={() => setNotice(null)}><X size={15} /></button>
        </div>
      )}

      {summary ? (
        <CleanCards
          summary={summary}
          duplicates={duplicates}
          scanning={scanning}
          desktopAvailable={desktopAvailable}
          onReview={setRequest}
          onScanDuplicates={() => void scanDuplicates()}
        />
      ) : (
        <div className="clean-loading" role="status">
          {loading ? <span className="button-spinner" /> : null}
          <span>{loading ? 'Measuring what can be cleaned…' : 'Connect the Windows app to review your storage.'}</span>
        </div>
      )}

      {unreadable > 0 && (
        <div className="permission-note" role="status">
          <FolderX size={15} />
          <span>
            {unreadable.toLocaleString()} item{unreadable === 1 ? '' : 's'} could not be read, usually because Windows will not give Sift
            permission. Nothing was changed; the rest of the numbers are still accurate.
          </span>
          <button type="button" className="text-button" onClick={onOpenSettings}>See the list</button>
        </div>
      )}

      <p className="clean-footnote">
        Junk, duplicates, large files, old downloads and screenshots are moved to the Recycle Bin.
        Emptying the Recycle Bin and uninstalling an app are the two actions that cannot be undone here.
      </p>

      {request && (
        <ConfirmCleanSheet
          request={request}
          busy={busy}
          desktopAvailable={desktopAvailable}
          onClose={() => setRequest(null)}
          onConfirmPaths={(paths, label) => {
            setRequest(null);
            void cleanPaths(paths, label);
          }}
          onConfirmJunk={(groupIds, expectedBytes) => {
            setRequest(null);
            void cleanJunk(groupIds, expectedBytes);
          }}
          onUninstall={(app) => void uninstall(app)}
        />
      )}
    </div>
  );
}
