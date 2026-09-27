import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  isTauri: () => false,
  convertFileSrc: (path: string) => `file://${path}`,
}));

import type { CleanCard, CleanSummary, DuplicateReport } from '../../lib/bindings';
import { CleanCards } from './CleanCards';
import type { ConfirmRequest } from './ConfirmCleanSheet';

function card(overrides: Partial<CleanCard> & Pick<CleanCard, 'id'>): CleanCard {
  return {
    title: overrides.id,
    description: `${overrides.id} description`,
    action: 'deletePaths',
    itemCount: 0,
    reclaimableBytes: 0,
    groups: [],
    items: [],
    truncated: false,
    skipped: 0,
    ready: false,
    ...overrides,
  };
}

const summary: CleanSummary = {
  cards: [
    card({ id: 'junk', title: 'Junk files', action: 'deleteJunk', itemCount: 900, reclaimableBytes: 2_147_483_648, ready: true }),
    card({ id: 'duplicates', title: 'Duplicate files', action: 'unavailable' }),
    card({ id: 'large-files', title: 'Large files', itemCount: 3, reclaimableBytes: 512 * 1024 * 1024, ready: true }),
    card({ id: 'old-downloads', title: 'Old downloads' }),
    card({ id: 'old-screenshots', title: 'Old screenshots' }),
    card({ id: 'unused-apps', title: 'Unused apps', action: 'uninstall', itemCount: 12, ready: true }),
  ],
  recycleBin: { bytes: 0, items: 0, available: true },
  apps: [],
  appsBytes: 0,
  indexedFiles: 12_345,
  generatedAtUnix: 1_800_000_000,
};

const duplicates: DuplicateReport = {
  sets: [
    {
      fingerprint: 'abc',
      size: 5_000,
      reclaimableBytes: 5_000,
      files: [
        { path: 'C:\\a.bin', name: 'a.bin', size: 5_000, modifiedUnix: 1, original: true },
        { path: 'C:\\b.bin', name: 'b.bin', size: 5_000, modifiedUnix: 2, original: false },
      ],
    },
  ],
  reclaimableBytes: 5_000,
  candidates: 40,
  hashed: 40,
  skipped: 0,
};

function renderCards(overrides: { duplicates?: DuplicateReport | null } = {}) {
  const onReview = vi.fn();
  const onScanDuplicates = vi.fn();
  render(
    <CleanCards
      summary={summary}
      duplicates={overrides.duplicates ?? null}
      scanning={false}
      desktopAvailable
      onReview={onReview}
      onScanDuplicates={onScanDuplicates}
    />,
  );
  return { onReview, onScanDuplicates };
}

afterEach(cleanup);

describe('Clean card grid', () => {
  it('renders all six cards in the order the backend sends them', () => {
    renderCards();
    const titles = screen.getAllByRole('heading', { level: 3 }).map((node) => node.textContent);
    expect(titles).toEqual([
      'Junk files',
      'Duplicate files',
      'Large files',
      'Old downloads',
      'Old screenshots',
      'Unused apps',
    ]);
  });

  it('states the space each card can give back', () => {
    renderCards();
    expect(screen.getByText('Free up 2.0 GB')).toBeTruthy();
    expect(screen.getByText('900 items')).toBeTruthy();
    // readableSize drops the decimal from 100 and up, so 512 MB has no ".0"
    expect(screen.getByText('Free up 512 MB')).toBeTruthy();
    // old downloads and old screenshots are both empty in this fixture
    expect(screen.getAllByText('Nothing to clear')).toHaveLength(2);
    // the apps card lists programs but the registry gave no sizes for them
    expect(screen.getByText('12 to review')).toBeTruthy();
    expect(screen.getByText('Scan to find out')).toBeTruthy();
  });

  it('offers a duplicate scan until one has run', () => {
    const { onScanDuplicates } = renderCards();
    const scan = screen.getByRole('button', { name: /scan for duplicates/i });
    fireEvent.click(scan);
    expect(onScanDuplicates).toHaveBeenCalledTimes(1);
    expect(screen.getByText('Not scanned yet')).toBeTruthy();
  });

  it('opens the duplicate review once sets are known', () => {
    const { onReview } = renderCards({ duplicates });
    expect(screen.getByText('1 duplicate set')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /review copies/i }));
    const request = onReview.mock.calls[0]?.[0] as ConfirmRequest;
    expect(request.kind).toBe('duplicates');
  });

  it('routes each card to the right confirm sheet', () => {
    const { onReview } = renderCards({ duplicates });
    fireEvent.click(screen.getByRole('button', { name: /review junk/i }));
    fireEvent.click(screen.getByRole('button', { name: /manage apps/i }));
    const kinds = onReview.mock.calls.map((call) => (call[0] as ConfirmRequest).kind);
    expect(kinds).toEqual(['junk', 'apps']);
  });

  it('disables cards with nothing to remove', () => {
    renderCards();
    const buttons = screen.getAllByRole('button', { name: /choose files/i }) as HTMLButtonElement[];
    expect(buttons).toHaveLength(3);
    expect(buttons[0]?.disabled).toBe(false); // large files has something to show
    expect(buttons[1]?.disabled).toBe(true); // old downloads does not
  });
});
