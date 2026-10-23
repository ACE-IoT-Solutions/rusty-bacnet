#!/usr/bin/env python3
"""Capture already-built immutable A/B artifacts for an opt-in new measurement matrix."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify_tree(label, root, expected):
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    if head != expected['commit']:
        raise ValueError(f'{label}: checkout must be at {expected["commit"]}')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=root, text=True):
        raise ValueError(f'{label}: source checkout is not clean')
    for relative, digest in expected['source_hashes'].items():
        if sha(root / relative) != digest:
            raise ValueError(f'{label}: source drift in {relative}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('label', choices=['A', 'B'])
    parser.add_argument('root', type=Path)
    parser.add_argument('cargo_json', type=Path)
    parser.add_argument('native_target', type=Path)
    parser.add_argument('linux_volume')
    parser.add_argument('output', type=Path)
    parser.add_argument('--image', default='rb-batch10-callgrind:rust1.99.0')
    args = parser.parse_args()
    original = json.loads((HERE / 'inputs.json').read_text())
    root = args.root.resolve()
    verify_tree(args.label, root, original['variants'][args.label])
    artifacts = [json.loads(line) for line in args.cargo_json.read_text().splitlines() if line.startswith('{')]
    if not any(a.get('reason') == 'build-finished' and a.get('success') is True for a in artifacts):
        raise ValueError('Cargo JSON lacks a successful completed build')
    executables = [a['executable'] for a in artifacts if a.get('reason') == 'compiler-artifact'
                   and a.get('target', {}).get('name') == 'bip_latency' and a.get('executable')]
    if len(executables) != 1:
        raise ValueError('expected exactly one bip_latency executable')
    binary = Path(executables[0]).resolve()
    target = args.native_target.resolve()
    if not binary.is_relative_to(target):
        raise ValueError('executable is outside the private native target')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / 'outputs').mkdir(exist_ok=True)
    inputs_path = output / 'inputs.json'
    if inputs_path.exists():
        inputs = json.loads(inputs_path.read_text())
    else:
        inputs = {k: v for k, v in original.items() if k not in ['variants', 'native_environment', 'linux_environment']}
        inputs['variants'] = {}
        inputs['reproduction_of_original_inputs_sha256'] = sha(HERE / 'inputs.json')
        inputs['image'] = args.image
        inputs['image_id'] = subprocess.check_output(['docker', 'image', 'inspect', args.image, '--format', '{{.Id}}'], text=True).strip()
        inputs['native_environment'] = {'platform': platform.platform(),
            'rustc': subprocess.check_output(['rustc', '-Vv'], cwd=root, text=True),
            'cargo': subprocess.check_output(['cargo', '-V'], cwd=root, text=True), 'workers': 2,
            'CARGO_INCREMENTAL': '0'}
        inputs['linux_environment'] = subprocess.check_output(['docker', 'run', '--rm', args.image, 'sh', '-c',
            'rustc -Vv && cargo -V && valgrind --version'], text=True)
        if not inputs['native_environment']['rustc'].startswith('rustc 1.99.0 ') or not inputs['linux_environment'].startswith('rustc 1.99.0 ') or 'valgrind-3.22.0' not in inputs['linux_environment']:
            raise ValueError('wrong compiler or Callgrind version')
    if args.label in inputs['variants']:
        raise ValueError('label already captured; use a fresh output directory')
    if inputs['image'] != args.image:
        raise ValueError('A and B must use the same image')
    for relative, expected in original['common'].items():
        source = HERE / 'common' / relative
        destination = output / relative
        if sha(source) != expected:
            raise ValueError('distributed common input changed')
        if destination.exists():
            if sha(destination) != expected:
                raise ValueError('existing common input differs')
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    linux_hash = subprocess.check_output(['docker', 'run', '--rm', '-v', args.linux_volume + ':/target:ro',
        args.image, 'sha256sum', '/target/release/rp-harness'], text=True).split()[0]
    record = dict(original['variants'][args.label])
    record.update(root=str(root), native_binary=str(binary), native_binary_sha256=sha(binary),
                  native_target=str(target), volume=args.linux_volume, linux_binary_sha256=linux_hash,
                  cargo_json_sha256=sha(args.cargo_json))
    inputs['variants'][args.label] = record
    inputs['measurement_scripts'] = {name: sha(HERE / name) for name in ['callgrind_matrix.py', 'native_matrix.py', 'analyze.py']}
    inputs_path.write_text(json.dumps(inputs, indent=2) + '\n')
    print('Captured', args.label, record['native_binary_sha256'], linux_hash)


if __name__ == '__main__':
    main()
