### Python baseline

Run from the repository root on Linux: `uv run python benchmarks/baseline.py --repeats 5`.
The script creates and removes a unique temporary dataset directory in the project root;
it never touches sample data. Each run is a fresh Python process, using default enabled
patterns and one serial worker. Output is JSON Lines, including every individual run.

Datasets are deterministic, with no random seed required:

- `small`: 20 directories, 2,000 ordinary text files and 300 Nextcloud pairs of tiny,
  distinct text files (2,600 files total, 300 groups, 600 files analyzed).
- `large`: four pacman pairs of identical binary files, each 4 MiB (8 files, 4 groups,
  32 MiB analyzed). Each file repeats the bytes 0 through 255.

The timed region calls `scan()` and `analyze_group()` for every group, including hashing,
sniffing and metadata collection, but excludes dataset generation, Python startup, CLI,
engine cache, rendering, and cleanup. Scan and analysis times, process CPU time, peak
RSS and Linux `/proc/self/io` deltas are recorded. RSS is the process lifetime high-water
mark, including imports; Linux reports it in KiB. I/O `rchar` counts bytes returned by
read-like calls, while `read_bytes` counts storage reads, not logical bytes. Counts are
validated after timing. This is not a full end-to-end application benchmark.

### Results (2026-11-24)

Python 3.13.15, Linux 6.12 x86-64, local NVMe, Btrfs; five runs per dataset.
Datasets were just written and retained in the OS page cache; no cache eviction was
attempted. CPU and storage contention were not controlled. Values are medians, with
total-time range in parentheses:

| Dataset | Scan | Analysis | Total | CPU | Peak RSS | Throughput | Logical read / storage read |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| small | 0.152 s | 0.089 s | 0.229 s (0.217–0.363) | 0.228 s | 28.9 MiB | 11,367 discovered files/s | 10,458 B / 0 B |
| large | 0.001 s | 0.266 s | 0.267 s (0.232–0.319) | 0.266 s | 25.3 MiB | 120 MiB analyzed/s | 33,620,066 B / 0 B |

Throughput uses the median total time: discovered files for `small`, nominal analyzed
bytes for `large`. Neither throughput number isolates hashing or disk speed. Zero
storage-read bytes indicate these particular runs did not measure cold-disk I/O. For
future comparisons, use the same machine, filesystem, datasets and cache conditions;
cold-cache and network-storage results require separate controlled experiments.

### Rust benchmark

On Linux, run `uv run python benchmarks/rust.py --repeats 5 --workers 1 4`.
It builds the public-API example in release mode using the locked Cargo dependencies,
then uses `baseline.py`'s generators and expected counts. Each measurement starts a
fresh Rust process; build, startup, pattern construction, generation, count validation,
and cleanup are outside wall-clock timing. JSON Lines report each run, worker count,
scan, subsequent group analysis and total times, CPU, peak RSS and Linux I/O deltas.
The runner validates discovered dataset files, final groups, analyzed files and
successful provisional events, and fails on scan or analysis errors. It does not
validate hashes or semantic equivalence. `max_in_flight` stays at its default 64.

Rust's scan includes provisional SHA-256 and text sniffing of each conflict copy;
afterward the harness calls `analyze_group` serially for every final group, hashing
both base and copies again. Thus 4 workers parallelize scan, not final analysis.
Python scans without hashing and analyzes each file once. Both perform hashing,
text sniffing, and group equality checks, but Rust does not collect media metadata,
and its snapshot validation and implementation differ. Neither benchmark calculates
recommendations. These totals measure related, **not identical**, workloads; individual
scan and analysis columns are especially not directly comparable.

Rust RSS is `/proc/self/status` `VmHWM`, including startup; Python RSS is
`getrusage` high-water RSS. Rust CPU uses child process `getrusage`, including startup,
whereas Python CPU covers the timed region. The operating system and runtime may affect
both memory measures; neither isolates the timed region. Linux `/proc` is required.

### Comparison (2026-11-24)

Same machine and conditions as above; Rust 1.96.0 release build, Python 3.13.15.
Five runs per setting, Rust settings first followed by a fresh Python baseline;
datasets were generated separately but identically. Warm page cache, uncontrolled
contention and fixed run order limit conclusions. Medians (total range in parentheses):

| Dataset | Implementation | Scan | Analysis | Total | CPU | Peak RSS | Logical / storage read |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| small | Python, serial | 0.153 s | 0.080 s | 0.228 s (0.216–0.368) | 0.221 s | 28.7 MiB | 10,458 B / 0 B |
| small | Rust, 1 worker | 0.220 s | 0.018 s | 0.238 s (0.145–0.493) | 0.203 s | 5.5 MiB | 7,864 B / 0 B |
| small | Rust, 4 workers | 0.066 s | 0.012 s | 0.077 s (0.060–0.228) | 0.157 s | 5.6 MiB | 7,864 B / 0 B |
| large | Python, serial | 0.001 s | 0.277 s | 0.278 s (0.221–0.310) | 0.275 s | 25.3 MiB | 33,620,066 B / 0 B |
| large | Rust, 1 worker | 0.317 s | 0.606 s | 0.884 s (0.860–1.033) | 0.884 s | 4.9 MiB | 50,331,742 B / 0 B |
| large | Rust, 4 workers | 0.140 s | 0.728 s | 0.855 s (0.507–1.320) | 1.142 s | 5.0 MiB | 50,331,742 B / 0 B |

Median columns are calculated independently and need not sum to the median total.
Rust reads about 48 MiB for the large dataset's 32 MiB of unique files because of
provisional rehashing; Python also reads extra sniff/metadata bytes. Zero storage
reads indicate warm-cache runs, not disk throughput. Variability and workload
differences preclude general speedup claims.

### Scheduler follow-up (2026-11-24)

After removing the pending-analysis limit, three warm-cache Rust runs per setting on
the same machine gave median totals of 0.095 s and 0.050 s for `small`, and 0.563 s
and 0.419 s for `large`, with one and four workers respectively. Median CPU times
were 0.094 s, 0.103 s, 0.565 s and 0.599 s; peak RSS medians were 5.4, 5.5, 4.9
and 4.9 MiB. Storage reads were zero. These uncontrolled runs are not a controlled
before/after comparison and retain the workload differences described above.