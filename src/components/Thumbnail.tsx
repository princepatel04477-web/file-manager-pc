import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { commands } from '../lib/bindings';
import type { FileEntry } from '../lib/bindings';

const cache = new Map<string, string>();
const pending = new Map<string, Promise<string | null>>();
const MAX_CACHED = 400;

function remember(key: string, value: string): void {
  if (cache.size >= MAX_CACHED) {
    const oldest = cache.keys().next();
    if (!oldest.done && oldest.value !== undefined) cache.delete(oldest.value);
  }
  cache.set(key, value);
}

/** Ask the backend for a shell thumbnail. Cloud placeholders never reach the backend. */
function request(entry: FileEntry): Promise<string | null> {
  const key = `${entry.path}|${entry.modifiedUnix ?? 0}|${entry.size}`;
  const cached = cache.get(key);
  if (cached) return Promise.resolve(cached);
  const inflight = pending.get(key);
  if (inflight) return inflight;
  const task = commands
    .getThumbnail(entry.path)
    .then((thumbnail) => {
      remember(key, thumbnail.dataUrl);
      return thumbnail.dataUrl;
    })
    .catch(() => {
      // Not every file has an extractable image; the caller falls back to a glyph.
      return null;
    })
    .finally(() => pending.delete(key));
  pending.set(key, task);
  return task;
}

export function useThumbnail(entry: FileEntry, enabled: boolean): string | null {
  const key = `${entry.path}|${entry.modifiedUnix ?? 0}|${entry.size}`;
  const [source, setSource] = useState<string | null>(() => (enabled ? cache.get(key) ?? null : null));

  useEffect(() => {
    if (!enabled || !isTauri() || entry.isCloudPlaceholder || entry.isDirectory) {
      setSource(null);
      return;
    }
    const cached = cache.get(key);
    if (cached) {
      setSource(cached);
      return;
    }
    let active = true;
    void request(entry).then((value) => {
      if (active) setSource(value);
    });
    return () => {
      active = false;
    };
  }, [enabled, entry, key]);

  return source;
}
