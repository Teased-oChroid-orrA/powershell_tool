#!/usr/bin/env python3
"""Reproducible local/release qualification. Optional GPU checks require a native adapter."""
import argparse
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
if args.cad:
    base += ['--features', 'app-tui/occt-source' if sys.platform == 'win32' else 'app-tui/occt']
commands = [base, ['cargo', 'check', '-p', 'app-tui', '--no-default-features', '--bins', '--lib', '--tests', '--locked', '-j', str(args.jobs)]]
if args.gpu:
    commands.append(['cargo', 'test', '-p', 'app-tui', '--lib', '--locked', '-j', str(args.jobs), '--', '--ignored', 'gpu_viewer', '--skip', 'snapshot'])
report = {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(), 'platform': sys.platform, 'dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=root, text=True).strip()), 'checks': []}
failed = False
for command in commands:
    print('Running:', ' '.join(command), flush=True)
    started = time.monotonic()
    result = subprocess.run(command, cwd=root)
    failed |= result.returncode != 0
    report['checks'].append({'command': command, 'exit_code': result.returncode, 'seconds': time.monotonic() - started})
report['passed'] = not failed
pathlib.Path(args.output).write_text(json.dumps(report, indent=2) + '\n')
sys.exit(1 if failed else 0)
