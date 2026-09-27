import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { commands, type IndexProgress } from './bindings';

export { commands };
export type { IndexProgress };

/** Typed subscription to the backend's background indexing progress stream. */
export function onIndexProgress(handler: (progress: IndexProgress) => void): Promise<UnlistenFn> {
  return listen<IndexProgress>('index://progress', (event) => handler(event.payload));
}
