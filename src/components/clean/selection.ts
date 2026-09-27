import type { CleanCard, CleanGroup, DuplicateSet } from '../../lib/bindings';

/**
 * Selection math for the Clean tab's confirm sheet, kept apart from the markup so it can
 * be tested without a DOM.
 */

/** Everything except the copy Sift protects: the default duplicate selection. */
export function defaultDuplicateSelection(sets: DuplicateSet[]): string[] {
  return sets.flatMap((set) => set.files.filter((file) => !file.original).map((file) => file.path));
}

/** True when at least one protected copy survives in every set. */
export function keepsOneCopyPerSet(sets: DuplicateSet[], selected: string[]): boolean {
  const chosen = new Set(selected);
  return sets.every((set) => set.files.some((file) => !chosen.has(file.path)));
}

export function sumBytes(items: Array<{ path: string; size: number }>, paths: string[]): number {
  const chosen = new Set(paths);
  return items.reduce((total, item) => (chosen.has(item.path) ? total + item.size : total), 0);
}

export function groupBytes(groups: CleanGroup[], ids: string[]): number {
  const chosen = new Set(ids);
  return groups.reduce((total, group) => (chosen.has(group.id) ? total + group.reclaimableBytes : total), 0);
}

export function groupItems(groups: CleanGroup[], ids: string[]): number {
  const chosen = new Set(ids);
  return groups.reduce((total, group) => (chosen.has(group.id) ? total + group.itemCount : total), 0);
}

/** The junk card's Recycle Bin row is a pseudo-location with no path behind it. */
export function isRecycleBinGroup(group: CleanGroup): boolean {
  return group.id === 'recycle-bin';
}

export function cardIsActionable(card: CleanCard, hasDuplicates: boolean): boolean {
  if (card.id === 'duplicates') return hasDuplicates;
  return card.ready;
}
