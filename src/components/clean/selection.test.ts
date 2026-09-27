import { describe, expect, it } from 'vitest';
import type { CleanGroup, DuplicateSet } from '../../lib/bindings';
import {
  cardIsActionable,
  defaultDuplicateSelection,
  groupBytes,
  groupItems,
  isRecycleBinGroup,
  keepsOneCopyPerSet,
  sumBytes,
} from './selection';

function set(fingerprint: string, size: number, names: string[], originalIndex: number): DuplicateSet {
  return {
    fingerprint,
    size,
    reclaimableBytes: size * (names.length - 1),
    files: names.map((name, index) => ({
      path: `C:\\Users\\me\\${name}`,
      name,
      size,
      modifiedUnix: 1_700_000_000 + index,
      original: index === originalIndex,
    })),
  };
}

describe('duplicate selection defaults', () => {
  it('selects every copy except the protected original', () => {
    const sets = [set('a', 5_000, ['keep-me.mp4', 'copy-1.mp4', 'copy-2.mp4'], 0)];
    expect(defaultDuplicateSelection(sets)).toEqual([
      'C:\\Users\\me\\copy-1.mp4',
      'C:\\Users\\me\\copy-2.mp4',
    ]);
  });

  it('protects one copy in every set, not just the first', () => {
    const sets = [
      set('a', 5_000, ['a-keep.zip', 'a-copy.zip'], 0),
      set('b', 9_000, ['b-copy.zip', 'b-keep.zip'], 1),
    ];
    const selected = defaultDuplicateSelection(sets);
    expect(selected).toEqual(['C:\\Users\\me\\a-copy.zip', 'C:\\Users\\me\\b-copy.zip']);
    expect(keepsOneCopyPerSet(sets, selected)).toBe(true);
  });

  it('refuses a selection that would remove the last copy of a file', () => {
    const sets = [set('a', 5_000, ['only-two.mp4', 'only-two-copy.mp4'], 0)];
    const everything = sets[0]?.files.map((file) => file.path) ?? [];
    expect(keepsOneCopyPerSet(sets, everything)).toBe(false);
    expect(keepsOneCopyPerSet(sets, [everything[1] ?? ''])).toBe(true);
  });

  it('reclaims size times the copies beyond the first', () => {
    const sets = [set('a', 4_096, ['x.bin', 'y.bin', 'z.bin'], 0)];
    const selected = defaultDuplicateSelection(sets);
    expect(sumBytes(sets[0]?.files ?? [], selected)).toBe(8_192);
  });
});

describe('junk group totals', () => {
  const groups: CleanGroup[] = [
    { id: 'temp-user', label: 'Temporary files', itemCount: 120, reclaimableBytes: 1_000 },
    { id: 'chrome-default-cache', label: 'Chrome cache', itemCount: 40, reclaimableBytes: 4_000 },
    { id: 'recycle-bin', label: 'Recycle Bin', itemCount: 7, reclaimableBytes: 900 },
  ];

  it('sums only the selected groups', () => {
    expect(groupBytes(groups, ['temp-user', 'recycle-bin'])).toBe(1_900);
    expect(groupItems(groups, ['chrome-default-cache'])).toBe(40);
    expect(groupBytes(groups, [])).toBe(0);
  });

  it('treats the Recycle Bin as a pseudo-location', () => {
    expect(groups.filter(isRecycleBinGroup).map((group) => group.label)).toEqual(['Recycle Bin']);
  });
});

describe('card availability', () => {
  it('needs a duplicate scan before the duplicate card can be opened', () => {
    const card = { id: 'duplicates', ready: false } as never;
    expect(cardIsActionable(card, false)).toBe(false);
    expect(cardIsActionable(card, true)).toBe(true);
  });

  it('follows the ready flag everywhere else', () => {
    expect(cardIsActionable({ id: 'junk', ready: true } as never, false)).toBe(true);
    expect(cardIsActionable({ id: 'junk', ready: false } as never, false)).toBe(false);
  });
});
