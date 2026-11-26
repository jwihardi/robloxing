#!/usr/bin/env python3
"""Validate review evidence bookkeeping. Does not evaluate images or execute game tests."""
import argparse
import json
from pathlib import Path

CHECKS = ('identity', 'hierarchy', 'craft', 'copy', 'state-truth', 'interaction',
          'scaling', 'accessibility', 'stress', 'performance')
VISUAL = {'identity', 'hierarchy', 'craft', 'copy', 'scaling'}
SCOPES = {'patch', 'new-screen', 'redesign', 'critique', 'specification'}
IMAGE = {'.png', '.jpg', '.jpeg', '.webp', '.bmp', '.tga'}


def inspect(path):
    malformed, incomplete = [], []
    try:
        record = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as exc:
        return 2, [str(exc)]
    if not isinstance(record, dict):
        return 2, ['Record must be a JSON object.']
    root = path.parent
    def nonempty(value):
        return isinstance(value, str) and bool(value.strip())
    def files(value, label):
        if not isinstance(value, list) or not all(nonempty(x) for x in value):
            malformed.append(f'{label}: expected a list of nonempty file paths')
            return []
        found = []
        for item in value:
            p = root / item
            try:
                if not p.is_file() or p.stat().st_size == 0:
                    incomplete.append(f'{label}: missing or empty file {item}')
                else:
                    found.append(p.resolve())
            except OSError as exc:
                incomplete.append(f'{label}: {exc}')
        return found
    scope = record.get('scope')
    if not isinstance(scope, str) or scope not in SCOPES:
        malformed.append('scope: expected one of ' + ', '.join(sorted(SCOPES)))
    build = isinstance(scope, str) and scope in {'patch', 'new-screen', 'redesign'}
    for key in ('target', 'style_contract', 'baseline_note', 'refinement_note'):
        if not nonempty(record.get(key)):
            incomplete.append(f'{key}: missing explanation')
    for key in ('acceptance_criteria', 'references'):
        value = record.get(key)
        if not isinstance(value, list) or not all(nonempty(x) for x in value):
            malformed.append(f'{key}: expected list of nonempty strings')
        elif not value:
            incomplete.append(f'{key}: provide criteria or source/availability notes')
    captures = files(record.get('captures'), 'captures')
    images = {p for p in captures if p.suffix.lower() in IMAGE}
    if build and not images:
        incomplete.append('implementation: no existing image capture listed')
    checks = record.get('checks')
    if not isinstance(checks, dict):
        malformed.append('checks: expected object')
        checks = {}
    for key in CHECKS:
        check = checks.get(key)
        if not isinstance(check, dict):
            malformed.append(f'{key}: missing check object')
            continue
        status = check.get('status')
        if not isinstance(status, str) or status not in {'pass', 'fail', 'untested', 'na'}:
            malformed.append(f'{key}: invalid status')
            continue
        if not nonempty(check.get('note')):
            incomplete.append(f'{key}: missing observation or reason')
        evidence = files(check.get('evidence'), f'{key} evidence')
        if status in {'fail', 'untested'}:
            incomplete.append(f'{key}: {status}')
        if build and key in {'identity', 'hierarchy', 'craft', 'copy'} and status == 'na':
            incomplete.append(f'{key}: core visual check cannot be excluded for implementation')
        if build and key in VISUAL and status == 'pass' and not images.intersection(evidence):
            incomplete.append(f'{key}: passing visual check must cite a listed capture')
    return (2 if malformed else 1 if incomplete else 0), malformed + incomplete


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('review', type=Path)
    args = parser.parse_args()
    code, issues = inspect(args.review)
    if code:
        print('REVIEW INVALID' if code == 2 else 'REVIEW INCOMPLETE')
        for issue in issues:
            print('- ' + issue)
    else:
        print('RECORD PASS: evidence bookkeeping is complete. Visual quality and runtime behavior are not independently verified.')
    return code


if __name__ == '__main__':
    raise SystemExit(main())
