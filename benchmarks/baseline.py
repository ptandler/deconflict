"""Generate synthetic datasets and measure the Python scan and analysis pipeline."""

from __future__ import annotations

import argparse
import json
import os
import resource
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from deconflict.analyze import analyze_group
from deconflict.patterns import build_patterns
from deconflict.scan import scan

ROOT = Path(__file__).resolve().parents[1]


def generate(root: Path, dataset: str) -> tuple[int, int, int]:
    if dataset == "small":
        for i in range(2000):
            directory = root / f"dir-{i % 20:02d}"
            directory.mkdir(exist_ok=True)
            (directory / f"ordinary-{i:04d}.txt").write_bytes(f"ordinary {i}\n".encode())
        for i in range(300):
            directory = root / f"dir-{i % 20:02d}"
            (directory / f"file-{i:04d}.txt").write_bytes(f"base {i}\n".encode())
            (directory / f"file-{i:04d} (conflicted copy 2026-01-01).txt").write_bytes(
                f"copy {i}\n".encode()
            )
        return 2600, 300, 600
    root.mkdir(exist_ok=True)
    block = bytes(range(256)) * 4096
    for i in range(4):
        for name in (f"file-{i}.bin", f"file-{i}.bin.pacnew"):
            with (root / name).open("wb") as stream:
                for _ in range(4):
                    stream.write(block)
    return 8, 4, 8


def io_counters() -> dict[str, int]:
    try:
        return {
            key: int(value)
            for line in Path("/proc/self/io").read_text().splitlines()
            for key, value in [line.split(": ")]
        }
    except (OSError, ValueError):
        return {}


def measure(root: Path, expected: tuple[int, int, int]) -> None:
    patterns = build_patterns()
    before_io = io_counters()
    before = resource.getrusage(resource.RUSAGE_SELF)
    start = time.perf_counter()
    result = scan([root], patterns)
    scan_end = time.perf_counter()
    analyses = [analyze_group(group, group.base, group.conflicts) for group in result.groups]
    end = time.perf_counter()
    after = resource.getrusage(resource.RUSAGE_SELF)
    after_io = io_counters()
    files = sum(1 for path in root.rglob("*") if path.is_file())
    analyzed = sum((analysis.base is not None) + len(analysis.copies) for analysis in analyses)
    assert (files, len(result.groups), analyzed) == expected
    print(
        json.dumps(
            {
                "scan_seconds": scan_end - start,
                "analysis_seconds": end - scan_end,
                "total_seconds": end - start,
                "cpu_seconds": after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime,
                "peak_rss_kib": after.ru_maxrss,
                "io_bytes": {
                    key: after_io[key] - before_io[key] for key in before_io if key in after_io
                },
                "files": files,
                "groups": len(result.groups),
                "analyzed_files": analyzed,
            }
        )
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--child", nargs=4, metavar=("ROOT", "FILES", "GROUPS", "ANALYZED"))
    args = parser.parse_args()
    if args.child:
        root, *counts = args.child
        measure(Path(root), tuple(map(int, counts)))
        return
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    print(json.dumps({"python": sys.version, "platform": sys.platform, "pid": os.getpid()}))
    with tempfile.TemporaryDirectory(prefix="benchmark-data-", dir=ROOT) as temp:
        for dataset in ("small", "large"):
            root = Path(temp) / dataset
            root.mkdir()
            expected = generate(root, dataset)
            for repetition in range(args.repeats):
                output = subprocess.check_output(
                    [sys.executable, __file__, "--child", str(root), *map(str, expected)],
                    text=True,
                )
                print(json.dumps({"dataset": dataset, "run": repetition + 1, **json.loads(output)}))


if __name__ == "__main__":
    main()
