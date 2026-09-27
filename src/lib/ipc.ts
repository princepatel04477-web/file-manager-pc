import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { commands, type IndexProgress, type OperationProgress } from './bindings';

export { commands };
export type { IndexProgress, OperationProgress };

/** Typed subscription to the backend's background indexing progress stream. */
export function onIndexProgress(handler: (progress: IndexProgress) => void): Promise<UnlistenFn> {
  return listen<IndexProgress>('index://progress', (event) => handler(event.payload));
}

/** Typed subscription to copy/move/delete progress and cancellation updates. */
export function onOperationProgress(handler: (progress: OperationProgress) => void): Promise<UnlistenFn> {
  return listen<OperationProgress>('ops://progress', (event) => handler(event.payload));
}
