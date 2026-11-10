#!/usr/bin/env python3
"""Generate the request JSON from the Markdown, or check it with --check."""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MARKDOWN = ROOT / '.scratch/testing/recorded-streams.md'
OUTPUT = ROOT / 'viewer/Requests/recorded-streams.json'


def requests_from_markdown():
    rows = []
    for line in MARKDOWN.read_text().splitlines():
        if not re.match(r'^\|\s*\d+\s*\|', line):
            continue
        cells = [cell.strip() for cell in line.strip('|').split('|')]
        if len(cells) not in (3, 4):
            raise ValueError(f'invalid request row: {line}')
        rows.append(dict(number=int(cells[0]), model=cells[1].lower(),
                         request=cells[-1], of=int(cells[2]) if len(cells) == 4 else None))
    assert [row['number'] for row in rows] == list(range(1, 151)), 'expected requests 1–150 in order'
    assert sum(row['of'] is None for row in rows) == 120, 'expected 120 fresh screens'
    assert len({row['request'] for row in rows}) == 150, 'duplicate request'
    for row in rows:
        assert row['model'] == ('sonnet' if row['number'] % 2 else 'haiku'), 'models must alternate'
        if row['of'] is not None:
            assert 0 < row['of'] < row['number'], 'parent must precede follow-up'
            parent = rows[row['of'] - 1]
            assert parent['of'] is None and parent['model'] == row['model'], 'invalid follow-up parent/model'
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    rows = requests_from_markdown()
    if args.check:
        assert json.loads(OUTPUT.read_text()) == rows, 'request JSON differs from Markdown; run viewer/Requests/sync-recorded-streams.py'
        print('Recorded requests: all 150 Markdown and JSON rows agree')
    else:
        OUTPUT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + '\n')
