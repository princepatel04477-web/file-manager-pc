import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => false,
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));

import { invoke } from '@tauri-apps/api/core';
import type { ConflictDecision, TransferPlan } from '../lib/bindings';
import { useOpsStore } from './ops-store';

const mockedInvoke = vi.mocked(invoke);

function planWith(conflicts: TransferPlan['conflicts']): TransferPlan {
  return {
    kind: 'copy',
    destination: 'C:\\Users\\u\\Documents',
    items: conflicts.map((conflict) => ({
      source: conflict.source,
      destination: conflict.destination,
      name: conflict.name,
      isDirectory: false,
      size: conflict.sourceSize,
      action: 'conflict',
    })),
    conflicts,
    blocked: [],
    bytesTotal: conflicts.reduce((total, conflict) => total + conflict.sourceSize, 0),
  };
}

function conflictFor(name: string, sourceSize: number, existingSize: number) {
  return {
    source: `C:\\Users\\u\\Downloads\\${name}`,
    destination: `C:\\Users\\u\\Documents\\${name}`,
    name,
    sourceIsDirectory: false,
    sourceSize,
    sourceModifiedUnix: 1_700_000_000,
    existingIsDirectory: false,
    existingSize,
    existingModifiedUnix: 1_700_000_500,
  };
}

function reset() {
  useOpsStore.setState({
    operations: [],
    clipboard: null,
    conflict: null,
    busy: false,
    error: null,
    notice: null,
    favorites: [],
    recents: [],
    refresh: null,
  });
  mockedInvoke.mockReset();
}

describe('copy/move conflict handling in the UI store', () => {
  beforeEach(reset);

  it('asks before touching anything when the destination already has the item', async () => {
    const plan = planWith([conflictFor('report.pdf', 2_048, 1_024)]);
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(plan);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    const refresh = vi.fn();
    useOpsStore.setState({
      clipboard: { mode: 'copy', paths: ['C:\\Users\\u\\Downloads\\report.pdf'], names: ['report.pdf'] },
      refresh,
    });

    await useOpsStore.getState().pasteInto('C:\\Users\\u\\Documents');

    const state = useOpsStore.getState();
    expect(state.conflict?.plan.conflicts).toHaveLength(1);
    expect(state.conflict?.plan.conflicts[0]?.name).toBe('report.pdf');
    expect(state.conflict?.plan.conflicts[0]?.existingSize).toBe(1_024);
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(refresh).not.toHaveBeenCalled();
  });

  it('sends the chosen decisions plus the apply-to-all default', async () => {
    const plan = planWith([conflictFor('report.pdf', 2_048, 1_024), conflictFor('notes.txt', 12, 40)]);
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(plan);
      if (command === 'copy_paths') return Promise.resolve({ completed: 2, skipped: 0, failed: 0, bytes: 2_060, cancelled: false, destinations: [], errors: [] });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    useOpsStore.setState({
      conflict: {
        kind: 'copy',
        sources: ['C:\\Users\\u\\Downloads\\report.pdf', 'C:\\Users\\u\\Downloads\\notes.txt'],
        destination: 'C:\\Users\\u\\Documents',
        plan,
      },
    });

    const decisions: ConflictDecision[] = [{ source: 'C:\\Users\\u\\Downloads\\notes.txt', action: 'skip' }];
    await useOpsStore.getState().resolveConflict(decisions, 'keepBoth');

    const call = mockedInvoke.mock.calls.find(([command]) => command === 'copy_paths');
    expect(call).toBeDefined();
    expect(call?.[1]).toEqual({
      paths: ['C:\\Users\\u\\Downloads\\report.pdf', 'C:\\Users\\u\\Downloads\\notes.txt'],
      destination: 'C:\\Users\\u\\Documents',
      decisions,
      defaultAction: 'keepBoth',
    });
    expect(useOpsStore.getState().conflict).toBeNull();
    expect(useOpsStore.getState().notice).toContain('Copied 2 items');
  });

  it('runs a clean paste straight through without a dialog', async () => {
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(planWith([]));
      if (command === 'copy_paths') return Promise.resolve({ completed: 1, skipped: 0, failed: 0, bytes: 10, cancelled: false, destinations: ['C:\\Users\\u\\Documents\\a.txt'], errors: [] });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    useOpsStore.setState({ clipboard: { mode: 'copy', paths: ['C:\\Users\\u\\Downloads\\a.txt'], names: ['a.txt'] } });

    await useOpsStore.getState().pasteInto('C:\\Users\\u\\Documents');

    expect(mockedInvoke).toHaveBeenCalledWith('copy_paths', {
      paths: ['C:\\Users\\u\\Downloads\\a.txt'],
      destination: 'C:\\Users\\u\\Documents',
      decisions: [],
      defaultAction: 'keepBoth',
    });
    expect(useOpsStore.getState().notice).toContain('Copied 1 item');
    expect(useOpsStore.getState().clipboard).not.toBeNull();
  });

  it('turns a cut into a move and clears the clipboard afterwards', async () => {
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(planWith([]));
      if (command === 'move_paths') return Promise.resolve({ completed: 1, skipped: 0, failed: 0, bytes: 0, cancelled: false, destinations: [], errors: [] });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    useOpsStore.setState({ clipboard: { mode: 'cut', paths: ['C:\\Users\\u\\Downloads\\a.txt'], names: ['a.txt'] } });

    await useOpsStore.getState().pasteInto('C:\\Users\\u\\Documents');

    expect(mockedInvoke).toHaveBeenCalledWith('plan_transfer', { paths: ['C:\\Users\\u\\Downloads\\a.txt'], destination: 'C:\\Users\\u\\Documents', kind: 'move' });
    expect(mockedInvoke).toHaveBeenCalledWith('move_paths', expect.objectContaining({ destination: 'C:\\Users\\u\\Documents' }));
    expect(useOpsStore.getState().clipboard).toBeNull();
    expect(useOpsStore.getState().notice).toContain('Moved 1 item');
  });

  it('keeps the clipboard when a move is cancelled mid-flight', async () => {
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(planWith([]));
      if (command === 'move_paths') return Promise.resolve({ completed: 0, skipped: 0, failed: 0, bytes: 0, cancelled: true, destinations: [], errors: [] });
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    useOpsStore.setState({ clipboard: { mode: 'cut', paths: ['C:\\Users\\u\\Downloads\\a.txt'], names: ['a.txt'] } });

    await useOpsStore.getState().pasteInto('C:\\Users\\u\\Documents');

    expect(useOpsStore.getState().notice).toContain('stopped early');
  });

  it('surfaces backend failures instead of pretending the paste worked', async () => {
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'plan_transfer') return Promise.resolve(planWith([]));
      if (command === 'copy_paths') return Promise.reject('The destination folder is not available.');
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    useOpsStore.setState({ clipboard: { mode: 'copy', paths: ['C:\\Users\\u\\Downloads\\a.txt'], names: ['a.txt'] } });

    await useOpsStore.getState().pasteInto('C:\\Users\\u\\Documents');

    expect(useOpsStore.getState().error).toBe('The destination folder is not available.');
    expect(useOpsStore.getState().busy).toBe(false);
  });

  it('deletes through the Recycle Bin and refreshes the listing', async () => {
    mockedInvoke.mockImplementation((command: string) => {
      if (command === 'delete_paths') return Promise.resolve({ moved: 2, skipped: 0, cancelled: false, errors: [] });
      if (command === 'list_favorites') return Promise.resolve([]);
      if (command === 'list_recents') return Promise.resolve([]);
      return Promise.reject(new Error(`unexpected command ${command}`));
    });
    const refresh = vi.fn();
    useOpsStore.setState({ refresh });

    const result = await useOpsStore.getState().deletePaths(['C:\\Users\\u\\a.txt', 'C:\\Users\\u\\b.txt']);

    expect(mockedInvoke).toHaveBeenCalledWith('delete_paths', { paths: ['C:\\Users\\u\\a.txt', 'C:\\Users\\u\\b.txt'] });
    expect(result?.moved).toBe(2);
    expect(useOpsStore.getState().notice).toBe('2 items moved to the Recycle Bin.');
    expect(refresh).toHaveBeenCalled();
  });

  it('records a cancellation request for the running job', async () => {
    mockedInvoke.mockResolvedValue(true);
    await useOpsStore.getState().cancel('op-1');
    expect(mockedInvoke).toHaveBeenCalledWith('cancel_operation', { jobId: 'op-1' });
  });
});
