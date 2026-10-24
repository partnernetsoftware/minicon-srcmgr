"""Build and freeze a private Windows x64 candidate with source-path remapping.

Requires committed product inputs and this recipe. Does not sign or publish.
The private receipt retains exact flags; normal output contains no local paths.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess as sp
import tomllib
import traceback

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(*args):
    return sp.run(args, cwd=ROOT, check=True, capture_output=True).stdout


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    require(os.name == "nt", "this recipe requires a native Windows host")
    require(out.is_relative_to(ROOT / "target/qualification") and not out.exists(),
            "output must be a new private qualification directory")
    out.mkdir(parents=True)
    receipt = {"verdict": "RUNNING", "started_utc": datetime.now(timezone.utc).isoformat(),
               "target": TARGET, "build_profile": "release", "release_qualification": "UNQUALIFIED"}
    try:
        require(not os.environ.get("RUSTFLAGS") and not os.environ.get("CARGO_ENCODED_RUSTFLAGS"),
                "inherited Rust flags need an explicit recipe before candidate building")
        require(not any(os.environ.get(k) for k in ("RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER")),
                "compiler overrides need an explicit candidate recipe")
        paths = {p for pattern in ("crates/**/*.rs", "crates/**/Cargo.toml", "build-support/*.rs")
                 for p in ROOT.glob(pattern)}
        paths |= {ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")}
        paths |= {p for p in (ROOT / ".cargo").rglob("*") if p.is_file()}
        inputs = [{"path": p.relative_to(ROOT).as_posix(), "sha256": sha(p.read_bytes())} for p in sorted(paths)]
        recipe = {"path": Path(__file__).relative_to(ROOT).as_posix(), "sha256": sha(Path(__file__).read_bytes())}
        commit = run("git", "rev-parse", "HEAD").decode().strip()
        require(re.fullmatch(r"[0-9a-f]{40}", commit), "source must be an immutable commit")
        for item in inputs + [recipe]:
            blob = run("git", "show", commit + ":" + item["path"])
            require(item["sha256"] in (sha(blob), sha(blob.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))),
                    "product input or recipe differs from its committed source")
        compiler = run("rustc", "--version", "--verbose").decode()
        pinned = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
        require(compiler.startswith("rustc " + pinned + " "), "compiler does not match the pinned toolchain")
        require("host: " + TARGET in compiler, "this recipe requires the native x64 MSVC compiler host")
        user = Path(os.environ["USERPROFILE"]).resolve()
        cargo = Path(os.environ.get("CARGO_HOME", user / ".cargo")).resolve()
        sysroot = Path(run("rustc", "--print", "sysroot").decode().strip()).resolve()
        roots = [("developer", user), ("cargo", cargo), ("workspace", ROOT), ("toolchain", sysroot)]
        mappings = [(str(root), "/" + label) for label, root in roots]
        flags = []
        for source, destination in mappings:
            for spelling in dict.fromkeys((source, source.replace("\\", "/"))):
                flags.append("--remap-path-prefix=" + spelling + "=" + destination)
        env = dict(os.environ, CARGO_ENCODED_RUSTFLAGS="\x1f".join(flags))
        argv = ["cargo", "build", "--locked", "--offline", "--release", "--target", TARGET,
                "--target-dir", str(ROOT / "target"), "-p", "srcmgr-cli"]
        receipt.update(source_sha=commit, compiler=compiler, rustflags=flags, argv=argv,
                       build_recipe=recipe, remap_targets=[destination for _, destination in mappings])
        with (out / "build.log").open("wb") as log:
            result = sp.run(argv, cwd=ROOT, env=env, stdout=log, stderr=sp.STDOUT)
        require(result.returncode == 0, "candidate build failed; inspect the retained build.log")
        for item in inputs + [recipe]:
            require(sha((ROOT / item["path"]).read_bytes()) == item["sha256"], "build inputs changed while compiling")
        binary = ROOT / "target" / TARGET / "release/minicon-srcmgr.exe"
        data = binary.read_bytes()
        folded = data.lower()
        counts = {}
        for label, root in roots:
            variants = {str(root), str(root).replace("\\", "/")}
            patterns = {text.encode(encoding) for text in variants for encoding in ("utf-8", "utf-16-le")}
            counts[label] = sum(folded.count(pattern.lower()) for pattern in patterns)
        receipt["known_build_root_occurrences"] = counts
        require(not any(counts.values()), "candidate retains a known private build-root path")
        digest = sha(data)
        frozen = out / "candidate"
        frozen.mkdir()
        shutil.copy2(binary, frozen / binary.name)
        require(sha((frozen / binary.name).read_bytes()) == digest, "candidate changed during freeze")
        (frozen / "source-inputs.json").write_text(json.dumps({"artifact_sha256": digest, "source_inputs": inputs}, indent=2), encoding="utf-8")
        (frozen / "commit-binding.json").write_text(json.dumps({"commit": commit, "artifact_sha256": digest,
            "verdict": "PASS", "inputs": [{"path": i["path"], "match": True} for i in inputs]}, indent=2), encoding="utf-8")
        receipt.update(verdict="PASS", artifact_sha256=digest,
                       lockfile_sha256=sha((ROOT / "Cargo.lock").read_bytes()))
        (frozen / "build-receipt.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
        print(json.dumps({k: receipt[k] for k in ("verdict", "source_sha", "target", "artifact_sha256", "known_build_root_occurrences")}))
    except Exception as error:
        receipt.update(verdict="FAIL", failure=repr(error), traceback=traceback.format_exc())
        if isinstance(error, sp.CalledProcessError):
            (out / "command-stdout.log").write_bytes(error.stdout or b"")
            (out / "command-stderr.log").write_bytes(error.stderr or b"")
        raise
    finally:
        receipt["finished_utc"] = datetime.now(timezone.utc).isoformat()
        (out / "receipt.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
