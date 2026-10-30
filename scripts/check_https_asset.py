#!/usr/bin/env python3
"""Recheck raw native asset evidence; never trust the runner's result.json."""
import argparse,json
from pathlib import Path
from https_asset_gate import verify_evidence

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',required=True,type=Path);parser.add_argument('--binding',required=True,type=Path);parser.add_argument('--evidence',required=True,type=Path);args=parser.parse_args()
 result=verify_evidence(Path(__file__).resolve().parents[1],args.iso.resolve(strict=True),args.binding.resolve(strict=True),args.evidence.resolve(strict=True));print(json.dumps(result))

if __name__=='__main__':main()
