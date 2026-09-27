import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { TransferPlan } from '../lib/bindings';
import type { ConflictRequest } from '../stores/ops-store';
import { ConflictDialog } from './ConflictDialog';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => false,
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));

const conflicts: TransferPlan['conflicts'] = [
  {
    source: 'C:\\Users\\u\\Downloads\\report.pdf',
    destination: 'C:\\Users\\u\\Documents\\report.pdf',
    name: 'report.pdf',
    sourceIsDirectory: false,
    sourceSize: 2_048,
    sourceModifiedUnix: 1_700_000_000,
    existingIsDirectory: false,
    existingSize: 1_024,
    existingModifiedUnix: 1_700_000_500,
  },
  {
    source: 'C:\\Users\\u\\Downloads\\notes.txt',
    destination: 'C:\\Users\\u\\Documents\\notes.txt',
    name: 'notes.txt',
    sourceIsDirectory: false,
    sourceSize: 12,
    sourceModifiedUnix: 1_700_000_100,
    existingIsDirectory: false,
    existingSize: 40,
    existingModifiedUnix: 1_700_000_900,
  },
];

function request(): ConflictRequest {
  return {
    kind: 'copy',
    sources: conflicts.map((conflict) => conflict.source),
    destination: 'C:\\Users\\u\\Documents',
    plan: {
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
      blocked: [{ source: 'C:\\Users\\u\\Downloads\\online.docx', reason: 'cloudOnly' }],
      bytesTotal: 2_060,
    },
  };
}

describe('ConflictDialog', () => {
  afterEach(cleanup);

  it('shows both sides of every collision', () => {
    render(<ConflictDialog request={request()} busy={false} onResolve={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByText('Copy 2 conflicting items')).toBeTruthy();
    expect(screen.getByText('report.pdf')).toBeTruthy();
    expect(screen.getByText(/Incoming: 2\.0 KB ·/)).toBeTruthy();
    expect(screen.getByText(/Existing: 1\.0 KB ·/)).toBeTruthy();
    expect(screen.getByText(/1 item will not be copied/)).toBeTruthy();
  });

  it('defaults to keep both and only sends explicit per-item overrides', () => {
    const onResolve = vi.fn();
    render(<ConflictDialog request={request()} busy={false} onResolve={onResolve} onCancel={vi.fn()} />);

    fireEvent.click(screen.getByRole('radio', { name: /Replace/i }));
    const selects = screen.getAllByRole('combobox');
    fireEvent.change(selects[1] as HTMLSelectElement, { target: { value: 'skip' } });
    fireEvent.click(screen.getByRole('button', { name: /Copy 2 items/i }));

    expect(onResolve).toHaveBeenCalledWith([{ source: 'C:\\Users\\u\\Downloads\\notes.txt', action: 'skip' }], 'replace');
  });

  it('sends no overrides when every item follows the default', () => {
    const onResolve = vi.fn();
    render(<ConflictDialog request={request()} busy={false} onResolve={onResolve} onCancel={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /Copy 2 items/i }));
    expect(onResolve).toHaveBeenCalledWith([], 'keepBoth');
  });

  it('can be dismissed without touching the filesystem', () => {
    const onCancel = vi.fn();
    const onResolve = vi.fn();
    render(<ConflictDialog request={request()} busy={false} onResolve={onResolve} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onCancel).toHaveBeenCalled();
    expect(onResolve).not.toHaveBeenCalled();
  });

  it('labels move operations as moves', () => {
    const moveRequest = request();
    moveRequest.kind = 'move';
    render(<ConflictDialog request={moveRequest} busy={false} onResolve={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByText('Move 2 conflicting items')).toBeTruthy();
    expect(screen.getByRole('button', { name: /Move 2 items/i })).toBeTruthy();
  });
});
