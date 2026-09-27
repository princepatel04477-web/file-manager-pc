import { useEffect, useMemo, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import {
  ArrowDownToLine,
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  CircleUserRound,
  Cloud,
  Copy,
  FileClock,
  Folder,
  History,
  FolderOpen,
  Image as ImageIcon,
  LayoutDashboard,
  LockKeyhole,
  Maximize2,
  Minimize2,
  Moon,
  Search,
  Send,
  ShieldCheck,
  Sparkles,
  Star,
  Sun,
  SunMoon,
  Trash2,
  X,
  XCircle,
} from 'lucide-react';
import { FileGlyph, readableSize } from './components/FileList';
import { OperationProgressCard } from './components/OperationProgress';
import { BrowseRoute } from './routes/BrowseRoute';
import { CleanRoute } from './routes/CleanRoute';
import { SharePcPanel } from './share/SharePcPanel';
import { commands, type FileEntry, type IndexedEntry, type PcShareSession, type ShareLink } from './lib/bindings';
import { useIndexProgress } from './hooks/useIndexProgress';
import { useOpsProgress } from './hooks/useOpsProgress';
import { useAppStore } from './stores/app-store';
import { useIndexStore } from './stores/index-store';
import { useOpsStore } from './stores/ops-store';

type Tab = 'clean' | 'browse' | 'share';
type ThemeMode = 'system' | 'light' | 'dark';

const tabCopy: Record<Tab, { title: string; subtitle: string }> = {
  clean: { title: 'Clean', subtitle: 'A little more room for what matters.' },
  browse: { title: 'Browse', subtitle: 'Everything in its place.' },
  share: { title: 'Share', subtitle: 'Send files, simply and privately.' },
};

function SkeletonPanel() {
  return (
    <div className="skeleton-panel" aria-label="Loading your files" role="status">
      <div className="skeleton-line skeleton-title" />
      <div className="skeleton-line skeleton-subtitle" />
      <div className="skeleton-grid"><span /><span /><span /><span /></div>
      <div className="skeleton-line skeleton-row" />
      <div className="skeleton-line skeleton-row" />
      <span className="sr-only">Loading your files…</span>
    </div>
  );
}

function indexedToFileEntry(entry: IndexedEntry): FileEntry {
  const categoryKinds: Record<string, string> = { images: 'image', videos: 'video', audio: 'audio', documents: 'document', archives: 'archive' };
  return {
    name: entry.name,
    path: entry.path,
    extension: entry.ext,
    kind: entry.isDirectory ? 'folder' : entry.isCloud ? 'cloud' : categoryKinds[entry.category] ?? 'file',
    isDirectory: entry.isDirectory,
    isCloudPlaceholder: entry.isCloud,
    size: entry.size,
    modifiedUnix: entry.mtime,
  };
}

function App() {
  const desktopAvailable = isTauri();
  const reducedMotion = useReducedMotion();
  const selectedPaths = useAppStore((state) => state.selectedPaths);
  const loading = useAppStore((state) => state.loading);
  const error = useAppStore((state) => state.error);
  const indexLocations = useIndexStore((state) => state.locations);
  const indexEntries = useIndexStore((state) => state.entries);
  const indexPath = useIndexStore((state) => state.activePath);
  const indexProgress = useIndexStore((state) => state.progress);
  const initializeIndex = useIndexStore((state) => state.initialize);
  const loadIndexDirectory = useIndexStore((state) => state.loadDirectory);
  useIndexProgress();
  useOpsProgress();
  const notice = useAppStore((state) => state.notice);
  const initialize = useAppStore((state) => state.initialize);
  const openDirectory = useAppStore((state) => state.openDirectory);
  const clearSelection = useAppStore((state) => state.clearSelection);
  const deletePaths = useOpsStore((state) => state.deletePaths);
  const operations = useOpsStore((state) => state.operations);
  const cancelOperation = useOpsStore((state) => state.cancel);
  const opsError = useOpsStore((state) => state.error);
  const opsNotice = useOpsStore((state) => state.notice);
  const clearOpsError = useOpsStore((state) => state.clearError);
  const setOpsNotice = useOpsStore((state) => state.setNotice);
  const favorites = useOpsStore((state) => state.favorites);
  const recents = useOpsStore((state) => state.recents);
  const loadFavorites = useOpsStore((state) => state.loadFavorites);
  const loadRecents = useOpsStore((state) => state.loadRecents);
  const setSelectedPaths = useAppStore((state) => state.setSelectedPaths);
  const openFile = useAppStore((state) => state.openFile);
  const setNotice = useAppStore((state) => state.setNotice);
  const clearError = useAppStore((state) => state.clearError);

  const [tab, setTab] = useState<Tab>('clean');
  const [query, setQuery] = useState('');
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [theme, setTheme] = useState<ThemeMode>(() => {
    const saved = window.localStorage.getItem('sift-theme');
    return saved === 'light' || saved === 'dark' ? saved : 'system';
  });
  const [shareLink, setShareLink] = useState<ShareLink | null>(null);
  const [pcShareSession, setPcShareSession] = useState<PcShareSession | null>(null);
  const [sharing, setSharing] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (desktopAvailable) {
      void initialize();
      void initializeIndex();
      void loadFavorites();
      void loadRecents();
    }
  }, [desktopAvailable, initialize, initializeIndex, loadFavorites, loadRecents]);

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const applyTheme = () => {
      const dark = theme === 'dark' || (theme === 'system' && media.matches);
      document.documentElement.classList.toggle('dark', dark);
      document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
    };
    applyTheme();
    media.addEventListener('change', applyTheme);
    window.localStorage.setItem('sift-theme', theme);
    return () => media.removeEventListener('change', applyTheme);
  }, [theme]);

  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(null), 4200);
    return () => window.clearTimeout(timeout);
  }, [notice, setNotice]);

  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    };
    window.addEventListener('keydown', focusSearch);
    return () => window.removeEventListener('keydown', focusSearch);
  }, []);

  const selectedEntries = useMemo(() => {
    const byPath = new Map(indexEntries.map((entry) => [entry.path, indexedToFileEntry(entry)]));
    return selectedPaths.map((path) => byPath.get(path)).filter((entry): entry is FileEntry => entry !== undefined);
  }, [indexEntries, selectedPaths]);

  const currentLocation = indexLocations.find((location) => location.path.toLowerCase() === indexPath?.toLowerCase());
  const pageMotion = reducedMotion ? { duration: 0 } : { duration: 0.24, ease: [0.22, 1, 0.36, 1] as const };

  async function handleTrash() {
    if (selectedPaths.length === 0) return;
    const confirmed = window.confirm(`Move ${selectedPaths.length} selected item${selectedPaths.length === 1 ? '' : 's'} to the Recycle Bin?`);
    if (!confirmed) return;
    await deletePaths(selectedPaths);
    clearSelection();
  }

  async function handleStartShare() {
    const first = selectedEntries.find((entry) => !entry.isDirectory && !entry.isCloudPlaceholder);
    if (!first) {
      setShareError('Select a downloaded file in Browse first. Cloud-only files cannot be sent.');
      return;
    }
    setSharing(true);
    setShareError(null);
    try {
      const link = await commands.startShare(first.path);
      setShareLink(link);
    } catch (shareFailure: unknown) {
      setShareError(shareFailure instanceof Error ? shareFailure.message : typeof shareFailure === 'string' ? shareFailure : 'Sift could not start sharing.');
    } finally {
      setSharing(false);
    }
  }

  async function handleStopShare() {
    try {
      await commands.stopShare();
      setShareLink(null);
      setCopied(false);
    } catch {
      setShareError('Sift could not stop the sharing session. Close the app to stop sharing immediately.');
    }
  }

  async function handleCopyLink() {
    if (!shareLink) return;
    try {
      await navigator.clipboard.writeText(shareLink.url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      setShareError('The link could not be copied. You can select and copy it instead.');
    }
  }

  function openFavorite(path: string, isDirectory: boolean) {
    setTab('browse');
    setQuery('');
    if (isDirectory) {
      void loadIndexDirectory(path);
      void openDirectory(path);
      return;
    }
    const parent = path.slice(0, Math.max(0, Math.max(path.lastIndexOf('\\'), path.lastIndexOf('/'))));
    if (parent) {
      void loadIndexDirectory(parent);
      void openDirectory(parent);
      setSelectedPaths([path]);
    }
  }

  function cycleTheme() {
    setTheme((value) => value === 'system' ? 'light' : value === 'light' ? 'dark' : 'system');
  }

  return (
    <div className="app-window">
      <header className="titlebar" data-tauri-drag-region>
        <div className="titlebar-brand" data-tauri-drag-region>
          <span className="brand-mark small"><Sparkles size={14} fill="currentColor" /></span>
          <span>Sift</span>
        </div>
        <div className="titlebar-center" data-tauri-drag-region>Private files, thoughtfully organized</div>
        <div className="window-controls">
          <button type="button" className="window-control" aria-label="Minimize" title="Minimize" onClick={() => { if (desktopAvailable) void getCurrentWindow().minimize(); }}><Minimize2 size={14} /></button>
          <button type="button" className="window-control" aria-label={maximized ? 'Restore' : 'Maximize'} title="Maximize or restore" onClick={() => {
            if (!desktopAvailable) return;
            void getCurrentWindow().toggleMaximize().then(() => getCurrentWindow().isMaximized()).then(setMaximized);
          }}>{maximized ? <Minimize2 size={14} /> : <Maximize2 size={13} />}</button>
          <button type="button" className="window-control close-control" aria-label="Close" title="Close" onClick={() => { if (desktopAvailable) void getCurrentWindow().close(); }}><X size={15} /></button>
        </div>
      </header>

      <div className="workspace">
        <aside className="sidebar">
          <div className="brand-lockup">
            <span className="brand-mark"><Sparkles size={18} fill="currentColor" /></span>
            <div><strong>Sift</strong><small>File manager</small></div>
          </div>
          <nav className="primary-nav" aria-label="Main navigation">
            <button type="button" className={`nav-item${tab === 'clean' ? ' active' : ''}`} onClick={() => setTab('clean')}><Sparkles size={18} /><span>Clean</span></button>
            <button type="button" className={`nav-item${tab === 'browse' ? ' active' : ''}`} onClick={() => setTab('browse')}><FolderOpen size={18} /><span>Browse</span></button>
            <button type="button" className={`nav-item${tab === 'share' ? ' active' : ''}`} onClick={() => setTab('share')}><Send size={18} /><span>Share</span></button>
          </nav>

          <div className="sidebar-divider" />
          <div className="sidebar-section-title"><span>YOUR FOLDERS</span></div>
          <nav className="folder-nav" aria-label="Your folders">
            {indexLocations.filter((location) => location.id !== 'home').map((location) => {
              const active = indexPath?.toLowerCase() === location.path.toLowerCase();
              const Icon = location.id === 'pictures' ? ImageIcon : location.id === 'downloads' ? ArrowDownToLine : location.id === 'desktop' ? LayoutDashboard : location.id === 'videos' ? Folder : location.id === 'music' ? Folder : FileClock;
              return <button key={location.id} type="button" className={`folder-nav-item${active ? ' active' : ''}`} onClick={() => { setTab('browse'); setQuery(''); void loadIndexDirectory(location.path); void openDirectory(location.path); }}><Icon size={17} /><span>{location.label}</span></button>;
            })}
            {indexLocations.length === 0 && <span className="sidebar-loading">Your folders will appear here</span>}
          </nav>

          {favorites.length > 0 && <>
            <div className="sidebar-divider" />
            <div className="sidebar-section-title"><span>FAVORITES</span></div>
            <nav className="folder-nav" aria-label="Favorites">
              {favorites.slice(0, 6).map((favorite) => (
                <button key={favorite.path} type="button" className="folder-nav-item" title={favorite.path} onClick={() => void openFavorite(favorite.path, favorite.isDirectory)}>
                  {favorite.isDirectory ? <Star size={16} /> : <Star size={16} />}
                  <span>{favorite.name}</span>
                </button>
              ))}
            </nav>
          </>}

          {recents.length > 0 && <>
            <div className="sidebar-divider" />
            <div className="sidebar-section-title"><span>RECENT</span></div>
            <nav className="folder-nav" aria-label="Recent files">
              {recents.slice(0, 4).map((recent) => (
                <button key={recent.path} type="button" className="folder-nav-item" title={recent.path} onClick={() => { setTab('browse'); void openFile(recent.path); }}>
                  <History size={16} /><span>{recent.name}</span>
                </button>
              ))}
            </nav>
          </>}
          <div className="sidebar-spacer" />
          <div className="privacy-card">
            <span className="privacy-icon"><LockKeyhole size={16} /></span>
            <div><strong>Private by design</strong><span>No cloud uploads, ever.</span></div>
          </div>
          <div className="sidebar-footer">
            <button className="footer-action" type="button" onClick={cycleTheme} title={`Theme: ${theme}`} aria-label={`Change theme. Current: ${theme}`}>
              {theme === 'system' ? <SunMoon size={17} /> : theme === 'light' ? <Sun size={17} /> : <Moon size={17} />}
              <span>Appearance</span><ChevronDown size={14} className="footer-chevron" />
            </button>
          </div>
        </aside>

        <main className="main-panel">
          <div className="topbar">
            <div className="page-heading"><span className="eyebrow">SIFT / {tab.toUpperCase()}</span><h1>{tabCopy[tab].title}</h1></div>
            <div className="topbar-actions">
              <label className="search-box">
                <Search size={17} />
                <input ref={searchInputRef} value={query} onChange={(event) => { setQuery(event.target.value); if (event.target.value) setTab('browse'); }} placeholder="Search your files" aria-label="Search your files" />
                {query ? <button type="button" aria-label="Clear search" onClick={() => setQuery('')}><X size={14} /></button> : <kbd>Ctrl K</kbd>}
              </label>
              {selectedPaths.length > 0 && tab !== 'share' && <button type="button" className="toolbar-button danger-button" onClick={() => void handleTrash()} disabled={!desktopAvailable || loading}><Trash2 size={16} /><span>Recycle</span><b>{selectedPaths.length}</b></button>}
              <div className="avatar-button" aria-label="Current Windows user" title="Your Windows profile"><CircleUserRound size={18} /></div>
            </div>
          </div>

          {!desktopAvailable && <div className="preview-banner"><span className="preview-live-dot" /><div><strong>Desktop preview</strong><span>Connect Sift for Windows to browse your own files. This preview never uses sample or cloud files.</span></div><span className="preview-version">WINDOWS APP</span></div>}
          {(error ?? opsError) && <div className="alert-banner error-banner" role="alert"><XCircle size={18} /><span>{error ?? opsError}</span><button type="button" onClick={() => { clearError(); clearOpsError(); }} aria-label="Dismiss error"><X size={15} /></button></div>}
          {(notice ?? opsNotice) && <div className="alert-banner notice-banner" role="status"><Check size={18} /><span>{notice ?? opsNotice}</span><button type="button" onClick={() => { setNotice(null); setOpsNotice(null); }} aria-label="Dismiss notification"><X size={15} /></button></div>}

          <div className="content-scroll">
            <AnimatePresence mode="wait" initial={false}>
              <motion.section key={tab} className="page-content" initial={{ opacity: 0, y: reducedMotion ? 0 : 7 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: reducedMotion ? 0 : -5 }} transition={pageMotion}>
                <div className="section-intro"><p>{tabCopy[tab].subtitle}</p>{tab === 'browse' && currentLocation && <span className="current-location"><Folder size={14} />{currentLocation.label}</span>}</div>
                {tab === 'browse' ? (
                  <BrowseRoute
                    desktopAvailable={desktopAvailable}
                    query={query}
                    onQueryChange={setQuery}
                    onShareRequest={(entry) => {
                      setSelectedPaths([entry.path]);
                      setTab('share');
                    }}
                  />
                ) : loading && !desktopAvailable ? <SkeletonPanel /> : tab === 'clean' ? (
                  <CleanRoute desktopAvailable={desktopAvailable} />
                ) : (
                  <SharePage
                    desktopAvailable={desktopAvailable}
                    selected={selectedEntries}
                    selectedCount={selectedPaths.length}
                    onClear={clearSelection}
                    shareLink={shareLink}
                    sharing={sharing}
                    shareError={shareError}
                    copied={copied}
                    pcShareSession={pcShareSession}
                    onPcShareSessionChange={setPcShareSession}
                    onStart={() => void handleStartShare()}
                    onStop={() => void handleStopShare()}
                    onCopy={() => void handleCopyLink()}
                    onBrowse={() => setTab('browse')}
                  />
                )}
              </motion.section>
            </AnimatePresence>
          </div>
          <footer className="statusbar"><span><span className={`status-indicator${desktopAvailable ? ' connected' : ''}`} />{desktopAvailable ? 'On this device' : 'Windows desktop connection needed'}</span><span>{tab === 'browse' && indexPath ? `${indexEntries.length.toLocaleString()} indexed items${indexProgress?.skipped ? ` · ${indexProgress.skipped.toLocaleString()} skipped` : ''}` : 'Private until you choose to share'}</span></footer>
        </main>
      </div>

      <nav className="mobile-nav" aria-label="Main navigation">
        <button type="button" className={tab === 'clean' ? 'active' : ''} onClick={() => setTab('clean')}><Sparkles size={19} /><span>Clean</span></button>
        <button type="button" className={tab === 'browse' ? 'active' : ''} onClick={() => setTab('browse')}><FolderOpen size={19} /><span>Browse</span></button>
        <button type="button" className={tab === 'share' ? 'active' : ''} onClick={() => setTab('share')}><Send size={19} /><span>Share</span></button>
      </nav>

      <OperationProgressCard operations={operations} onCancel={(jobId) => void cancelOperation(jobId)} />
    </div>
  );
}

interface SharePageProps {
  desktopAvailable: boolean;
  selected: FileEntry[];
  selectedCount: number;
  onClear: () => void;
  shareLink: ShareLink | null;
  sharing: boolean;
  shareError: string | null;
  copied: boolean;
  pcShareSession: PcShareSession | null;
  onPcShareSessionChange: (session: PcShareSession | null) => void;
  onStart: () => void;
  onStop: () => void;
  onCopy: () => void;
  onBrowse: () => void;
}

function SharePage({ desktopAvailable, selected, selectedCount, onClear, shareLink, sharing, shareError, copied, pcShareSession, onPcShareSessionChange, onStart, onStop, onCopy, onBrowse }: SharePageProps) {
  const selectedFile = selected.find((entry) => !entry.isDirectory && !entry.isCloudPlaceholder);
  return (
    <div className="share-page">
      <section className="share-hero">
        <div className="share-hero-content"><span className="hero-kicker share-kicker"><Send size={14} /> NEARBY, NOT EVERYWHERE</span><h2>Send a file.<br /><em>Keep it yours.</em></h2><p>Share directly over your local Wi-Fi. No account, no cloud upload, no copy left behind.</p><div className="share-trust"><span><LockKeyhole size={14} /> Private link</span><span><Cloud size={14} /> Local network only</span></div></div>
        <div className="share-art" aria-hidden="true"><div className="share-halo" /><div className="share-device device-back"><span className="device-camera" /><span /><span /><span /></div><div className="share-device device-front"><span className="device-camera" /><div className="share-check"><Check size={19} /></div><b>Sent!</b><i /></div><span className="share-art-dot dot-a" /><span className="share-art-dot dot-b" /><span className="share-art-star">✳</span></div>
      </section>

      <SharePcPanel desktopAvailable={desktopAvailable} selected={selected} session={pcShareSession} onSessionChange={onPcShareSessionChange} />

      {!shareLink ? (
        <section className="share-content-card">
          <div className="section-title-row"><div><span className="section-overline">READY WHEN YOU ARE</span><h3>Choose something to send</h3><p>Select a downloaded file in Browse to create a one-time local link.</p></div><button type="button" className="secondary-button" onClick={onBrowse}><FolderOpen size={15} /> Browse files</button></div>
          {!desktopAvailable ? (
            <div className="desktop-required-card share-desktop-note"><div className="desktop-illustration"><Send size={24} /></div><div><strong>Nearby sharing runs from the Windows app</strong><p>When Sift is open on your PC, you can create a private QR link for another device on the same Wi-Fi.</p></div></div>
          ) : selectedFile ? (
            <div className="share-selected-file"><FileGlyph entry={selectedFile} /><div className="share-file-copy"><strong>{selectedFile.name}</strong><span>{readableSize(selectedFile.size)} · Ready to share</span></div><button type="button" className="text-button" onClick={onClear}>Clear selection</button></div>
          ) : (
            <div className="share-empty-selection"><div className="select-file-icon"><FolderOpen size={19} /></div><div><strong>{selectedCount > 0 ? 'Choose a file, not a folder' : 'No file selected'}</strong><p>Go to Browse, select one downloaded file, then come back here.</p></div><button type="button" className="text-button" onClick={onBrowse}>Go to Browse <ChevronRight size={14} /></button></div>
          )}
          {shareError && <div className="inline-error"><XCircle size={15} />{shareError}</div>}
          {desktopAvailable && <div className="share-action-row"><span><ShieldCheck size={15} /> Link expires automatically after 10 minutes.</span><button type="button" className="primary-button" onClick={onStart} disabled={!selectedFile || sharing}>{sharing ? <span className="button-spinner" /> : <Send size={16} />}{sharing ? 'Preparing secure link…' : 'Create sharing link'}<ArrowUpRight size={15} /></button></div>}
        </section>
      ) : (
        <section className="share-active-card">
          <div className="share-active-heading"><div><span className="active-share-badge"><span /> SHARING NOW</span><h3>Ready for a nearby device</h3><p>Keep both devices on the same Wi-Fi. The link stops working in 10 minutes.</p></div><button className="icon-button" type="button" aria-label="Stop sharing" title="Stop sharing" onClick={onStop}><X size={17} /></button></div>
          <div className="active-share-grid">
            <div className="qr-panel"><div className="qr-frame" dangerouslySetInnerHTML={{ __html: shareLink.qrSvg }} /><span>Scan with your phone camera</span></div>
            <div className="share-link-details"><div className="share-file-chip"><FileGlyph entry={selectedFile ?? { name: shareLink.fileName, path: '', extension: '', kind: 'file', isDirectory: false, isCloudPlaceholder: false, size: 0, modifiedUnix: null }} /><div><strong>{shareLink.fileName}</strong><small>Only this file is shared</small></div></div><label className="link-label">PRIVATE LINK</label><div className="link-copy-row"><input readOnly value={shareLink.url} aria-label="Private sharing link" onFocus={(event) => event.currentTarget.select()} /><button type="button" onClick={onCopy} aria-label="Copy link" title="Copy link">{copied ? <Check size={16} /> : <Copy size={16} />}</button></div><span className="link-expiry"><span /> Available for 10 minutes · On your local network</span></div>
          </div>
          <div className="share-security-note"><LockKeyhole size={15} /><span>Anyone with this private link on your network can download the file. Sharing stops when you stop it, close Sift, or the timer ends.</span><button type="button" className="stop-share-button" onClick={onStop}>Stop sharing</button></div>
        </section>
      )}
      <div className="share-footer-note"><ShieldCheck size={15} /><span>Sift serves only the file you selected. The link is protected by a random private token and never exposes your folders.</span></div>
    </div>
  );
}

export default App;
