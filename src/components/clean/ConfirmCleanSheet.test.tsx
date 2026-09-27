import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => false,
  convertFileSrc: (path: string) => `file://${path}`,
}));

import type { CleanCard, DuplicateSet } from '../../lib/bindings';
import { ConfirmCleanSheet, type ConfirmRequest } from './ConfirmCleanSheet';

const card: CleanCard = {
  id: 'duplicates',
  title: 'Duplicate files',
  description: 'Identical files found by hashing, one copy kept.',
  action: 'unavailable',
  itemCount: 3,
  reclaimableBytes: 10_000,
  groups: [],
  items: [],
  truncated: false,
  skipped: 0,
  ready: true,
};

const sets: DuplicateSet[] = [
  {
    fingerprint: 'abc123',
    size: 5_000,
    reclaimableBytes: 10_000,
    files: [
      { path: 'C:\\Users\\me\\Videos\\holiday.mp4', name: 'holiday.mp4', size: 5_000, modifiedUnix: 1_700_000_000, original: true },
      { path: 'C:\\Users\\me\\Desktop\\holiday.mp4', name: 'holiday.mp4', size: 5_000, modifiedUnix: 1_700_000_900, original: false },
      { path: 'C:\\Users\\me\\Downloads\\holiday-copy.mp4', name: 'holiday-copy.mp4', size: 5_000, modifiedUnix: 1_700_001_800, original: false },
    ],
  },
];

const request: ConfirmRequest = { kind: 'duplicates', card, sets };

function renderSheet() {
  return render(
    <ConfirmCleanSheet
      request={request}
      busy={false}
      desktopAvailable
      onClose={() => undefined}
      onConfirmPaths={() => undefined}
      onConfirmJunk={() => undefined}
      onUninstall={() => undefined}
    />,
  );
}

function checkbox(name: RegExp): HTMLInputElement {
  return screen.getByRole('checkbox', { name }) as HTMLInputElement;
}

afterEach(cleanup);

describe('duplicate review sheet', () => {
  it('pre-selects every copy except the protected original', () => {
    renderSheet();
    expect(checkbox(/Videos/).checked).toBe(false);
    expect(checkbox(/Desktop/).checked).toBe(true);
    expect(checkbox(/holiday-copy/).checked).toBe(true);
  });

  it('marks the protected copy and totals the reclaimable space', () => {
    renderSheet();
    expect(screen.getByText('Kept')).toBeTruthy();
    const total = screen.getByText(/free up/i);
    expect(total.textContent).toMatch(/2 items/);
    expect(total.textContent).toMatch(/free up \d+(\.\d+)? [KMGT]?B/);
  });

  it('stops the user removing the last copy of a file', () => {
    renderSheet();
    fireEvent.click(checkbox(/Videos/));
    expect(screen.getByRole('alert').textContent).toMatch(/leave one unchecked/i);
    const confirm = screen.getByRole('button', { name: /move 3 to recycle bin/i }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
  });

  it('hands back exactly the paths that stayed selected', () => {
    const onConfirmPaths = vi.fn();
    render(
      <ConfirmCleanSheet
        request={request}
        busy={false}
        desktopAvailable
        onClose={() => undefined}
        onConfirmPaths={onConfirmPaths}
        onConfirmJunk={() => undefined}
        onUninstall={() => undefined}
      />,
    );
    fireEvent.click(checkbox(/holiday-copy/));
    fireEvent.click(screen.getByRole('button', { name: /move 1 to recycle bin/i }));
    expect(onConfirmPaths).toHaveBeenCalledWith(['C:\\Users\\me\\Desktop\\holiday.mp4'], 'Duplicate files');
  });
});

describe('junk confirm sheet', () => {
  const junkCard: CleanCard = {
    ...card,
    id: 'junk',
    title: 'Junk files',
    action: 'deleteJunk',
    groups: [
      { id: 'temp-user', label: 'Temporary files', itemCount: 30, reclaimableBytes: 2_048 },
      { id: 'recycle-bin', label: 'Recycle Bin', itemCount: 4, reclaimableBytes: 1_024 },
    ],
  };

  it('starts with every location selected and reports the group total', () => {
    render(
      <ConfirmCleanSheet
        request={{ kind: 'junk', card: junkCard }}
        busy={false}
        desktopAvailable
        onClose={() => undefined}
        onConfirmPaths={() => undefined}
        onConfirmJunk={() => undefined}
        onUninstall={() => undefined}
      />,
    );
    expect(checkbox(/temporary files/i).checked).toBe(true);
    expect(screen.getByText(/34 items/)).toBeTruthy();
  });

  it('sends only the locations that stayed checked', () => {
    const onConfirmJunk = vi.fn();
    render(
      <ConfirmCleanSheet
        request={{ kind: 'junk', card: junkCard }}
        busy={false}
        desktopAvailable
        onClose={() => undefined}
        onConfirmPaths={() => undefined}
        onConfirmJunk={onConfirmJunk}
        onUninstall={() => undefined}
      />,
    );
    fireEvent.click(checkbox(/recycle bin/i));
    fireEvent.click(screen.getByRole('button', { name: /move 30 to recycle bin/i }));
    expect(onConfirmJunk).toHaveBeenCalledWith(['temp-user'], 2_048);
  });

  it('labels the action as emptying when only the Recycle Bin is selected', () => {
    render(
      <ConfirmCleanSheet
        request={{ kind: 'junk', card: junkCard }}
        busy={false}
        desktopAvailable
        onClose={() => undefined}
        onConfirmPaths={() => undefined}
        onConfirmJunk={() => undefined}
        onUninstall={() => undefined}
      />,
    );
    fireEvent.click(checkbox(/temporary files/i));
    expect(screen.getByRole('button', { name: /empty recycle bin/i })).toBeTruthy();
  });
});
