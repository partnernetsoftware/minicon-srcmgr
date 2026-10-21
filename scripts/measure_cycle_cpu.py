"""Compare one disposable standard-fixture GUI cycle with its actual Git argv.

Run only when other GUI probes have finished. Owned-process command capture is
read-only and opt-in; it adds no tracing to Git or the product. Raw argv stays
under ignored target/. A missing command or counter blocks the comparison.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess as sp
import sys
import time

import probe_workspace as probe
from windows_job_probe import OwnedTree


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--git", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    root = Path(args.out).resolve()
    workspace_target = Path(__file__).resolve().parents[1] / "target"
    if not root.is_relative_to(workspace_target) or root.exists():
        parser.error("out must be a new directory beneath this repository's target/")
    root.mkdir(parents=True)
    # Pin the clock only in this process and its children so deterministic
    # replay has the same immutable commit IDs as the actual GUI cycle.
    os.environ["GIT_AUTHOR_DATE"] = "1700001234 +0000"
    os.environ["GIT_COMMITTER_DATE"] = "1700001234 +0000"
    captured = {}
    original_sustained, original_receipt, original_init = probe.sustained, OwnedTree.receipt, OwnedTree.__init__

    def initialize(tree):
        original_init(tree, capture_commands=True)
        captured["tree"] = tree

    def receipt(tree, sample):
        result = original_receipt(tree, sample)
        captured["job_receipt"] = result
        return result

    def sustained(git, repo, out, *positional):
        def read(*argv):
            return sp.run([git, "-C", str(repo), *argv], check=True, capture_output=True).stdout
        assert repo == root / "native" / "repo"
        captured["repo"] = repo
        captured["initial_head"] = read("rev-parse", "HEAD").strip().decode()
        captured["dirty"] = {path.name: path.read_bytes() for path in repo.glob("f*.txt")
                             if int(path.stem[1:]) < 200}
        tree = captured["tree"]
        before = tree.accounting()
        captured["begin"] = time.time_ns() // 1000
        result = original_sustained(git, repo, out, *positional)
        captured["end"] = time.time_ns() // 1000
        after = tree.accounting()
        captured["gui_cpu_s"] = after["cpu_s"] - before["cpu_s"]
        captured["final_head"] = read("rev-parse", "HEAD").strip().decode()
        captured["final_tree"] = read("write-tree").strip().decode()
        captured["final_status"] = read("status", "--porcelain=v2", "-z").hex()
        return result

    OwnedTree.__init__, OwnedTree.receipt, probe.sustained = initialize, receipt, sustained
    sys.argv = [str(Path(probe.__file__)), "--exe", args.exe, "--git", args.git,
                "--out", str(root / "native"), "--tree", "--cycles", "1",
                "--idle-seconds", "1", "--settle-seconds", "30", "--skip-latency"]
    try:
        probe.main()
    finally:
        OwnedTree.__init__, OwnedTree.receipt, probe.sustained = original_init, original_receipt, original_sustained
    native = json.loads((root / "native" / "receipt.json").read_text())
    job = captured["job_receipt"]
    assert job["coverage"] == "COMPLETE", "GUI tree accounting incomplete"
    assert all(row["image"] != "unavailable at notification" for row in job["processes"]), "incomplete process identity capture"
    git_rows = [row for row in job["processes"] if row["image"].lower() == "git.exe"]
    assert all("argv" in row.get("command_capture", {}) for row in git_rows), "incomplete Git command capture"
    rows = [row for row in git_rows if row["command_capture"]["parent_pid"] == native["product_pid"]
            and captured["begin"] <= (row["created_100ns"] - 116444736000000000) // 10 <= captured["end"]]
    assert all(os.path.normcase(str(Path(row["command_capture"]["argv"][0]).resolve())) == os.path.normcase(str(Path(args.git).resolve())) for row in rows), "captured root uses a different Git installation"
    commands = [row["command_capture"]["argv"][1:] for row in sorted(rows, key=lambda row: row["created_100ns"])]
    assert commands and sum(argv[:1] == ["commit"] for argv in commands) == 1, "missing/ambiguous actual commit sequence"
    repo = captured["repo"]
    def git(*argv):
        return sp.run([args.git, "-C", str(repo), *argv], check=True, capture_output=True).stdout
    assert repo.resolve() == (root / "native" / "repo").resolve() and (repo / ".git").is_dir()
    git("reset", "--hard", captured["initial_head"])
    for name, content in captured["dirty"].items():
        (repo / name).write_bytes(content)
    (repo / "f00000.txt").write_bytes(b"soak-0\ntwo\nthree\n")
    direct_env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_OPTIONAL_LOCKS="0", GIT_PAGER="cat", LC_ALL="C")
    for name in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY",
                 "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_NAMESPACE", "GIT_PREFIX", "GIT_CONFIG",
                 "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT", "GIT_EXTERNAL_DIFF", "GIT_TRACE",
                 "GIT_TRACE2", "GIT_TRACE2_EVENT", "GIT_TRACE2_PERF", "GIT_TRACE_PACK_ACCESS",
                 "GIT_TRACE_PERFORMANCE", "GIT_TRACE_SETUP"):
        direct_env.pop(name, None)
    direct = OwnedTree()
    try:
        for argv in commands:
            commit = argv[:1] == ["commit"]
            if commit:
                assert argv == ["commit", "--cleanup=strip", "-F", "-"], "unexpected stdin command"
            child = direct.spawn([args.git, *argv], cwd=repo, env=direct_env,
                                 stdin=sp.PIPE if commit else sp.DEVNULL, stdout=sp.PIPE, stderr=sp.PIPE)
            stdout, stderr = child.communicate(b"soak" if commit else None, timeout=60)
            assert child.returncode == 0, f"direct command failed: {argv[:1]}"
        direct_receipt = direct.receipt(probe.native()[2])
    finally:
        direct.close()
    assert direct_receipt["coverage"] == "COMPLETE", "direct tree accounting incomplete"
    assert git("rev-parse", "HEAD").strip().decode() == captured["final_head"], "replay HEAD differs"
    assert git("write-tree").strip().decode() == captured["final_tree"], "replay index differs"
    assert git("status", "--porcelain=v2", "-z").hex() == captured["final_status"], "replay worktree differs"
    baseline_cpu = direct_receipt["accounting"]["cpu_s"]
    limit = 1.25 * baseline_cpu + 0.5
    result = {"gui_cycle_cpu_s": captured["gui_cpu_s"], "direct_cpu_s": baseline_cpu,
              "limit_cpu_s": limit, "verdict": "PASS" if captured["gui_cpu_s"] <= limit else "FAIL",
              "commands": commands, "command_count": len(commands), "direct_tree": direct_receipt,
              "effects": "PASS: identical deterministic HEAD, index tree and worktree status",
              "scope": "One standard-fixture inspect/edit/refresh/stage/commit/detail/switch cycle plus draft save. No Git Trace2 or product tracing. Same disposable repo path reset in place; replay reuses existing objects, a conservative advantage to direct Git. No custom hooks."}
    (root / "cpu-comparison.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print({key: value for key, value in result.items() if key not in ("commands", "direct_tree")})


if __name__ == "__main__":
    main()
