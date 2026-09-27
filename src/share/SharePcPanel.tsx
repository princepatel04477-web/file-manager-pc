import { useEffect, useState } from 'react';
import { Check, Copy, Laptop, LoaderCircle, Radio, RefreshCw, Send, ShieldCheck, Square, Wifi } from 'lucide-react';
import type { FileEntry, NearbyShare, PcShareSession } from '../lib/bindings';
import { commands } from '../lib/bindings';
import { readableSize } from '../components/FileList';

interface SharePcPanelProps {
  desktopAvailable: boolean;
  selected: FileEntry[];
  session: PcShareSession | null;
  onSessionChange: (session: PcShareSession | null) => void;
}

export function SharePcPanel({ desktopAvailable, selected, session, onSessionChange }: SharePcPanelProps) {
  const [devices, setDevices] = useState<NearbyShare[]>([]);
  const [scanning, setScanning] = useState(false);
  const [code, setCode] = useState('');
  const [activeDevice, setActiveDevice] = useState<NearbyShare | null>(null);
  const [transferProgress, setTransferProgress] = useState<number | null>(null);
  const [sourceBytes, setSourceBytes] = useState(0);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const selectedFile = selected.find((entry) => !entry.isDirectory && !entry.isCloudPlaceholder);

  useEffect(() => {
    if (!session) return;
    let mounted = true;
    const update = async () => {
      try {
        const bytes = await commands.getPcShareProgress();
        if (mounted && bytes !== null) setSourceBytes(bytes);
        else if (mounted && session) { onSessionChange(null); setMessage('Nearby sharing expired after 10 minutes.'); }
      } catch { /* the session may have ended while the status request was in flight */ }
    };
    void update();
    const timer = window.setInterval(() => void update(), 800);
    return () => { mounted = false; window.clearInterval(timer); };
  }, [session, onSessionChange]);

  async function startSharing() {
    if (!selectedFile) return;
    setBusy(true); setError(null); setMessage(null);
    try {
      onSessionChange(await commands.startPcShare(selectedFile.path));
      setSourceBytes(0);
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally { setBusy(false); }
  }

  async function stopSharing() {
    try { await commands.stopPcShare(); onSessionChange(null); setSourceBytes(0); }
    catch (failure) { setError(failure instanceof Error ? failure.message : 'Could not stop nearby sharing.'); }
  }

  async function scan() {
    setScanning(true); setError(null); setMessage(null);
    try {
      const found = await commands.discoverPcShares();
      setDevices(found);
      if (!found.length) setMessage('No Sift PCs are advertising a file right now. Check both PCs are on the same private network.');
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { setScanning(false); }
  }

  async function receive(device: NearbyShare) {
    if (!/^\d{6}$/.test(code)) { setError('Enter the six-digit pairing code shown on the sending PC.'); return; }
    setBusy(true); setError(null); setMessage(null); setTransferProgress(0); setActiveDevice(device);
    try {
      const address = device.host.includes(':') ? `[${device.host}]` : device.host;
      const url = `http://${address}:${device.port}/${encodeURIComponent(device.token)}/download?code=${encodeURIComponent(code)}`;
      const chunks: Uint8Array[] = [];
      let received = 0;
      let total = device.fileSize;
      let attempt = 0;
      let requested = false;
      while ((!requested || received < total) && attempt < 4) {
        let response: Response;
        try {
          response = await fetch(url, { headers: received ? { Range: `bytes=${received}-` } : undefined });
        } catch (failure) {
          attempt += 1;
          if (attempt >= 4) throw new Error('Could not reach that PC. Windows Firewall or the network may be blocking the sharing port. Allow Sift on Private networks only and confirm both PCs are on the same local Wi-Fi.');
          await new Promise((resolve) => window.setTimeout(resolve, 500 * attempt));
          continue;
        }
        requested = true;
        if (!response.ok || (received > 0 && response.status !== 206)) {
          throw new Error(response.status === 404 ? 'Pairing code was not accepted, or the sender stopped sharing.' : `The sender returned HTTP ${response.status}.`);
        }
        const contentRange = response.headers.get('Content-Range');
        const matchedTotal = contentRange?.match(/\/(\d+)$/)?.[1];
        if (matchedTotal) total = Number(matchedTotal);
        if (!total) total = Number(response.headers.get('Content-Length')) || 0;
        const reader = response.body?.getReader();
        if (!reader) throw new Error('This browser could not read the incoming file stream.');
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            if (!value?.length) continue;
            chunks.push(value);
            received += value.length;
            setTransferProgress(total ? Math.min(100, Math.round((received / total) * 100)) : null);
          }
        } catch {
          attempt += 1;
          if (attempt >= 4) throw new Error('The connection dropped repeatedly. Your partial bytes are retained, but the peer is unreachable; check the Wi-Fi and Windows Firewall’s Private network permission, then try again.');
          await new Promise((resolve) => window.setTimeout(resolve, 500 * attempt));
        } finally { reader.releaseLock(); }
      }
      if (!requested) throw new Error('The sender could not be reached after several attempts.');
      if (received !== total) throw new Error('The transfer ended before the complete file arrived.');
      const blob = new Blob(chunks, { type: 'application/octet-stream' });
      const downloadUrl = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = downloadUrl; anchor.download = device.fileName; anchor.click();
      window.setTimeout(() => URL.revokeObjectURL(downloadUrl), 60_000);
      setTransferProgress(100);
      setMessage(`${device.fileName} is ready in your browser downloads.`);
      setCode('');
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : 'The transfer could not be completed.');
    } finally { setBusy(false); setActiveDevice(null); }
  }

  async function copyCode() {
    if (!session) return;
    try { await navigator.clipboard.writeText(session.pairingCode); setCopied(true); window.setTimeout(() => setCopied(false), 1500); }
    catch { setError('Could not copy the code. You can select it and copy manually.'); }
  }

  return (
    <section className="pc-share-panel" aria-label="Share between PCs">
      <div className="pc-share-heading">
        <span className="pc-share-icon"><Laptop size={17} /></span>
        <div><strong>PC to PC</strong><p>Discover nearby Sift PCs and send directly over your local network.</p></div>
      </div>
      {!desktopAvailable && <div className="pc-share-notice">PC sharing is available in the Windows app. This browser preview cannot advertise or discover devices.</div>}
      {desktopAvailable && <>
        {!session ? <div className="pc-share-start-row">
          <span>{selectedFile ? `Ready to send ${selectedFile.name} · ${readableSize(selectedFile.size)}` : 'Select one downloaded file in Browse to send it.'}</span>
          <button type="button" className="primary-button" disabled={!selectedFile || busy} onClick={() => void startSharing()}>{busy ? <LoaderCircle className="pc-spin" size={15} /> : <Radio size={15} />} Advertise this file</button>
        </div> : <div className="pc-session-card">
          <div className="pc-session-copy"><span className="pc-live-dot" /> Sharing <b>{session.fileName}</b> from <b>{session.deviceName}</b><small>Available for 10 minutes · {readableSize(session.fileSize)}</small></div>
          <div className="pc-pairing"><span>PAIRING CODE</span><strong aria-label="Six digit pairing code">{session.pairingCode}</strong><button type="button" onClick={() => void copyCode()} aria-label="Copy pairing code">{copied ? <Check size={14} /> : <Copy size={14} />}</button></div>
          <div className="pc-source-progress"><span style={{ width: `${session.fileSize ? Math.min(100, sourceBytes / session.fileSize * 100) : 0}%` }} /><small>{sourceBytes ? `${readableSize(Math.min(sourceBytes, session.fileSize))} sent` : 'Waiting for a receiver to connect'}</small></div>
          <button className="pc-stop-button" type="button" onClick={() => void stopSharing()}><Square size={13} /> Stop</button>
        </div>}
        <div className="pc-discover-row"><div><strong>Receive from another PC</strong><p>The receiver enters the sender’s displayed code to confirm the transfer.</p></div><button className="secondary-button" type="button" onClick={() => void scan()} disabled={scanning || busy}>{scanning ? <LoaderCircle className="pc-spin" size={14} /> : <RefreshCw size={14} />}{scanning ? 'Scanning…' : 'Find nearby PCs'}</button></div>
        {devices.length > 0 && <div className="pc-device-list">{devices.map((device) => <div className="pc-device-row" key={device.id}>
          <div className="pc-device-info"><span className="pc-device-icon"><Wifi size={15} /></span><span><strong>{device.deviceName}</strong><small>{device.fileName} · {readableSize(device.fileSize)}</small></span></div>
          <input aria-label={`Pairing code for ${device.deviceName}`} inputMode="numeric" autoComplete="one-time-code" maxLength={6} placeholder="6-digit code" value={activeDevice?.id === device.id ? code : code} onChange={(event) => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))} />
          <button type="button" className="primary-button" disabled={busy || code.length !== 6} onClick={() => void receive(device)}>{busy && activeDevice?.id === device.id ? <LoaderCircle className="pc-spin" size={14} /> : <Send size={14} />}{busy && activeDevice?.id === device.id ? 'Receiving…' : 'Confirm & receive'}</button>
        </div>)}</div>}
        {transferProgress !== null && activeDevice && <div className="pc-transfer-progress"><span style={{ width: `${transferProgress}%` }} /><small>{transferProgress}% received — {activeDevice.fileName}</small></div>}
        <div className="pc-privacy-note"><ShieldCheck size={14} /><span>Pairing code required. Transfers stay on your local network; no cloud or account. If Windows Firewall asks, allow Sift on Private networks only (leave Public networks unchecked). Interrupted transfers resume from the last received byte while this panel stays open.</span></div>
      </>}
      {error && <p className="pc-feedback pc-error" role="alert">{error}</p>}
      {message && <p className="pc-feedback" role="status">{message}</p>}
    </section>
  );
}
