"""``syncflowy-conformance``: record goldens against production, check the mock.

Subcommands (scenario selectors are name globs, or ``@area``/``@tag``)::

    syncflowy-conformance list   [SEL...]          # scenarios + golden status
    syncflowy-conformance record [SEL...]          # run on PRODUCTION, write goldens
    syncflowy-conformance check  [SEL...]          # run on an in-process mock, diff
    syncflowy-conformance run    [SEL...] --backend mock|prod   # print transcript only
    syncflowy-conformance show   NAME              # print a golden's normalised transcript
    syncflowy-conformance renormalize [SEL...]     # refresh goldens' "normalized" from "raw"

See docs/conformance.md.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time

from .normalize import Clock, normalize, render
from .registry import select_scenarios
from .runner import (
    MOCK_API_KEY,
    HarnessError,
    MockPool,
    check_scenario,
    golden_path,
    has_errors,
    read_golden,
    run_scenario,
    write_golden,
)


def _cmd_list(args) -> int:
    for sc in select_scenarios(args.select):
        g = "golden" if golden_path(sc.name).exists() else "  -   "
        first = sc.doc.splitlines()[0] if sc.doc else ""
        opts = f" {sc.mock_options}" if sc.mock_options else ""
        print(f"{g}  {sc.area:10s} {sc.name:45s} {first}{opts}")
    return 0


def _cmd_record(args) -> int:
    from .backend import ProdBackend

    scs = select_scenarios(args.select)
    if not scs:
        print("no scenarios selected", file=sys.stderr)
        return 2
    if args.dry_run:
        return _record_dry_run(scs)
    if not args.yes and len(scs) > 3:
        print(
            f"About to run {len(scs)} scenarios against PRODUCTION. Re-run with --yes to confirm.",
            file=sys.stderr,
        )
        return 2
    backend = ProdBackend(keep_sandboxes=args.keep_sandboxes)
    failures = 0
    for sc in scs:
        if golden_path(sc.name).exists() and not args.force:
            print(f"skip   {sc.name} (golden exists; --force to re-record)")
            continue
        os.environ["SYNCFLOWY_TAG"] = f"conformance:{sc.name}"
        t = time.time()
        try:
            raw = run_scenario(backend, sc)
        except HarnessError as e:
            print(f"REFUSE {sc.name}: {e}")
            failures += 1
            continue
        errs = has_errors(raw)
        if errs and not args.allow_errors:
            print(f"ERROR  {sc.name}: scenario raised on production, not saved: {errs}")
            failures += 1
            continue
        clock = Clock(raw)
        flat = repr(normalize(raw))
        if (clock.overlaps or "<E?" in flat) and not args.allow_errors:
            print(
                f"ERROR  {sc.name}: timestamps not attributable to epochs "
                f"(overlapping epochs {clock.overlaps}, skew {clock.skew_interval}); not saved, re-run"
            )
            failures += 1
            continue
        p = write_golden(sc, raw)
        n = sum(1 for s in raw["steps"] if s.get("type") == "request")
        print(f"ok     {sc.name}: {n} requests, {time.time() - t:.1f}s -> {p}")
    backend.close()
    return 1 if failures else 0


def _record_dry_run(scs) -> int:
    """Run the selection on a vetting mock: proves production would accept
    (and the safety vetting would not refuse) every request, offline."""
    from syncflowy.mock.testing import running_mock

    from .backend import FakeClock, VettingMockBackend

    bad = 0
    clock = FakeClock()
    with running_mock(api_key=MOCK_API_KEY, clock=clock, faithful_cycles=True) as url:
        b = VettingMockBackend(url, MOCK_API_KEY, clock)
        for sc in scs:
            try:
                raw = run_scenario(b, sc)
            except HarnessError as e:
                print(f"REFUSE {sc.name}: {e}")
                bad += 1
                continue
            n = sum(1 for s in raw["steps"] if s.get("type") == "request")
            print(f"ok     {sc.name}: {n} requests")
    return 1 if bad else 0


def _cmd_check(args) -> int:
    scs = select_scenarios(args.select)
    counts = {"pass": 0, "fail": 0, "skip": 0, "error": 0}
    with MockPool(base_url=args.base_url, api_key=args.api_key) as pool:
        for sc in scs:
            r = check_scenario(sc, pool)
            counts[r.status] += 1
            if r.status == "pass":
                if args.verbose:
                    print(f"PASS  {sc.name}")
            elif r.status == "skip":
                if args.verbose:
                    print(f"SKIP  {sc.name}: {r.detail}")
            else:
                print(f"{r.status.upper():5s} {sc.name}")
                print(r.detail)
                print()
    print(
        f"{counts['pass']} passed, {counts['fail']} failed, {counts['error']} errors, "
        f"{counts['skip']} skipped"
    )
    return 1 if counts["fail"] or counts["error"] else 0


def _cmd_run(args) -> int:
    scs = select_scenarios(args.select)
    if args.backend == "prod":
        from .backend import ProdBackend

        if len(scs) > 3 and not args.yes:
            print("refusing to run >3 scenarios on production without --yes", file=sys.stderr)
            return 2
        backend = ProdBackend(keep_sandboxes=args.keep_sandboxes)
        for sc in scs:
            raw = run_scenario(backend, sc)
            _print(sc.name, raw, args.raw)
        backend.close()
        return 0
    with MockPool(base_url=args.base_url, api_key=args.api_key) as pool:
        for sc in scs:
            raw = run_scenario(pool.get(sc.mock_options), sc)
            _print(sc.name, raw, args.raw)
    return 0


def _print(name: str, raw: dict, as_raw: bool) -> None:
    print(f"=== {name}")
    if as_raw:
        print(json.dumps(raw, indent=1))
    else:
        print("\n".join(render(normalize(raw))))


def _cmd_show(args) -> int:
    g = read_golden(args.name)
    if g is None:
        print(f"no golden for {args.name}", file=sys.stderr)
        return 1
    print(g.get("doc", ""))
    print(f"(recorded {g.get('recorded_at')})\n")
    print("\n".join(render(normalize(g["raw"]))))
    return 0


def _cmd_renormalize(args) -> int:
    for sc in select_scenarios(args.select):
        g = read_golden(sc.name)
        if g is None:
            continue
        new = normalize(g["raw"])
        if new != g.get("normalized"):
            g["normalized"] = new
            golden_path(sc.name).write_text(json.dumps(g, indent=1) + "\n")
            print(f"updated {sc.name}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="syncflowy-conformance", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("list")
    p.add_argument("select", nargs="*")
    p.set_defaults(fn=_cmd_list)

    p = sub.add_parser("record", help="run scenarios against PRODUCTION and write goldens")
    p.add_argument("select", nargs="*")
    p.add_argument("--force", action="store_true", help="overwrite existing goldens")
    p.add_argument("--yes", action="store_true", help="confirm running more than 3 scenarios")
    p.add_argument("--keep-sandboxes", action="store_true", help="don't delete sandboxes afterwards")
    p.add_argument("--allow-errors", action="store_true", help="save goldens even if the scenario raised")
    p.add_argument("--dry-run", action="store_true",
                   help="don't touch production: run on a mock with production safety vetting")
    p.set_defaults(fn=_cmd_record)

    for name, fn in (("check", _cmd_check), ("run", _cmd_run)):
        p = sub.add_parser(name)
        p.add_argument("select", nargs="*")
        p.add_argument("--base-url", help="use an already-running mock (real sleeps) instead of running_mock")
        p.add_argument("--api-key", default=os.environ.get("SYNCFLOWY_MOCK_API_KEY", MOCK_API_KEY))
        if name == "check":
            p.add_argument("-v", "--verbose", action="store_true")
        else:
            p.add_argument("--backend", choices=("mock", "prod"), default="mock")
            p.add_argument("--raw", action="store_true", help="print the raw transcript")
            p.add_argument("--yes", action="store_true")
            p.add_argument("--keep-sandboxes", action="store_true")
        p.set_defaults(fn=fn)

    p = sub.add_parser("show")
    p.add_argument("name")
    p.set_defaults(fn=_cmd_show)

    p = sub.add_parser("renormalize")
    p.add_argument("select", nargs="*")
    p.set_defaults(fn=_cmd_renormalize)

    args = ap.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
