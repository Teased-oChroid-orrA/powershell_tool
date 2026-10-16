#!/usr/bin/env python3
"""Reproducible local/release qualification. Optional GPU checks require a native adapter."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--gpu', action='store_true', help='run ignored native GPU rendering tests')
parser.add_argument('--cad', action='store_true', help='include OpenCASCADE STEP import/edit tests')
parser.add_argument('--jobs', type=int, default=4)
parser.add_argument('--output', default='qualification.json')
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
base = ['cargo', 'test', '--workspace', '--locked', '-j', str(args.jobs), '--no-fail-fast']
cad_feature = 'app-tui/occt-source' if sys.platform == 'win32' else 'app-tui/occt'
if args.cad:
    base += ['--features', cad_feature]
commands = [base, ['cargo', 'check', '-p', 'app-tui', '--no-default-features', '--bins', '--lib', '--tests', '--locked', '-j', str(args.jobs)]]
if args.gpu:
    gpu = ['cargo', 'test', '-p', 'app-tui', '--lib', '--locked', '-j', str(args.jobs)]
    if args.cad:
        gpu += ['--features', cad_feature]
    commands.append(gpu + ['--', '--ignored', 'gpu_viewer', '--skip', 'snapshot'])
def source_state():
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    status = subprocess.check_output(['git', 'status', '--porcelain'], cwd=root)
    diff = subprocess.check_output(['git', 'diff', '--no-ext-diff', '--no-textconv', '--binary', 'HEAD'], cwd=root)
    digest = hashlib.sha256(commit.encode() + b'\0' + status + b'\0' + diff).hexdigest()
    return commit, bool(status.strip()), digest

starting_commit, starting_dirty, starting_state = source_state()
report = {'commit': starting_commit, 'platform': sys.platform, 'dirty': starting_dirty, 'checks': []}
failed = False
for command in commands:
    print('Running:', ' '.join(command), flush=True)
    started = time.monotonic()
    result = subprocess.run(command, cwd=root)
    failed |= result.returncode != 0
    report['checks'].append({'command': command, 'exit_code': result.returncode, 'seconds': time.monotonic() - started})
ending_commit, ending_dirty, ending_state = source_state()
report.update(ending_commit=ending_commit, ending_dirty=ending_dirty, source_changed=starting_state != ending_state)
failed |= report['source_changed']
if report['source_changed']:
    print('Source changed during qualification; results are not a stable-revision qualification.', file=sys.stderr)
report['passed'] = not failed
pathlib.Path(args.output).write_text(json.dumps(report, indent=2) + '\n')
sys.exit(1 if failed else 0)
