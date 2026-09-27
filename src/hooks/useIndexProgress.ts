import { useEffect } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { onIndexProgress } from '../lib/ipc';
import { useIndexStore } from '../stores/index-store';

export function useIndexProgress(): void {
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void onIndexProgress((progress) => useIndexStore.getState().acceptProgress(progress))
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch((error: unknown) => {
        const message = error instanceof Error ? error.message : 'Index progress could not be observed.';
        useIndexStore.setState({ error: message });
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}
