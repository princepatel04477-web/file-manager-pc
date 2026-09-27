import { useEffect } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { onOperationProgress } from '../lib/ipc';
import { useOpsStore } from '../stores/ops-store';

/** Keeps the floating progress card in sync with copy/move/delete jobs. */
export function useOpsProgress(): void {
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void useOpsStore.getState().refreshOperations();
    void onOperationProgress((progress) => useOpsStore.getState().acceptProgress(progress))
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => {
        // Without events the card simply stays empty; operations still run.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
}
