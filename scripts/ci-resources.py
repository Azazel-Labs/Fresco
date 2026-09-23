"""Choose Cargo concurrency for GitHub-hosted Ubuntu VMs."""

import argparse
import os
from pathlib import Path
import subprocess

GIB = 1024**3
# Conservative starting budgets, not measured per-crate peak guarantees.
MEMORY_PER_BUILD_JOB = 3 * GIB
MEMORY_RESERVE = 2 * GIB


def build_jobs(cpus, available_memory):
    if cpus < 1 or available_memory < 0:
        raise ValueError("CPU count must be positive and memory nonnegative")
    usable_memory = max(0, available_memory - MEMORY_RESERVE)
    return max(2, min(cpus, usable_memory // MEMORY_PER_BUILD_JOB))


def available_memory(meminfo):
    for line in meminfo.splitlines():
        fields = line.split()
        if fields and fields[0] == "MemAvailable:":
            if len(fields) != 3 or fields[2] != "kB":
                raise ValueError("Unexpected MemAvailable format")
            return int(fields[1]) * 1024
    raise ValueError("MemAvailable is missing from /proc/meminfo")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-threads", action="store_true")
    args = parser.parse_args()
    cpus = int(subprocess.check_output(["nproc"], text=True).strip())
    memory = available_memory(Path("/proc/meminfo").read_text())
    jobs = build_jobs(cpus, memory)
    with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
        output.write(f"CARGO_BUILD_JOBS={jobs}\n")
        if args.test_threads:
            output.write(f"RUST_TEST_THREADS={cpus}\n")
    print(
        f"Cargo: {jobs} build jobs; {cpus} available CPUs; "
        f"{memory / GIB:.2f} GiB available RAM; "
        "2 GiB reserved; 3 GiB budget per build job; minimum 2 jobs"
    )
    if args.test_threads:
        print(f"Native tests: {cpus} workers")


if __name__ == "__main__":
    main()
