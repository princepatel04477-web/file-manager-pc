"""Prove the new db.rs SQL against a real SQLite database.

The SQL is extracted straight out of src-tauri/src/db.rs (no copy-paste), the Rust
string continuations are undone, and the numbered params are turned into qmarks.
"""
import os
import re
import sqlite3
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = open(os.path.join(ROOT, 'src-tauri', 'src', 'db.rs'), encoding='utf-8').read()


def unescape(raw: str) -> str:
    # Rust: a backslash at end-of-line eats the newline and the following indent.
    raw = re.sub(r'\\\s*\n\s*', ' ', raw)
    return raw.replace('\\\\', '\\').replace('\\"', '"')


def literal_at(opening: int) -> str:
    """opening = index of the opening double quote."""
    cursor = opening + 1
    body = ''
    while True:
        char = SRC[cursor]
        if char == '\\':
            body += SRC[cursor:cursor + 2]
            cursor += 2
            continue
        if char == '"':
            break
        body += char
        cursor += 1
    return unescape(body)


def literal_containing(marker: str) -> str:
    return literal_at(SRC.rindex('"', 0, SRC.index(marker)))


def schema() -> str:
    return literal_at(SRC.index('"', SRC.index('connection.execute_batch(')))


def query(marker: str) -> str:
    sql = literal_containing(marker)
    return re.sub(r'\?\d+', '?', sql)


connection = sqlite3.connect(':memory:')
connection.executescript(schema())

rows = [
    # path, parent, name, category, size, mtime, cloud, directory
    ('C:\\Users\\me\\Videos\\movie.mkv', 'C:\\Users\\me\\Videos', 'movie.mkv', 'video', 900_000_000, 1_700_000_000, 0, 0),
    ('C:\\Users\\me\\Videos\\backup.mkv', 'C:\\Users\\me\\Videos', 'backup.mkv', 'video', 900_000_000, 1_700_000_500, 0, 0),
    ('C:\\Users\\me\\Documents\\report.pdf', 'C:\\Users\\me\\Documents', 'report.pdf', 'document', 250_000_000, 1_600_000_000, 0, 0),
    ('C:\\Users\\me\\Documents\\small.txt', 'C:\\Users\\me\\Documents', 'small.txt', 'document', 40, 1_600_000_001, 0, 0),
    ('C:\\Users\\me\\Documents\\small copy.txt', 'C:\\Users\\me\\Documents', 'small copy.txt', 'document', 40, 1_600_000_002, 0, 0),
    ('C:\\Users\\me\\Downloads\\old-installer.exe', 'C:\\Users\\me\\Downloads', 'old-installer.exe', 'app', 80_000_000, 1_500_000_000, 0, 0),
    ('C:\\Users\\me\\Downloads\\nested\\old-archive.zip', 'C:\\Users\\me\\Downloads\\nested', 'old-archive.zip', 'archive', 90_000_000, 1_500_000_100, 0, 0),
    ('C:\\Users\\me\\Downloads\\fresh.pdf', 'C:\\Users\\me\\Downloads', 'fresh.pdf', 'document', 1_000, 1_900_000_000, 0, 0),
    ('C:\\Users\\me\\Downloads 2\\not-a-download.bin', 'C:\\Users\\me\\Downloads 2', 'not-a-download.bin', 'other', 123, 1_500_000_200, 0, 0),
    ('C:\\Users\\me\\Pictures\\Screenshots\\2026-01-01.png', 'C:\\Users\\me\\Pictures\\Screenshots', '2026-01-01.png', 'image', 400_000, 1_760_000_000, 0, 0),
    ('C:\\Users\\me\\Desktop\\Screenshot 2026-02-02.png', 'C:\\Users\\me\\Desktop', 'Screenshot 2026-02-02.png', 'image', 300_000, 1_760_000_100, 0, 0),
    ('C:\\Users\\me\\Desktop\\snipping tool shot.png', 'C:\\Users\\me\\Desktop', 'snipping tool shot.png', 'image', 200_000, 1_760_000_200, 0, 0),
    ('C:\\Users\\me\\Desktop\\capture-card-driver.zip', 'C:\\Users\\me\\Desktop', 'capture-card-driver.zip', 'archive', 500, 1_760_000_300, 0, 0),
    ('C:\\Users\\me\\Videos\\cloud-only.mkv', 'C:\\Users\\me\\Videos', 'cloud-only.mkv', 'video', 900_000_000, 1_700_000_600, 1, 0),
    ('C:\\Users\\me\\Videos\\folder', 'C:\\Users\\me\\Videos', 'folder', 'folder', 900_000_000, 1_700_000_700, 0, 1),
    ('C:\\Users\\me\\Documents\\no-time.txt', 'C:\\Users\\me\\Documents', 'no-time.txt', 'document', 5, None, 0, 0),
]
connection.executemany(
    'INSERT INTO files(path,parent_path,name,ext,category,size,mtime,ctime,is_hidden,is_cloud,is_directory,drive,last_seen) '
    'VALUES(?,?,?,?,?,?,?,?,0,?,?,?,1)',
    [(r[0], r[1], r[2], r[2].rsplit('.', 1)[-1], r[3], r[4], r[5], r[5], r[6], r[7], 'C:') for r in rows],
)
connection.commit()

failures = []


def check(label, actual, expected):
    ok = actual == expected
    print(('PASS ' if ok else 'FAIL ') + label)
    if not ok:
        print('   expected: %r' % (expected,))
        print('   actual:   %r' % (actual,))
        failures.append(label)


large = query('FROM files WHERE is_directory = 0 AND size >= ?1')
names = [row[3] for row in connection.execute(large, (100 * 1024 * 1024, 250))]
# Same-size files fall back to name order, so the three 900 MB entries are alphabetical.
check('large_files: over 100 MB, largest first, no folders', names,
      ['backup.mkv', 'cloud-only.mkv', 'movie.mkv', 'report.pdf'])
check('large_files: limit is honoured',
      [row[3] for row in connection.execute(large, (100 * 1024 * 1024, 2))], ['backup.mkv', 'cloud-only.mkv'])

older = query('FROM files WHERE is_directory = 0 AND substr(path, 1, length(?1)) = ?1')
prefix = 'C:\\Users\\me\\Downloads\\'
names = [row[3] for row in connection.execute(older, (prefix, prefix, 1_700_000_000, 250))]
check('files_older_than_under: nested entries included, sibling folder excluded, oldest first', names,
      ['old-installer.exe', 'old-archive.zip'])
names = [row[3] for row in connection.execute(older, (prefix, prefix, 1_950_000_000, 250))]
check('files_older_than_under: a later cutoff adds the recent download', names,
      ['old-installer.exe', 'old-archive.zip', 'fresh.pdf'])
documents = 'C:\\Users\\me\\Documents\\'
check('files_older_than_under: rows without an mtime are never returned',
      [row[3] for row in connection.execute(older, (documents, documents, 1_950_000_000, 250))],
      ['report.pdf', 'small.txt', 'small copy.txt'])

shots = query("OR lower(name) LIKE '%snipping%'")
screenshots = 'C:\\Users\\me\\Pictures\\Screenshots\\'
shot_rows = list(connection.execute(shots, (screenshots,) * 4 + (250,)))
check('screenshot_candidates: broad name net + folder membership flag',
      [(row[3], row[13]) for row in shot_rows],
      [('2026-01-01.png', 1), ('Screenshot 2026-02-02.png', 0), ('snipping tool shot.png', 0),
       ('capture-card-driver.zip', 0)])  # size DESC, then oldest first

groups_sql = query('FROM files WHERE is_directory = 0 AND is_cloud = 0 AND size >= ?1')
groups = [row[0] for row in connection.execute(groups_sql, (1024, 900))]
check('duplicate_size_groups: only shared sizes, biggest total first, cloud + folders excluded',
      groups, [900_000_000])

members_sql = query('FROM files WHERE is_directory = 0 AND is_cloud = 0 AND size IN (')
members_sql = members_sql.replace('{placeholders}', ','.join(['?'] * len(groups)))
members = [row[3] for row in connection.execute(members_sql, groups)]
check('duplicate_candidates: the members of those groups, cloud copy excluded', members,
      ['backup.mkv', 'movie.mkv'])  # path order inside a size group

print('')
if failures:
    print('%d SQL CHECK(S) FAILED: %s' % (len(failures), ', '.join(failures)))
    sys.exit(1)
print('ALL SQL CHECKS PASSED')
