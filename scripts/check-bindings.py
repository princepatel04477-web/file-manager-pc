"""Cross-check the hand-written specta-bindings.ts against the Rust DTOs.

`npm run build` cannot see Rust, so this walks the types reachable from the registered
Tauri commands, works out the JSON field names serde will emit for each one, and compares
them with the matching TypeScript declaration. Field *types* are not compared.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RUST_ROOT = os.path.join(ROOT, 'src-tauri', 'src')
TS = open(os.path.join(ROOT, 'src', 'lib', 'specta-bindings.ts'), encoding='utf-8').read()
LIB = open(os.path.join(RUST_ROOT, 'lib.rs'), encoding='utf-8').read()

SOURCES = {}
for base, _dirs, files in os.walk(RUST_ROOT):
    for name in files:
        if name.endswith('.rs'):
            path = os.path.join(base, name)
            SOURCES[path] = open(path, encoding='utf-8').read()
ALL_RUST = '\n'.join(SOURCES.values())

DERIVE = r'#\[derive\([^\)]*\bType\b[^\)]*\)\]\s*(?:#\[serde\([^\]]*\)\]\s*)*'


def lower_first(name: str) -> str:
    return name[:1].lower() + name[1:]


def camel(name: str) -> str:
    parts = name.split('_')
    return parts[0] + ''.join(part[:1].upper() + part[1:] for part in parts[1:])


def camel_variant(name: str) -> str:
    """serde's camelCase turns `Machine64` into `machine64`."""
    return lower_first(camel(name))


def parse_structs():
    for match in re.finditer(DERIVE + r'pub struct (\w+)\s*(?:<[^>]*>)?\s*\{([^}]*)\}', ALL_RUST):
        name, body = match.group(1), match.group(2)
        head = ALL_RUST[max(0, match.start(1) - 260):match.start(1)]
        if 'rename_all' not in head:
            continue
        fields = re.findall(r'pub (\w+)\s*:\s*([^,;]*),', body)
        yield name, {camel(field): kind.strip() for field, kind in fields}


def parse_enums():
    for match in re.finditer(DERIVE + r'pub enum (\w+)\s*\{([^}]*)\}', ALL_RUST):
        name, body = match.group(1), match.group(2)
        head = ALL_RUST[max(0, match.start(1) - 260):match.start(1)]
        if 'rename_all' not in head:
            continue
        variants = re.findall(r'^\s*([A-Z]\w*)\s*(\([^)]*\))?\s*,', body, flags=re.MULTILINE)
        if variants and all(payload == '' for _name, payload in variants):
            yield name, [camel_variant(variant) for variant, _payload in variants]


STRUCTS = dict(parse_structs())
ENUMS = dict(parse_enums())


def ts_members(name: str):
    match = re.search(r'export interface %s \{(.*?)\n\}' % re.escape(name), TS, flags=re.DOTALL)
    if match:
        return set(re.findall(r'^\s*(\w+)\??:', match.group(1), flags=re.MULTILINE)), 'interface'
    match = re.search(r'export type %s\s*=\s*([^;]+);' % re.escape(name), TS, flags=re.DOTALL)
    if match:
        return set(re.findall(r'"([^"]+)"', match.group(1))), 'union'
    return None, None


# --- the types the frontend can actually receive, starting from the command list -------
registered = re.search(r'collect_commands!\[(.*?)\]', LIB, flags=re.DOTALL).group(1)
roots = {name.strip() for name in registered.split(',') if name.strip()}

signature_types = set()
for command in roots:
    match = re.search(r'fn %s\s*(?:<[^>]*>)?\s*\((.*?)\)\s*->\s*([^\{;]+)\{' % re.escape(command), ALL_RUST, flags=re.DOTALL)
    if not match:
        continue
    signature_types.update(re.findall(r'\b([A-Z]\w*)\b', match.group(1) + ' ' + match.group(2)))

reachable = set()
queue = sorted(signature_types)
while queue:
    name = queue.pop()
    if name in reachable:
        continue
    reachable.add(name)
    if name in STRUCTS:
        for kind in STRUCTS[name].values():
            queue.extend(re.findall(r'\b([A-Z]\w*)\b', kind))

problems = []
checked = 0
KNOWN_INTERNAL = {'AppError', 'String', 'Result', 'Vec', 'Option', 'State', 'AppHandle', 'PathBuf'}
for name in sorted(reachable - KNOWN_INTERNAL - set(STRUCTS) - set(ENUMS)):
    if name in TS:
        problems.append('%s is declared in TypeScript but no serde camelCase Type was found in Rust' % name)
for name in sorted(reachable & (set(STRUCTS) | set(ENUMS))):
    found, shape = ts_members(name)
    if found is None:
        problems.append('%s is reachable from a command but missing from specta-bindings.ts' % name)
        continue
    checked += 1
    expected = set(STRUCTS[name]) if name in STRUCTS else set(ENUMS[name])
    missing = sorted(expected - found)
    extra = sorted(found - expected)
    if missing:
        problems.append('%s (%s) is missing %s' % (name, shape, ', '.join(missing)))
    if extra:
        problems.append('%s (%s) has extra %s' % (name, shape, ', '.join(extra)))

unreachable = sorted((set(STRUCTS) | set(ENUMS)) - reachable)
print('reachable types: %d   shared with TypeScript: %d' % (len(reachable), checked))
print('internal only (no TypeScript needed): %s' % ', '.join(unreachable) if unreachable else 'internal only: none')
for line in problems:
    print('FAIL ' + line)
if problems:
    sys.exit(1)
print('All %d command-facing types agree between Rust and specta-bindings.ts.' % checked)
