import { describe, expect, it } from 'vitest';
import { extensionOf, previewKindFor, stemOf, validateFileName } from './preview';

function entry(name: string, extra: Partial<{ isDirectory: boolean; isCloudPlaceholder: boolean }> = {}) {
  return { name, path: `C:\\Users\\u\\${name}`, isDirectory: false, isCloudPlaceholder: false, ...extra };
}

describe('validateFileName mirrors the Windows rules enforced in Rust', () => {
  it('rejects reserved, illegal, and empty names', () => {
    for (const invalid of ['', '   ', '.', '..', 'con', 'CON.txt', 'lpt9', 'NUL', 'com1.log', 'bad<>name', 'a|b', 'q?', 'tail.', 'trail ']) {
      expect(validateFileName(invalid), `${JSON.stringify(invalid)} should be rejected`).not.toBeNull();
    }
  });

  it('accepts ordinary names, including the ones Sift generates itself', () => {
    for (const valid of ['report.pdf', 'report (1).pdf', '.env', 'my file.txt', 'archive.tar (2).gz', 'consoles.txt']) {
      expect(validateFileName(valid), `${valid} should be accepted`).toBeNull();
    }
  });

  it('rejects names over the 255 character component limit', () => {
    expect(validateFileName('a'.repeat(255))).toBeNull();
    expect(validateFileName('a'.repeat(256))).not.toBeNull();
  });
});

describe('previewKindFor routes each file to the right viewer', () => {
  it('maps extensions to preview kinds', () => {
    expect(previewKindFor(entry('photo.JPG'))).toBe('image');
    expect(previewKindFor(entry('clip.mp4'))).toBe('video');
    expect(previewKindFor(entry('song.flac'))).toBe('audio');
    expect(previewKindFor(entry('paper.pdf'))).toBe('pdf');
    expect(previewKindFor(entry('main.rs'))).toBe('text');
    expect(previewKindFor(entry('notes.md'))).toBe('text');
  });

  it('falls back to the shell viewer for anything it cannot render safely', () => {
    expect(previewKindFor(entry('movie.mkv'))).toBe('none');
    expect(previewKindFor(entry('setup.exe'))).toBe('none');
    expect(previewKindFor(entry('no-extension'))).toBe('none');
  });

  it('never previews folders or cloud placeholders', () => {
    expect(previewKindFor(entry('Documents', { isDirectory: true }))).toBe('none');
    expect(previewKindFor(entry('photo.png', { isCloudPlaceholder: true }))).toBe('none');
  });
});

describe('name helpers', () => {
  it('splits extensions the way Explorer does', () => {
    expect(extensionOf('archive.tar.gz')).toBe('gz');
    expect(extensionOf('.env')).toBe('');
    expect(extensionOf('README')).toBe('');
  });

  it('pre-selects the stem for rename', () => {
    expect(stemOf('report.pdf')).toBe(6);
    expect(stemOf('.env')).toBe(4);
    expect(stemOf('README')).toBe(6);
  });
});
