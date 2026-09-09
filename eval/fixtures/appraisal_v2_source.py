#!/usr/bin/env python3
"""Harder appraisal tasks; reuse the baseline's fail-closed artifact oracle."""
import json
from pathlib import Path
import appraisal_source as oracle

oracle.ROOT = Path(__file__).resolve().parent / 'appraisal-v2'
oracle.CASES = json.loads((oracle.ROOT / 'cases.json').read_text())

if __name__ == '__main__':
    oracle.main()
