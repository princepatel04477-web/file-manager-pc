import type { FileEntry } from './bindings';

export type PreviewKind = 'image' | 'video' | 'audio' | 'pdf' | 'text' | 'none';

const IMAGE = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'avif'];
/** Browsers cannot play every container; the rest falls back to the shell viewer. */
const VIDEO = ['mp4', 'webm', 'm4v', 'mov'];
const AUDIO = ['mp3', 'wav', 'ogg', 'm4a', 'aac', 'flac'];
const TEXT = [
  'txt', 'md', 'markdown', 'log', 'json', 'jsonc', 'csv', 'tsv', 'xml', 'yml', 'yaml', 'toml',
  'ini', 'cfg', 'conf', 'env', 'gitignore', 'rs', 'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'css',
  'scss', 'html', 'htm', 'svg', 'py', 'sh', 'ps1', 'sql', 'java',
];

export function extensionOf(name: string): string {
  const index = name.lastIndexOf('.');
  if (index <= 0) return '';
  return name.slice(index + 1).toLowerCase();
}

export function previewKindFor(entry: Pick<FileEntry, 'name' | 'isDirectory' | 'isCloudPlaceholder'>): PreviewKind {
  if (entry.isDirectory || entry.isCloudPlaceholder) return 'none';
  const extension = extensionOf(entry.name);
  if (IMAGE.includes(extension)) return 'image';
  if (VIDEO.includes(extension)) return 'video';
  if (AUDIO.includes(extension)) return 'audio';
  if (extension === 'pdf') return 'pdf';
  if (TEXT.includes(extension)) return 'text';
  return 'none';
}

/** Mirrors the Windows naming rules the Rust side enforces before touching disk. */
export function validateFileName(name: string): string | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return 'Enter a name.';
  if (trimmed !== name) return 'A name cannot start or end with a space.';
  if (trimmed === '.' || trimmed === '..') return 'That name is reserved.';
  if (/[<>:"/\\|?*]/.test(trimmed)) return 'A name cannot contain < > : " / \\ | ? or *';
  if (/[\u0000-\u001f\u007f]/.test(trimmed)) return 'A name cannot contain control characters.';
  if (/[. ]$/.test(trimmed)) return 'A name cannot end with a dot or a space.';
  const reserved = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
  if (reserved.test(trimmed.split('.')[0] ?? trimmed)) return 'That name is reserved by Windows.';
  if ([...trimmed].length > 255) return 'That name is too long.';
  return null;
}

/** Pre-selects the stem so "Rename" behaves like Explorer's F2. */
export function stemOf(name: string): number {
  const index = name.lastIndexOf('.');
  return index <= 0 ? name.length : index;
}
