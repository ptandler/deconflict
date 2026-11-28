"""Run the Rust benchmark on the Python baseline's deterministic datasets."""

from __future__ import annotations

import argparse
import json
import platform
import resource
import subprocess
import sys
import tempfile
from pathlib import Path

from baseline import ROOT, generate

EXAMPLE = ROOT / "rust" / "target" / "release" / "examples" / "benchmark"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 4])
    parser.add_argument(
        "--child", nargs=5, metavar=("ROOT", "WORKERS", "FILES", "GROUPS", "ANALYZED")
    )
    args = parser.parse_args()
    if args.child:
        root, workers, files, groups, analyzed = args.child
        output = subprocess.check_output([str(EXAMPLE), root, workers], text=True)
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        result = json.loads(output)
        actual_files = sum(1 for path in Path(root).rglob("*") if path.is_file())
        if (
            actual_files,
            result["groups"],
            result["analyzed_files"],
            result["provisional_files"],
        ) != (int(files), int(groups), int(analyzed), int(groups)):
            raise ValueError(f"unexpected counts: {actual_files}, {result}")
        result.update(files=actual_files, cpu_seconds=usage.ru_utime + usage.ru_stime)
        print(json.dumps(result))
        return
    if args.repeats < 1 or any(workers < 1 for workers in args.workers):
        parser.error("repeats and workers must be positive")
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "--manifest-path",
            str(ROOT / "rust/Cargo.toml"),
            "--example",
            "benchmark",
        ],
        check=True,
    )
    print(
        json.dumps(
            {
                "platform": platform.platform(),
                "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            }
        ),
        flush=True,
    )
    with tempfile.TemporaryDirectory(prefix="benchmark-data-", dir=ROOT) as temp:
        for dataset in ("small", "large"):
            root = Path(temp) / dataset
            root.mkdir()
            expected = generate(root, dataset)
            for workers in args.workers:
                for repetition in range(args.repeats):
                    output = subprocess.check_output(
                        [
                            sys.executable,
                            __file__,
                            "--child",
                            str(root),
                            str(workers),
                            *map(str, expected),
                        ],
                        text=True,
                    )
                    print(
                        json.dumps(
                            {
                                "dataset": dataset,
                                "workers": workers,
                                "run": repetition + 1,
                                **json.loads(output),
                            }
                        ),
                        flush=True,
                    )


if __name__ == "__main__":
    main()
