# SPDX-License-Identifier: GPL-2.0-only

import argparse
import ast
import datetime
import json
import re
import sys
import tomllib
from pathlib import Path, PurePosixPath

MAX_BYTES = 1_048_576
MAX_DEPTH = 8
MAX_INCLUDES = 64
MAX_PROFILES = 4096
MAX_MATCHES = 8
MAX_DEVICES = 64
MAX_FEATURES = 64
MAX_PCI = 256
MAX_PCI_PRODUCTS = 65536
MAX_IDENT = 64
MAX_ITEMS = 4096
MAX_NODES = 65536
MAX_RUNS = 10000
MAX_TREE_DEPTH = 16
MAX_TREE_FILES = 16384
STALE_DAYS = 90
LICENSE = 'SPDX-License-Identifier: GPL-2.0-only'
NOTES = '<!-- profile-notes -->\n'
FEATURES = 'boot serial-console display opengl vulkan second-display disk network sound microphone keyboard pointer usb tpm iommu power-off installer'.split()
LAPTOP_FEATURES = 'battery lid brightness suspend wifi bluetooth webcam fingerprint'.split()
KINDS = 'vm cloud laptop desktop server board'.split()
ARCHES = ['x86_64', 'aarch64', 'riscv64gc']
DMI = 'sys_vendor product_name product_family product_sku board_vendor board_name bios_vendor chassis_type'.split()
FILE_PREFIXES = ['/etc/pcid.d/', '/usr/lib/pcid.d/', '/etc/xhcid.d/', '/usr/lib/init.d/', '/etc/ferrion-hardware/']
SCHEMA = {
    'root': {'include': 'strings', 'packages': 'table', 'files': 'tables', 'general': 'table', 'users': 'table', 'groups': 'table', 'profile': 'table', 'match': 'tables', 'vm': 'table', 'iommu': 'table', 'boot': 'table', 'device': 'tables', 'feature': 'tables'},
    'profile': {'format': 'int', 'name': 'str', 'title': 'str', 'kind': 'str', 'arch': 'strings', 'boot': 'str', 'private': 'bool'},
    'match': {'hypervisor': 'strings', 'dmi': 'table', 'pci_all': 'strings', 'pci_none': 'strings'},
    'vm': {'tool': 'str', 'make': 'strings', 'host_needs': 'strings'},
    'iommu': {'unit': 'str'},
    'general': {'efi_partition_size': 'int'},
    'files': {'path': 'str', 'data': 'str', 'mode': 'int'},
    'device': {'id': 'str', 'name': 'str', 'bus': 'str', 'ids': 'strings', 'class': 'str', 'route': 'str', 'driver': 'str', 'linux_driver': 'str', 'stage': 'str', 'firmware': 'tables', 'needs': 'strings', 'acpi_methods': 'strings', 'gap': 'table'},
    'firmware': {'package': 'str', 'supplied_by': 'str'},
    'gap': {'plan': 'str', 'from': 'strings', 'slice': 'str'},
    'feature': {'id': 'str', 'name': 'str', 'device': 'str', 'checks': 'strings', 'conditions': 'strings'},
    'evidence': {'run': 'tables'},
    'run': {'check': 'str', 'verdict': 'str', 'passes': 'int', 'runs': 'int', 'date': 'str', 'ferrion': 'str', 'hardware': 'str', 'config': 'str', 'make': 'strings', 'conditions': 'strings', 'qemu': 'str', 'record': 'str', 'imported_from': 'str', 'renderer': 'str'},
    'machine': {'arch': 'str', 'hypervisor': 'str', 'dmi': 'table', 'pci': 'strings'},
}
REQUIRED_RUN = set(SCHEMA['run']) - {'imported_from', 'renderer'}
IDENTIFIER = re.compile(r'(?i)(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}|[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|(?:serial[ _]?(?:number|no\.?)|s/n|wwn|wwid|eui\.|nguid)[:= ]+\S+')
HOST_PATH = re.compile(r'(?:^|[\s"\'=])(?:/|~/|[A-Za-z]:[\\/])')
PCI_ID = re.compile(r'[0-9a-f]{4}:[0-9a-f]{4}')
PCI_PATTERN = re.compile(r'[0-9a-f]{4}:(?:[0-9a-f]{4}|\*)')
TOKEN = re.compile(r'[a-z0-9][a-z0-9_.-]*')


class Invalid(ValueError):
    pass


def require(condition, rule, message):
    if not condition:
        raise Invalid(f'R{rule:02}: {message}')


def bounded(values, limit, label, rule=1):
    require(len(values) <= limit, rule, f'{label} exceeds {limit}')
    return values


def text(path):
    with path.open('rb') as stream:
        data = stream.read(MAX_BYTES + 1)
    bounded(data, MAX_BYTES, 'file bytes')
    return data.decode('utf-8')


def tree_bound(value, rule=1):
    pending = [(value, 0)]
    count = 0
    while pending:
        node, depth = pending.pop()
        count += 1
        require(count <= MAX_NODES, rule, 'data nodes exceed limit')
        require(depth <= MAX_TREE_DEPTH, rule, 'data nesting exceeds limit')
        if isinstance(node, dict):
            bounded(node, MAX_ITEMS, 'table entries', rule)
            pending.extend((child, depth + 1) for child in node.values())
        elif isinstance(node, list):
            bounded(node, MAX_RUNS, 'array entries', rule)
            pending.extend((child, depth + 1) for child in node)
        elif isinstance(node, str):
            bounded(node.encode(), MAX_BYTES, 'string bytes', rule)


def parse(path):
    try:
        value = tomllib.loads(text(path))
    except (tomllib.TOMLDecodeError, UnicodeError) as error:
        raise Invalid(f'R01: invalid TOML in {path.name}: {error}') from error
    tree_bound(value)
    return value


def shape(value, section, rule=1):
    require(type(value) is dict, rule, f'{section} must be a table')
    for key, item in value.items():
        require(key in SCHEMA[section], rule, f'unknown {section} key {key}')
        kind = SCHEMA[section][key]
        types = {'str': str, 'int': int, 'bool': bool, 'table': dict, 'tables': list, 'strings': list}
        require(type(item) is types[kind], rule, f'{section}.{key} must be {kind}')
        if kind in ('tables', 'strings'):
            bounded(item, MAX_ITEMS, f'{section}.{key}', rule)
            element = dict if kind == 'tables' else str
            require(all(type(entry) is element for entry in item), rule, f'{section}.{key} has wrong element type')


def required(value, keys, label, rule=1):
    require(set(keys) <= value.keys(), rule, f'{label} missing {", ".join(sorted(set(keys) - value.keys()))}')


def identifier(value, rule=4):
    bounded(value.encode(), MAX_IDENT, 'identification string bytes', rule)
    require(not IDENTIFIER.search(value) and not HOST_PATH.search(value), rule, 'identification value contains a private identifier or host path')


def strings_safe(value, rule=10):
    pending = [value]
    while pending:
        node = pending.pop()
        if isinstance(node, dict):
            pending.extend(node.values())
        elif isinstance(node, list):
            pending.extend(node)
        elif isinstance(node, str):
            require(not HOST_PATH.search(node), rule, 'evidence contains a host path')
            require(not IDENTIFIER.search(node), rule, 'evidence contains a private identifier')


def scan_files(root):
    pending = [(root, 0)]
    result = []
    seen = 0
    while pending:
        directory, depth = pending.pop()
        require(depth <= MAX_TREE_DEPTH, 1, 'directory depth exceeds limit')
        for child in directory.iterdir():
            seen += 1
            require(seen <= MAX_TREE_FILES, 1, 'directory entries exceed limit')
            require(not child.is_symlink(), 2, 'symbolic links are not profile inputs')
            if child.is_dir():
                pending.append((child, depth + 1))
            elif child.is_file():
                result.append(child)
    return sorted(result)


def validate_source(data):
    shape(data, 'root')
    pending = [data]
    while pending:
        node = pending.pop()
        if isinstance(node, dict):
            require(node.get('private') is not True, 12, 'private profile is forbidden in the public catalog')
            pending.extend(node.values())
        elif isinstance(node, list):
            pending.extend(node)
    require('users' not in data and 'groups' not in data, 3, 'profiles cannot add accounts')
    require('boot' not in data, 3, 'boot settings require HP6; IOMMU mode is never a profile option')
    for section in ('profile', 'vm', 'iommu', 'general'):
        if section in data:
            shape(data[section], section, 3 if section == 'general' else 1)
    for name, rule in data.get('packages', {}).items():
        require(TOKEN.fullmatch(name) and rule == {}, 3, 'packages must have empty rules')
    for entry in data.get('files', []):
        shape(entry, 'files', 3)
        required(entry, ['path', 'data'], 'file', 3)
        path = PurePosixPath(entry['path'])
        require(str(path) == entry['path'] and '..' not in path.parts and any(entry['path'].startswith(prefix) for prefix in FILE_PREFIXES), 3, 'file path is outside the allow-list')
        mode = entry.get('mode', 0o644)
        require(0 <= mode <= 0o7777 and not mode & 0o6002, 3, 'file mode is privileged or world-writable')
    for section, limit in [('match', MAX_MATCHES), ('device', MAX_DEVICES), ('feature', MAX_FEATURES)]:
        entries = bounded(data.get(section, []), limit, section)
        for entry in entries:
            shape(entry, section, 4 if section == 'match' else 9 if section == 'feature' else 1)
        if section != 'match':
            ids = [entry.get('id') for entry in entries]
            require(None not in ids and len(set(ids)) == len(ids), 1, f'duplicate or missing {section} id')


def merge(left, right):
    out = dict(left)
    for key, value in right.items():
        if key in ('include', 'match'):
            continue
        if key in ('device', 'feature'):
            entries = {item['id']: item for item in out.get(key, [])}
            entries.update((item['id'], item) for item in value)
            out[key] = list(entries.values())
        elif key == 'files':
            out[key] = out.get(key, []) + value
        elif isinstance(value, dict):
            out[key] = out.get(key, {}) | value
        else:
            out[key] = value
    return out


def load_profile(path, root):
    count = 0

    def visit(current, ancestors):
        nonlocal count
        require(current.is_relative_to(root), 2, 'include leaves the repository')
        require(current not in ancestors, 2, 'include cycle')
        require(len(ancestors) <= MAX_DEPTH, 1, 'include depth exceeds 8')
        count += 1
        require(count <= MAX_INCLUDES, 1, 'included files exceed 64')
        data = parse(current)
        validate_source(data)
        out = {}
        for name in bounded(data.get('include', []), MAX_INCLUDES, 'includes'):
            require(not Path(name).is_absolute(), 2, 'absolute include is forbidden')
            child = (current.parent / name).resolve()
            out = merge(out, visit(child, ancestors + (current,)))
        out = merge(out, data)
        out['match'] = data.get('match', [])
        return out

    return visit(path.resolve(), ())


def validate_match(block, kind):
    shape(block, 'match', 4)
    require(bool(block) and any(block.values()), 4, 'empty match block')
    for key, value in block.get('dmi', {}).items():
        require(key in DMI or key == 'chassis_asset_tag' and kind == 'cloud', 4, f'forbidden match key {key}')
        require(type(value) is str, 4, 'DMI values must be strings')
        identifier(value)
        if key == 'chassis_asset_tag':
            require(value == '7783-7084-3265-9085-8269-3286-77', 4, 'cloud asset tag must be a published constant')
    for value in bounded(block.get('hypervisor', []), MAX_MATCHES, 'hypervisor signatures'):
        identifier(value)
    for key in ('pci_all', 'pci_none'):
        for value in bounded(block.get(key, []), MAX_PCI, 'PCI matches'):
            require(PCI_ID.fullmatch(value), 4, 'match PCI IDs must be vendor:device hex')
    require(not set(block.get('pci_all', [])) & set(block.get('pci_none', [])), 4, 'contradictory PCI match')


def validate_profile(data, name):
    required(data, ['profile', 'match', 'device', 'feature'], 'profile')
    profile = data['profile']
    required(profile, ['format', 'name', 'title', 'kind', 'arch', 'boot'], 'profile')
    require(profile['format'] == 1, 1, 'unsupported profile format')
    require(profile['name'] == name, 1, 'profile name differs from its folder')
    require(profile['kind'] in KINDS, 1, 'unknown profile kind')
    require(profile['boot'] in ('uefi', 'bios', 'u-boot', 'none'), 1, 'unknown boot protocol')
    require(profile['arch'] and set(profile['arch']) <= set(ARCHES), 1, 'unknown architecture')
    require(all(TOKEN.fullmatch(part) for part in name.split('/')) and len(name.split('/')) == 2, 1, 'profile folder must be vendor/model')
    if 'vm' in data:
        require(data['vm'].get('tool') in ('qemu', 'cloud-hypervisor', 'virtualbox', 'none'), 1, 'unknown VM tool')
        for item in data['vm'].get('make', []):
            require(re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*=[A-Za-z0-9_.+-]+', item), 1, 'invalid make assignment')
    require(data.get('iommu', {}).get('unit', 'none') in ('none', 'vt-d', 'amd-vi', 'virtio', 'smmu'), 1, 'unknown IOMMU unit')
    for block in bounded(data['match'], MAX_MATCHES, 'match blocks'):
        validate_match(block, profile['kind'])
    if profile['kind'] in ('vm', 'cloud'):
        require(any(not block.get('dmi') for block in data['match']), 5, 'VM or cloud profile needs a match without SMBIOS')
    devices = {}
    for device in bounded(data['device'], MAX_DEVICES, 'devices'):
        required(device, ['id', 'name', 'bus', 'route'], 'device')
        require(TOKEN.fullmatch(device['id']), 1, 'invalid device id')
        require(device['bus'] in ('pci', 'acpi', 'usb', 'vmbus', 'mmio', 'platform'), 1, 'unknown device bus')
        require(device['route'] in ('native', 'host', 'firmware', 'none'), 1, 'unknown device route')
        if device['route'] in ('native', 'host'):
            required(device, ['driver', 'stage'], 'driven device')
            require(device['stage'] in ('initfs', 'rootfs'), 1, 'unknown driver stage')
        if device['route'] == 'host':
            required(device, ['linux_driver'], 'host device')
        if device['route'] == 'none':
            required(device, ['gap'], 'unhandled device')
        if 'gap' in device:
            shape(device['gap'], 'gap')
            required(device['gap'], SCHEMA['gap'], 'gap')
            require(device['gap']['plan'] in ('native', 'host') and device['gap']['from'], 1, 'gap needs a route and source')
        if device['bus'] == 'pci':
            required(device, ['ids', 'class'], 'PCI device')
            require(re.fullmatch(r'[0-9a-f]{6}', device['class']), 1, 'PCI class must have six hex digits')
            require(device['ids'], 1, 'PCI device needs IDs')
            for value in bounded(device['ids'], MAX_PCI, 'device IDs'):
                require(PCI_PATTERN.fullmatch(value), 1, 'PCI device needs vendor:device IDs or vendor:*')
        for firmware in device.get('firmware', []):
            shape(firmware, 'firmware')
            required(firmware, SCHEMA['firmware'], 'firmware')
        for method in device.get('acpi_methods', []):
            require(re.fullmatch(r'\\?(?:[A-Z_][A-Z0-9_]{3}\.)*[A-Z_][A-Z0-9_]{3}', method), 1, 'invalid ACPI method')
        devices[device['id']] = device
    features = bounded(data['feature'], MAX_FEATURES, 'features')
    vocabulary = FEATURES + (LAPTOP_FEATURES if profile['kind'] == 'laptop' else [])
    require({entry['id'] for entry in features} == set(vocabulary), 9, 'feature vocabulary is incomplete or unknown')
    for feature in features:
        required(feature, ['id', 'name', 'checks'], 'feature', 9)
        require('device' not in feature or feature['device'] in devices, 9, 'feature references an unknown device')
        require(len(set(feature['checks'])) == len(feature['checks']), 9, 'duplicate feature check')
        for check in feature['checks']:
            require(TOKEN.fullmatch(check), 9, 'invalid check name')
    return devices


def evidence(path, data):
    value = parse(path)
    shape(value, 'evidence', 10)
    rows = bounded(value.get('run', []), MAX_RUNS, 'evidence rows', 10)
    checks = {check for feature in data['feature'] for check in feature['checks']}
    for row in rows:
        shape(row, 'run', 10)
        required(row, REQUIRED_RUN, 'evidence row', 10)
        require(row['check'] in checks, 10, 'evidence check is not used by a feature')
        require(row['verdict'] in ('pass', 'fail', 'flaky', 'blocked'), 10, 'unknown evidence verdict')
        require(0 <= row['passes'] <= row['runs'] <= MAX_RUNS and row['runs'] > 0, 10, 'invalid evidence counts')
        expected = 'pass' if row['passes'] == row['runs'] else 'flaky' if row['passes'] else 'fail'
        require(row['verdict'] == expected or row['verdict'] == 'blocked' and row['passes'] == 0, 10, 'evidence verdict disagrees with counts')
        try:
            day = datetime.date.fromisoformat(row['date'])
        except ValueError as error:
            raise Invalid('R10: invalid evidence date') from error
        require(day <= datetime.date.today(), 10, 'evidence date is in the future')
        for revision in ('ferrion', 'hardware'):
            require(re.fullmatch(r'[0-9a-f]{7,40}', row[revision]) or row[revision] == 'unknown' and row['record'] == 'none', 10, 'invalid evidence revision')
        require(re.fullmatch(r'[0-9]+\.[0-9]+(?:\.[0-9]+)?', row['qemu']) or row['qemu'] == 'unknown' and row['record'] == 'none', 10, 'QEMU must contain only its version')
        if row['record'] == 'none':
            require(bool(row.get('imported_from')), 10, 'imported evidence needs a source')
        else:
            require(re.fullmatch(r'[0-9a-f]{64}', row['record']), 10, 'record must be a BLAKE3 digest')
            require('imported_from' not in row, 10, 'recorded evidence cannot claim an imported source')
        require(row.get('renderer', 'host GPU') == 'host GPU', 10, 'renderer must be host GPU')
        strings_safe(row)
    return rows


def latest_runs(rows):
    latest = {}
    for row in rows:
        if row['check'] not in latest or row['date'] >= latest[row['check']]['date']:
            latest[row['check']] = row
    return latest


def check_host_routes(data, devices, rows):
    if data['profile']['kind'] in ('vm', 'cloud'):
        return
    latest = latest_runs(rows)
    iommu = next(feature for feature in data['feature'] if feature['id'] == 'iommu')
    strict = any(check in latest and latest[check]['verdict'] == 'pass' and 'iommu-strict' in latest[check]['conditions'] for check in iommu['checks'])
    for device in devices.values():
        if device['route'] == 'host':
            require({'iommu-strict', 'device-grant'} <= set(device.get('needs', [])), 8, 'physical host device needs iommu-strict and device-grant')
            require(strict, 8, 'physical host device lacks passing strict-mode IOMMU evidence')


def score(data, machine):
    if machine['arch'] not in data['profile']['arch']:
        return -1
    scores = []
    pci = set(machine.get('pci', []))
    for block in data['match']:
        if block.get('hypervisor') and machine.get('hypervisor') not in block['hypervisor']:
            continue
        if not set(block.get('pci_all', [])) <= pci or set(block.get('pci_none', [])) & pci:
            continue
        if any(machine.get('dmi', {}).get(key) != value for key, value in block.get('dmi', {}).items()):
            continue
        scores.append(100 * len(block.get('dmi', {})) + 10 * len(block.get('pci_all', [])) + bool(block.get('hypervisor')))
    return max(scores, default=-1)


def effect(data):
    return {key: data.get(key, {} if key != 'files' else []) for key in ('packages', 'files', 'general')}


def check_matches(catalog):
    for name, (data, _, _) in catalog.items():
        pci = [value.replace('*', '0000') for device in data['device'] if device['bus'] == 'pci' for value in device['ids']]
        bounded(pci, MAX_PCI, 'profile PCI functions')
        for block in data['match']:
            described = list(pci)
            for value in block.get('pci_all', []):
                if value not in described and any(value.startswith(pattern[:5]) for device in data['device'] if device['bus'] == 'pci' for pattern in device['ids'] if pattern.endswith('*')):
                    described.append(value)
            for arch in data['profile']['arch']:
                for hypervisor in block.get('hypervisor', ['']):
                    machine = {'arch': arch, 'hypervisor': hypervisor, 'dmi': block.get('dmi', {}), 'pci': described}
                    own = score(data, machine)
                    require(own >= 0, 6, f'{name} does not match its own devices')
                    for other, (candidate, _, _) in catalog.items():
                        rank = score(candidate, machine)
                        require(rank < own or rank == own and effect(candidate) == effect(data), 6, f'{name} is outranked or has an unequal-effect tie with {other}')


def driver_entries(value):
    require(isinstance(value.get('drivers'), list), 7, 'invalid driver table')
    entries = bounded(value['drivers'], MAX_ITEMS, 'driver entries', 7)
    for entry in entries:
        require(isinstance(entry, dict), 7, 'invalid driver entry')
        command = entry.get('command')
        require(isinstance(command, list) and command and all(isinstance(arg, str) and arg for arg in command), 7, 'driver command is empty or invalid')
        for key in ('class', 'subclass', 'interface', 'vendor', 'device'):
            if key in entry:
                require(type(entry[key]) is int and 0 <= entry[key] <= (255 if key in ('class', 'subclass', 'interface') else 65535), 7, 'invalid numeric driver match')
        if 'ids' in entry:
            require(isinstance(entry['ids'], dict), 7, 'driver IDs must be a table')
            for vendor, ids in entry['ids'].items():
                require(re.fullmatch(r'(?:0x)?[0-9a-fA-F]{1,4}', vendor) and isinstance(ids, list) and all(type(item) is int and 0 <= item <= 65535 for item in ids), 7, 'invalid driver IDs')
        if 'device_id_range' in entry:
            span = entry['device_id_range']
            require(isinstance(span, dict) and set(span) == {'start', 'end'} and all(type(item) is int and 0 <= item <= 65535 for item in span.values()) and span['start'] < span['end'], 7, 'invalid device ID range')
    return entries


def driver_tables(root):
    initfs = driver_entries(parse(root / 'initfs.toml'))
    files = {}
    for path in scan_files(root):
        if path.name == 'config.toml' and len(path.relative_to(root).parts) <= 3:
            name = path.parent.name + '.toml'
            require(name not in files, 7, 'duplicate installed driver filename')
            files[name] = driver_entries(parse(path))
    return initfs, files


def driver_matches(entry, pci, device):
    vendor, product = (int(part, 16) for part in pci.split(':'))
    for key, offset in [('class', 0), ('subclass', 2), ('interface', 4)]:
        if key in entry and entry[key] != int(device['class'][offset:offset + 2], 16):
            return False
    if 'ids' in entry:
        if not any(int(key, 16) == vendor and product in values for key, values in entry['ids'].items()):
            return False
    elif entry.get('vendor', vendor) != vendor or entry.get('device', product) != product:
        return False
    span = entry.get('device_id_range', {'start': 0, 'end': 65536})
    return span['start'] <= product < span['end']


def check_drivers(data, tables):
    initfs, stock = tables
    lib, etc = dict(stock), {}
    for item in data.get('files', []):
        path = PurePosixPath(item['path'])
        if str(path.parent) not in ('/etc/pcid.d', '/usr/lib/pcid.d'):
            continue
        try:
            entries = driver_entries(tomllib.loads(item['data']))
        except tomllib.TOMLDecodeError as error:
            raise Invalid('R07: invalid handwritten pcid.d file') from error
        (etc if str(path.parent) == '/etc/pcid.d' else lib)[path.name] = entries
    rootfs = lib | etc
    ordered = [('initfs', initfs)] + [('rootfs', rootfs[name]) for name in sorted(rootfs)]
    for device in data['device']:
        if device['bus'] != 'pci':
            continue
        for pattern in device['ids']:
            ids = (f'{pattern[:5]}{product:04x}' for product in range(MAX_PCI_PRODUCTS)) if pattern.endswith('*') else (pattern,)
            for pci in ids:
                claim = next(((Path(entry['command'][0]).name, stage) for stage, entries in ordered for entry in entries if driver_matches(entry, pci, device)), None)
                expected = None if device['route'] in ('none', 'firmware') else (device['driver'], device['stage'])
                require(claim == expected, 7, f'{device["id"]} {pci}: expected {expected}, spawner selects {claim}')


def check_names(path):
    source = text(path)
    if path.suffix != '.py':
        return set(bounded(source.splitlines(), MAX_ITEMS, 'check names'))
    module = ast.parse(source)
    names = set()
    for node in bounded(module.body, MAX_NODES, 'Python statements'):
        if not isinstance(node, ast.Assign):
            continue
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id in ('TESTS', 'OWN_IMAGE') and isinstance(node.value, ast.Dict):
                names.update(key.value for key in node.value.keys if isinstance(key, ast.Constant) and isinstance(key.value, str))
            if isinstance(target, ast.Subscript) and isinstance(target.value, ast.Name) and target.value.id in ('TESTS', 'OWN_IMAGE') and isinstance(target.slice, ast.Constant) and isinstance(target.slice.value, str):
                names.add(target.slice.value)
    require(names, 9, 'check registry has no check names')
    return names


def private_patterns(root, files):
    git = root / '.git'
    if git.is_file():
        pointer = text(git).strip()
        require(pointer.startswith('gitdir: '), 12, 'invalid git directory pointer')
        git = (root / pointer[8:]).resolve()
    locations = [git / 'info/ferrion-private-patterns']
    common = git / 'commondir'
    if common.is_file():
        locations.append((git / text(common).strip()).resolve() / 'info/ferrion-private-patterns')
    patterns = []
    for location in locations:
        if location.is_file():
            patterns.extend(line.casefold() for line in text(location).splitlines() if line.strip())
    bounded(patterns, MAX_ITEMS, 'private patterns', 12)
    for path in files:
        content = text(path).casefold()
        require(not any(pattern in content for pattern in patterns), 12, 'local private pattern matched a public file')


def load_catalog(root, drivers=None, checks=None):
    files = scan_files(root / 'profiles')
    if (root / 'common').exists():
        common = scan_files(root / 'common')
        for path in common:
            if path.suffix == '.toml':
                validate_source(parse(path))
        files += common
    private_patterns(root, files)
    paths = bounded([path for path in files if path.name == 'profile.toml'], MAX_PROFILES, 'profiles')
    require(paths, 1, 'catalog contains no profiles')
    catalog = {}
    tables = driver_tables(drivers) if drivers else None
    known = check_names(checks) if checks else None
    for path in paths:
        name = path.parent.relative_to(root / 'profiles').as_posix()
        data = load_profile(path, root)
        devices = validate_profile(data, name)
        rows = evidence(path.parent / 'evidence.toml', data)
        check_host_routes(data, devices, rows)
        if tables:
            check_drivers(data, tables)
        if known is not None:
            for feature in data['feature']:
                require(set(feature['checks']) <= known, 9, f'unknown check for {feature["id"]}')
        catalog[name] = (data, rows, path.parent)
    check_matches(catalog)
    return catalog


def cell(feature, devices, latest):
    device = devices.get(feature.get('device'))
    if device and device['route'] == 'none':
        return 'no driver', f'{device["gap"]["plan"]}: {", ".join(device["gap"]["from"])}; {device["gap"]["slice"]}'
    if not feature['checks'] or any(check not in latest for check in feature['checks']):
        return 'not shown', '; '.join(feature.get('conditions', []))
    rows = [latest[check] for check in feature['checks']]
    if all(row['verdict'] == 'pass' for row in rows):
        status = 'works'
    elif any(row['verdict'] == 'flaky' for row in rows) or any(row['passes'] for row in rows) and any(row['verdict'] == 'fail' for row in rows):
        status = 'flaky'
    elif any(row['verdict'] == 'fail' for row in rows):
        status = 'fails'
    else:
        status = 'blocked'
    if any(row['record'] == 'none' for row in rows):
        status += ' (imported)'
    details = []
    for row in rows:
        detail = f'{row["check"]}: {row["passes"]}/{row["runs"]}; FerrionOS {row["ferrion"]}; {row["date"]}; QEMU {row["qemu"]}'
        conditions = row['conditions'] + feature.get('conditions', [])
        if row['make']:
            conditions = row['make'] + conditions
        if row.get('renderer'):
            conditions += [row['renderer']]
        if conditions:
            detail += '; ' + ', '.join(conditions)
        if row.get('imported_from'):
            detail += '; ' + row['imported_from']
        age = (datetime.date.today() - datetime.date.fromisoformat(row['date'])).days
        if age > STALE_DAYS:
            detail += f'; stale ({age} days)'
        details.append(detail)
    return status, '<br>'.join(details)


def md(value):
    return str(value).replace('&', '&amp;').replace('|', '&#124;').replace('\n', ' ').replace('<', '&lt;').replace('>', '&gt;')


def readme(data, rows, directory):
    lines = [f'<!-- {LICENSE} -->', '', '# ' + data['profile']['title'], '', 'Generated by `python3 scripts/profile.py readme`. Format: `python3 scripts/profile.py schema`.', '']
    vm = data.get('vm', {})
    if vm:
        lines += ['Start: `make ' + vm['tool'] + (' ' + ' '.join(vm.get('make', [])) if vm.get('make') else '') + '`.', '']
        if vm.get('host_needs'):
            lines += ['Requires: ' + '; '.join(vm['host_needs']) + '.', '']
    lines += ['## Devices', '', '| Device | Bus and IDs | Route | Driver and stage |', '|---|---|---|---|']
    devices = {device['id']: device for device in data['device']}
    for device in devices.values():
        lines.append('| ' + ' | '.join(md(value) for value in [device['name'], device['bus'] + ' ' + ', '.join(device.get('ids', [])), device['route'], device.get('driver', 'none') + ' ' + device.get('stage', '')]) + ' |')
    lines += ['', '## Image additions', '', 'Packages: ' + (', '.join(sorted(data.get('packages', {}))) or 'none') + '.', '', '| File | Hardware right |', '|---|---|']
    for item in data.get('files', []):
        hardware = 'not a unit'
        if item['path'].startswith('/usr/lib/init.d/'):
            try:
                unit = tomllib.loads(item['data'])
            except tomllib.TOMLDecodeError as error:
                raise Invalid('R03: init unit must be valid TOML') from error
            hardware = 'yes' if unit.get('service', {}).get('hardware') is True else 'no'
        lines.append(f'| {md(item["path"])} | {hardware} |')
    if not data.get('files'):
        lines.append('| none | none |')
    for device in devices.values():
        for firmware in device.get('firmware', []):
            lines += ['', f'Firmware: {md(firmware["package"])}; supplied by {md(firmware["supplied_by"])}.']
    lines += ['', '## What works', '', '| Feature | State | Evidence and conditions |', '|---|---|---|']
    latest = latest_runs(rows)
    for feature in data['feature']:
        status, detail = cell(feature, devices, latest)
        lines.append(f'| {md(feature["name"])} | {status} | {md(detail).replace("&lt;br&gt;", "<br>")} |')
    prior = text(directory / 'README.md') if (directory / 'README.md').exists() else ''
    notes = prior.split(NOTES, 1)[1] if NOTES in prior else ''
    return '\n'.join(lines) + '\n\n' + NOTES + notes


def toml_value(value):
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, int):
        return str(value)
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(item) for item in value) + ']'
    if isinstance(value, dict):
        return '{ ' + ', '.join(toml_value(key) + ' = ' + toml_value(item) for key, item in value.items()) + ' }'
    raise Invalid('R01: cannot encode TOML value')


def index(catalog):
    lines = ['# ' + LICENSE, '']
    for name, (data, _, _) in catalog.items():
        lines.append('[[profile]]')
        for key in ('name', 'title', 'kind', 'arch'):
            lines.append(key + ' = ' + toml_value(data['profile'][key]))
        lines.append('match = ' + toml_value(data['match']))
        lines.append('')
    return '\n'.join(lines)


def catalog_readme(catalog):
    lines = [f'<!-- {LICENSE} -->', '', '# Hardware profiles', '', 'Format and bounds: `python3 scripts/profile.py schema`. Check: `python3 scripts/profile.py check --drivers DIR --checks FILE`. Driver start files are handwritten and checked against the device table.', '', 'Generated catalog: [index.toml](index.toml). Regenerate with `python3 scripts/profile.py index` and `python3 scripts/profile.py readme`.', '', '| Profile | Kind | Architectures |', '|---|---|---|']
    for name, (data, _, _) in catalog.items():
        profile = data['profile']
        lines.append(f'| [{md(profile["title"])}]({name}/README.md) | {profile["kind"]} | {", ".join(profile["arch"])} |')
    return '\n'.join(lines) + '\n'


def generated(root, catalog):
    result = {root / 'profiles/index.toml': index(catalog), root / 'profiles/README.md': catalog_readme(catalog)}
    for data, rows, directory in catalog.values():
        result[directory / 'README.md'] = readme(data, rows, directory)
    return result


def import_evidence(args, root, catalog):
    from profile_digest import blake3

    require(args.profile in catalog, 10, 'unknown evidence profile')
    data, old, directory = catalog[args.profile]
    paths = bounded(args.records, MAX_RUNS, 'input records', 10)
    additions = []
    if args.legacy:
        for name in paths:
            row = json.loads(text(Path(name)))
            tree_bound(row, 10)
            require(type(row) is dict and row.get('record') == 'none', 10, 'legacy import needs a curated row with record none')
            additions.append(row)
    else:
        required(vars(args), ['ferrion', 'hardware', 'date', 'config'], 'import metadata', 10)
        require(all((args.ferrion, args.hardware, args.date, args.config)), 10, 'import requires --ferrion --hardware --date --config')
        groups = {}
        total_bytes = 0
        for name in paths:
            raw = text(Path(name))
            total_bytes += len(raw.encode()) + 8
            require(total_bytes <= MAX_BYTES, 10, 'record set bytes exceed limit')
            record = json.loads(raw)
            tree_bound(record, 10)
            require(record.get('verdict') in ('PASS', 'FAIL', 'BLOCKED'), 10, 'invalid input verdict')
            check = record.get('test')
            make = record.get('make', [])
            require(isinstance(check, str) and isinstance(make, list) and all(isinstance(item, str) for item in make), 10, 'invalid input check or make')
            version = record.get('host', {}).get('qemu', '')
            require(isinstance(version, str), 10, 'missing QEMU version')
            match = re.search(r'(?:version\s+)?([0-9]+\.[0-9]+(?:\.[0-9]+)?)', version)
            require(match is not None, 10, 'missing QEMU version')
            key = (check, tuple(make), match[1], record['verdict'] == 'BLOCKED')
            groups.setdefault(key, []).append((raw.encode(), record['verdict']))
        for (check, make, qemu, blocked), records in groups.items():
            passes = sum(verdict == 'PASS' for _, verdict in records)
            digest_input = b''.join(len(raw).to_bytes(8, 'little') + raw for raw, _ in records)
            bounded(digest_input, MAX_BYTES, 'record set bytes', 10)
            row = {'check': check, 'verdict': 'blocked' if blocked else 'pass' if passes == len(records) else 'flaky' if passes else 'fail', 'passes': passes, 'runs': len(records), 'date': args.date, 'ferrion': args.ferrion, 'hardware': args.hardware, 'config': args.config, 'make': list(make), 'conditions': args.condition, 'qemu': qemu, 'record': blake3(digest_input)}
            if check in ('gl-exact', 'vk-exact'):
                row['renderer'] = 'host GPU'
            additions.append(row)
    bounded(old + additions, MAX_RUNS, 'evidence rows', 10)
    content = '# ' + LICENSE + '\n'
    for row in old + additions:
        content += '\n[[run]]\n' + ''.join(key + ' = ' + toml_value(value) + '\n' for key, value in row.items())
    bounded(content.encode(), MAX_BYTES, 'evidence file bytes', 10)
    target = directory / 'evidence.toml'
    temporary = directory / '.agent-profile-evidence.toml'
    require(not temporary.exists(), 10, 'temporary evidence file already exists')
    try:
        temporary.write_text(content)
        evidence(temporary, data)
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)
    print(f'Imported {len(additions)} run sets into {args.profile}')


def main():
    parser = argparse.ArgumentParser(description='FerrionOS public hardware profiles')
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('check', 'readme', 'index', 'match', 'schema', 'evidence'):
        command = commands.add_parser(name)
        command.add_argument('--root', type=Path, default=Path(__file__).resolve().parent.parent)
        if name == 'check':
            command.add_argument('--drivers', type=Path)
            command.add_argument('--checks', type=Path, help='check names, one per line, or vm-test.py (parsed without executing it)')
        elif name == 'match':
            command.add_argument('--machine', type=Path, required=True)
        elif name == 'evidence':
            command.add_argument('operation', choices=['import'])
            command.add_argument('--profile', required=True)
            command.add_argument('--legacy', action='store_true', help='import curated pre-tooling rows with record none')
            for option in ('ferrion', 'hardware', 'date', 'config'):
                command.add_argument('--' + option)
            command.add_argument('--condition', action='append', default=[])
            command.add_argument('records', nargs='+')
    args = parser.parse_args()
    root = args.root.resolve()
    if args.command == 'schema':
        print(json.dumps({
            'keys': SCHEMA, 'required_run': sorted(REQUIRED_RUN),
            'features': FEATURES, 'laptop_features': LAPTOP_FEATURES,
            'kinds': KINDS, 'architectures': ARCHES, 'dmi': DMI,
            'file_prefixes': FILE_PREFIXES,
            'limits': {
                'bytes': MAX_BYTES, 'include_depth': MAX_DEPTH,
                'included_files': MAX_INCLUDES, 'profiles': MAX_PROFILES,
                'matches': MAX_MATCHES, 'devices': MAX_DEVICES,
                'features': MAX_FEATURES, 'pci': MAX_PCI,
                'identification_bytes': MAX_IDENT,
                'rows': MAX_RUNS, 'items': MAX_ITEMS,
                'nodes': MAX_NODES, 'tree_depth': MAX_TREE_DEPTH,
                'tree_files': MAX_TREE_FILES,
            },
            'composition': 'Includes merge in order, own file last; tables key by key; files append; devices/features replace by id; matches never inherited.',
            'routes': ['native', 'host', 'firmware', 'none'],
            'stages': ['initfs', 'rootfs'],
            'buses': ['pci', 'acpi', 'usb', 'vmbus', 'mmio', 'platform'],
            'boot': ['uefi', 'bios', 'u-boot', 'none'],
            'iommu': ['none', 'vt-d', 'amd-vi', 'virtio', 'smmu'],
            'packages': 'Empty rules only; users and groups are forbidden.',
            'files': 'Regular inline data files only. No setuid, setgid or world-write bit. Start files are handwritten.',
            'matching': 'DMI keys score 100 each, required PCI IDs 10 each, hypervisor 1. Equal-score profiles must have equal image effects. Cloud asset tags accept only the published Azure constant.',
            'pci': 'Four hex vendor digits, colon, four hex device digits or *; class is six hex digits. Wildcard driver claims are checked over all 65536 products.',
            'evidence': 'Latest date then append order wins per check. Import hashes the ordered concatenation of each record UTF-8 bytes prefixed by its u64 little-endian byte length. Legacy unknown revisions and QEMU versions are explicit, not inferred. Renderer is host GPU only.',
            'unsupported': 'Boot settings and reserved IOMMU regions await HP6 and IOMMU slice 7.',
        }, indent=2))
        return
    catalog = load_catalog(root, getattr(args, 'drivers', None), getattr(args, 'checks', None))
    if args.command == 'evidence':
        import_evidence(args, root, catalog)
    elif args.command == 'match':
        machine = parse(args.machine)
        shape(machine, 'machine', 4)
        required(machine, ['arch'], 'machine', 4)
        require(machine['arch'] in ARCHES, 4, 'unknown machine architecture')
        for key, value in machine.get('dmi', {}).items():
            require(key in DMI or key == 'chassis_asset_tag', 4, 'forbidden machine DMI key')
            require(isinstance(value, str), 4, 'machine DMI must be text')
            identifier(value)
        identifier(machine.get('hypervisor', ''))
        for pci in bounded(machine.get('pci', []), MAX_PCI, 'PCI functions'):
            require(PCI_ID.fullmatch(pci), 4, 'invalid machine PCI ID')
        ranks = [(score(data, machine), name) for name, (data, _, _) in catalog.items()]
        print(json.dumps([{'name': name, 'score': rank} for rank, name in sorted(ranks, key=lambda item: (-item[0], item[1])) if rank >= 0], indent=2))
    else:
        for path, content in generated(root, catalog).items():
            if args.command == 'check':
                require(path.exists() and text(path) == content, 11, f'generated file differs: {path.relative_to(root)}')
            elif args.command == 'readme' and path.name == 'README.md' or args.command == 'index' and path.name == 'index.toml':
                path.write_text(content)
        if args.command == 'check':
            for name, (data, rows, _) in catalog.items():
                devices = {device['id']: device for device in data['device']}
                states = [cell(feature, devices, latest_runs(rows))[0] for feature in data['feature']]
                shown = sum(state not in ('not shown', 'no driver') for state in states)
                print(f'{name}: {shown} evidence rows; {states.count("not shown")} not shown; {states.count("no driver")} no driver')
            print(f'PASS: {len(catalog)} profiles; 12 rules; drivers {"checked" if args.drivers else "not supplied"}; checks {"checked" if args.checks else "not supplied"}')


if __name__ == '__main__':
    try:
        main()
    except (Invalid, OSError, ValueError, TypeError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
