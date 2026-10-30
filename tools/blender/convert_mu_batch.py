#!/usr/bin/env python3
"""Run convert_mu.py over a job list with parallel headless Blender processes.

    python3 tools/blender/convert_mu_batch.py <jobs.json> <results.json> [--workers N] [--skip-existing]

Jobs are split into small chunks that N Blender processes work through. If Blender dies or hangs
on a job, that job is marked failed and the rest of its chunk runs in a fresh Blender. The merged
results are written in job order (also after every chunk). Blender logs go to <results>.logs/.
The exit status is 0 when the batch ran, even if some jobs failed: check "ok" in the results.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "convert_mu.py")
DEFAULT_BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")


def job_hash(job: dict) -> str:
    return hashlib.sha1(json.dumps(job, sort_keys=True).encode()).hexdigest()[:16]


def up_to_date(job: dict, prev: dict | None) -> bool:
    """Output exists, is newer than every source and (if known) was made from the same job."""
    out = job.get("output")
    if not out or not os.path.isfile(out):
        return False
    if prev is not None and (not prev.get("ok") or prev.get("job_hash", job_hash(job)) != job_hash(job)):
        return False
    sources = {m["path"] for m in job.get("models", [])}
    if prev is not None:
        sources.update(prev.get("sources") or [])
    mtime = os.path.getmtime(out)
    for path in sources:
        try:
            if os.path.getmtime(path) > mtime:
                return False
        except OSError:
            return False
    return True


def failure(job: dict, error: str) -> dict:
    return {"id": job.get("id"), "ok": False, "error": error, "output": job.get("output"), "bytes": 0,
            "triangles": 0, "bbox_min": None, "bbox_max": None, "transforms": {}, "textures": [],
            "warnings": []}


def read_json(path: str):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def write_json(path: str, data) -> None:
    tmp = f"{path}.tmp{os.getpid()}"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=1)
    os.replace(tmp, path)


def run_chunk(blender: str, chunk: list, logdir: str, tag: str, timeout: float) -> list:
    """Convert (index, job) pairs in one Blender, restarting after a crash. Returns (index, result)."""
    out = []
    pending = list(chunk)
    attempt = 0
    while pending:
        attempt += 1
        stem = os.path.join(logdir, f"{tag}.{attempt}")
        jobs_path, results_path, log_path = stem + ".jobs.json", stem + ".results.json", stem + ".log"
        write_json(jobs_path, [job for _, job in pending])
        if os.path.exists(results_path):
            os.remove(results_path)
        cmd = [blender, "-b", "--factory-startup", "--python-exit-code", "1",
               "--python", SCRIPT, "--", jobs_path, results_path]
        with open(log_path, "w") as log:
            try:
                status = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                                        timeout=timeout * len(pending) + 60).returncode
            except subprocess.TimeoutExpired:
                status = "timeout"
        got = read_json(results_path) or []
        got = got[: len(pending)]
        out.extend(zip([i for i, _ in pending], got))
        rest = pending[len(got):]
        if rest:
            # The first unfinished job took Blender down (or hung it).
            index, job = rest[0]
            out.append((index, failure(job, f"Blender exited ({status}) while converting this job; see {log_path}")))
            rest = rest[1:]
        else:
            for path in (jobs_path, results_path):
                os.remove(path)
        pending = rest
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("jobs")
    ap.add_argument("results")
    ap.add_argument("--workers", type=int, default=6)
    ap.add_argument("--skip-existing", action="store_true",
                    help="skip jobs whose .glb is newer than all its sources (keeps the previous results)")
    ap.add_argument("--chunk", type=int, default=0, help="jobs per Blender process (default: automatic)")
    ap.add_argument("--timeout", type=float, default=300.0, help="seconds allowed per job (default 300)")
    ap.add_argument("--blender", default=DEFAULT_BLENDER, help="Blender executable (or $BLENDER)")
    args = ap.parse_args()

    with open(args.jobs) as f:
        jobs = json.load(f)
    ids = [job.get("id") for job in jobs]
    if len(set(ids)) != len(ids):
        print("warning: duplicate job ids", file=sys.stderr)

    previous = {}
    if os.path.isfile(args.results):
        for res in read_json(args.results) or []:
            previous[res.get("id")] = res

    results: list = [None] * len(jobs)
    todo = []
    for index, job in enumerate(jobs):
        prev = previous.get(job.get("id"))
        if args.skip_existing and up_to_date(job, prev):
            if prev is None:
                prev = {"id": job.get("id"), "ok": True, "error": None, "output": job["output"],
                        "bytes": os.path.getsize(job["output"]),
                        "warnings": ["skipped: up to date, no previous result"]}
            results[index] = dict(prev, skipped=True)
        else:
            todo.append((index, job))

    # Biggest models first so the long ones don't end up last.
    def cost(item):
        return sum(os.path.getsize(m["path"]) for m in item[1].get("models", []) if os.path.isfile(m["path"]))

    todo.sort(key=cost, reverse=True)
    workers = max(1, args.workers)
    size = args.chunk or max(1, min(12, math.ceil(len(todo) / (workers * 4)))) if todo else 1
    chunks = [todo[i:i + size] for i in range(0, len(todo), size)]
    logdir = os.path.abspath(args.results) + ".logs"
    os.makedirs(logdir, exist_ok=True)
    print(f"{len(jobs)} jobs: {len(todo)} to convert in {len(chunks)} chunks on {workers} workers, "
          f"{len(jobs) - len(todo)} skipped", flush=True)

    lock = threading.Lock()
    t0 = time.time()
    done = 0

    def finish():
        merged = [r for r in results if r is not None]
        write_json(args.results, merged)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(run_chunk, args.blender, chunk, logdir, f"chunk{n:04d}", args.timeout)
                   for n, chunk in enumerate(chunks)]
        for fut in as_completed(futures):
            with lock:
                for index, res in fut.result():
                    res["job_hash"] = job_hash(jobs[index])
                    results[index] = res
                    done += 1
                    if not res.get("ok"):
                        print(f"  FAILED {res.get('id')}: {res.get('error')}", flush=True)
                finish()
                print(f"[{done}/{len(todo)}] {time.time() - t0:.0f} s", flush=True)
    finish()

    final = [r for r in results if r is not None]
    ok = [r for r in final if r.get("ok")]
    failed = [r for r in final if not r.get("ok")]
    skipped = [r for r in final if r.get("skipped")]
    total = sum(r.get("bytes") or 0 for r in ok)
    elapsed = time.time() - t0
    converted = [r for r in ok if not r.get("skipped")]
    print(f"ok {len(ok)} (skipped {len(skipped)}), failed {len(failed)}, "
          f"output {total / 1e6:.1f} MB, {elapsed:.0f} s"
          + (f", {sum(r.get('seconds', 0) for r in converted) / len(converted):.2f} s/job in Blender"
             if converted else ""))
    for r in failed[:20]:
        print(f"  {r.get('id')}: {r.get('error')}")
    if len(failed) > 20:
        print(f"  ... {len(failed) - 20} more")
    return 0


if __name__ == "__main__":
    sys.exit(main())
