"""Prepare a private, unqualified Windows x64 portable candidate from frozen bytes.

Does not build, sign, scan, qualify or publish. Requires an exact source binding
and a passing native receipt for the supplied executable. Outputs stay ignored.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess as sp
import tarfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
FAILURE_ROOT = None


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(*args, **kwargs):
    return sp.run(args, check=True, capture_output=True, **kwargs).stdout


def pe_imports(data):
    """Read direct and delayed DLL imports; this is not DLL resolution evidence."""
    require(data[:2] == b"MZ", "candidate is not a PE executable")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    require(data[pe:pe + 4] == b"PE\0\0", "invalid PE signature")
    machine, sections = struct.unpack_from("<HH", data, pe + 4)
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    require(machine == 0x8664 and struct.unpack_from("<H", data, optional)[0] == 0x20B,
            "candidate must be a native Windows x64 PE32+ executable")
    require(struct.unpack_from("<H", data, optional + 68)[0] in (2, 3), "invalid executable subsystem")
    headers = struct.unpack_from("<I", data, optional + 60)[0]
    image_base = struct.unpack_from("<Q", data, optional + 24)[0]
    ranges = []
    for index in range(sections):
        size, rva, raw_size, raw = struct.unpack_from("<IIII", data, optional + optional_size + index * 40 + 8)
        ranges.append((rva, max(size, raw_size), raw, raw_size))

    def offset(rva):
        if rva < headers:
            return rva
        for start, size, raw, raw_size in ranges:
            if start <= rva < start + size:
                require(rva - start < raw_size, "import points outside raw PE section")
                return raw + rva - start
        raise ValueError("unmapped PE import RVA")

    imports = set()
    directory_count = struct.unpack_from("<I", data, optional + 108)[0]
    for directory, width, name_field in ((1, 20, 12), (13, 32, 4)):
        if directory >= directory_count:
            continue
        rva, size = struct.unpack_from("<II", data, optional + 112 + directory * 8)
        if not rva:
            continue
        terminated = False
        for step in range(size // width):
            start = offset(rva + step * width)
            descriptor = data[start:start + width]
            require(len(descriptor) == width, "truncated PE import directory")
            if not any(descriptor):
                terminated = True
                break
            name = struct.unpack_from("<I", descriptor, name_field)[0]
            if directory == 13 and not struct.unpack_from("<I", descriptor)[0] & 1:
                name -= image_base
            start = offset(name)
            end = data.find(b"\0", start, start + 256)
            require(end > start, "unterminated DLL name")
            dll = data[start:end].decode("ascii")
            require(re.fullmatch(r"[A-Za-z0-9_.-]+\.dll", dll, re.I), "invalid DLL import name")
            imports.add(dll.lower())
        require(terminated, "unterminated PE import descriptors")
    return sorted(imports)


def main():
    global FAILURE_ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--build-profile", required=True, choices=("release", "debug"))
    parser.add_argument("--receipt", required=True, action="append", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    require(os.name == "nt", "Windows signature inspection requires the Windows host")
    out = args.out.resolve()
    require(out.is_relative_to(ROOT / "target/qualification") and not out.exists(),
            "output must be a new private qualification directory")
    out.mkdir(parents=True)
    FAILURE_ROOT = out
    candidate = args.candidate.resolve()
    executable = (candidate / "minicon-srcmgr.exe").read_bytes()
    digest = sha(executable)
    inputs = json.loads((candidate / "source-inputs.json").read_text(encoding="utf-8"))
    binding = json.loads((candidate / "commit-binding.json").read_text(encoding="utf-8"))
    require(inputs["artifact_sha256"] == binding["artifact_sha256"] == digest,
            "candidate executable hash differs from its frozen identity")
    require(binding["verdict"] == "PASS" and re.fullmatch(r"[0-9a-f]{40}", binding["commit"]),
            "candidate lacks a passing immutable source binding")
    seen = set()
    for item in inputs["source_inputs"]:
        path = Path(item["path"])
        require(not path.is_absolute() and ".." not in path.parts and path.as_posix() not in seen,
                "invalid or duplicated source input path")
        seen.add(path.as_posix())
        require(sha((ROOT / path).read_bytes()) == item["sha256"], "current product inputs differ from candidate")
        blob = command("git", "-C", str(ROOT), "show", binding["commit"] + ":" + path.as_posix())
        require(item["sha256"] in (sha(blob), sha(blob.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))),
                "source input does not bind to the candidate commit")
    expected = {p.relative_to(ROOT).as_posix() for pattern in
                ("crates/**/*.rs", "crates/**/Cargo.toml", "build-support/*.rs") for p in ROOT.glob(pattern)}
    expected |= {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
    expected |= {p.relative_to(ROOT).as_posix() for p in (ROOT / ".cargo").rglob("*") if p.is_file()}
    require(seen == expected, "frozen source inventory is incomplete")
    build_raw = (candidate / "build-receipt.json").read_bytes()
    build = json.loads(build_raw)
    require(build.get("verdict") == "PASS" and build.get("artifact_sha256") == digest
            and build.get("source_sha") == binding["commit"] and build.get("target") == TARGET
            and build.get("build_profile") == args.build_profile
            and build.get("lockfile_sha256") == sha((ROOT / "Cargo.lock").read_bytes()),
            "candidate lacks a matching successful recipe-build receipt")
    require(set(build.get("known_build_root_occurrences", {})) == {"workspace", "developer", "cargo", "toolchain"}
            and not any(build["known_build_root_occurrences"].values()), "known private build roots remain unqualified")
    recipe = build["build_recipe"]
    require(recipe["path"] == "scripts/build_windows_candidate.py"
            and recipe["sha256"] == sha((ROOT / recipe["path"]).read_bytes()), "build recipe identity changed")
    blob = command("git", "-C", str(ROOT), "show", binding["commit"] + ":" + recipe["path"])
    require(recipe["sha256"] in (sha(blob), sha(blob.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))),
            "build recipe does not bind to frozen source")
    receipts = []
    for path in args.receipt:
        raw = path.read_bytes()
        receipt = json.loads(raw)
        tree = receipt.get("process_tree", {})
        require(receipt.get("artifact_sha256") == digest and receipt.get("verdict") == "PASS"
                and receipt.get("exit_code") == 0 and tree.get("coverage") == "COMPLETE"
                and tree.get("accounting", {}).get("active_processes") == 0,
                "native receipt does not qualify this artifact's selected functional scope")
        receipts.append({"id": path.parent.name, "sha256": sha(raw), "scope": "selected functional cases only"})
    imports = pe_imports(executable)
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["workspace"]["package"]["version"]
    require(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version), "unsupported package version")
    metadata = json.loads(command("cargo", "metadata", "--locked", "--offline", "--filter-platform", TARGET,
                                  "--format-version", "1", cwd=ROOT))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    cli = next(p["id"] for p in packages.values() if p["name"] == "srcmgr-cli" and p["source"] is None)
    visited, todo = set(), [cli]
    while todo:
        package = todo.pop()
        if package not in visited:
            visited.add(package)
            todo.extend(d["pkg"] for d in nodes[package]["deps"]
                        if any(k["kind"] != "dev" for k in d["dep_kinds"]))
    files = {"minicon-srcmgr.exe": executable, "LICENSE": (ROOT / "LICENSE").read_bytes()}
    inventory = []
    locked = {(p["name"], p["version"]): p for p in
              tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))["package"]}
    for package_id in sorted(visited, key=lambda i: (packages[i]["name"], packages[i]["version"])):
        package = packages[package_id]
        entry = {"name": package["name"], "version": package["version"], "license": package["license"], "notices": []}
        require(entry["license"], "dependency has no declared license")
        if package["source"] is not None:
            base = Path(package["manifest_path"]).parent
            roots = [base]
            archive = None
            git_root = None
            if package["source"].startswith("git+"):
                git_root = Path(command("git", "-C", str(base), "rev-parse", "--show-toplevel").decode().strip())
                require(base.is_relative_to(git_root), "Git dependency root escaped checkout")
                roots += [p for p in base.parents if p == git_root or p.is_relative_to(git_root)]
            else:
                archive = base.parents[2] / "cache" / base.parent.name / (base.name + ".crate")
                with archive.open("rb") as source:
                    require(hashlib.file_digest(source, "sha256").hexdigest() ==
                            locked[(package["name"], package["version"])]["checksum"],
                            "registry source archive differs from locked checksum")

            def source_bytes(path):
                if git_root is not None:
                    revision = package["source"].rsplit("#", 1)[-1]
                    require(re.fullmatch(r"[0-9a-f]{40}", revision), "Git source is not immutable")
                    return command("git", "-C", str(git_root), "show", revision + ":" + path.relative_to(git_root).as_posix())
                with tarfile.open(archive) as source:
                    member = source.extractfile(base.name + "/" + path.relative_to(base).as_posix())
                    require(member is not None, "license source is not a regular archive member")
                    return member.read()

            original_manifest = source_bytes(Path(package["manifest_path"]))
            require(original_manifest.replace(b"\r\n", b"\n") ==
                    Path(package["manifest_path"]).read_bytes().replace(b"\r\n", b"\n"),
                    "dependency metadata manifest differs from immutable source")
            for directory in roots:
                notices = [p for p in directory.iterdir() if p.is_file()
                           and p.name.lower().startswith(("license", "copying", "notice"))]
                if notices:
                    break
            require(notices, "no license text found for " + package["name"])
            for notice in sorted(notices):
                name = f"licenses/{package['name']}-{package['version']}/{notice.name}"
                files[name] = source_bytes(notice)
                entry["notices"].append(name)
            if package["name"] == "libsqlite3-sys" and "bundled" in nodes[package_id]["features"]:
                prefix = source_bytes(base / "sqlite3/sqlite3.c")[:8192]
                marker = prefix.index(b"The author disclaims copyright")
                start = prefix.rfind(b"/*", 0, marker)
                end = prefix.index(b"*/", marker) + 2
                name = f"licenses/{package['name']}-{package['version']}/SQLITE-SOURCE-NOTICE.txt"
                files[name] = prefix[:prefix.index(b"*/") + 2] + b"\n\n" + prefix[start:end] + b"\n"
                entry["notices"].append(name)
        else:
            entry["notices"] = ["LICENSE"]
        inventory.append(entry)
    require(command("rustc", "--version", "--verbose", cwd=ROOT).decode() == build["compiler"],
            "installed compiler differs from the candidate's runtime notice source")
    sysroot = Path(command("rustc", "--print", "sysroot", cwd=ROOT).decode().strip())
    rust_docs = sysroot / "share/doc/rust"
    runtime_notices = []
    for notice in [rust_docs / "COPYRIGHT-library.html", *sorted((rust_docs / "licenses").glob("*.txt"))]:
        name = "licenses/rust-standard-library/" + notice.relative_to(rust_docs).as_posix()
        files[name] = notice.read_bytes()
        runtime_notices.append(name)
    require(len(runtime_notices) > 1, "Rust standard-library distribution notices are unavailable")
    # A PowerShell 7 parent can export incompatible module paths into 5.1.
    # Let this no-profile inspection host discover its own built-in modules.
    env = {k: v for k, v in os.environ.items() if k.casefold() != "psmodulepath"}
    env["SRCMGR_CANDIDATE_EXE"] = str(candidate / "minicon-srcmgr.exe")
    signature = command("powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                        "Import-Module Microsoft.PowerShell.Security -ErrorAction Stop; "
                        "(Get-AuthenticodeSignature -LiteralPath $env:SRCMGR_CANDIDATE_EXE).Status.ToString()", env=env).decode().strip()
    require(signature and "\n" not in signature, "signature inspection returned no single status")
    package_id = f"minicon-srcmgr-{version}-windows-x86_64"
    files["THIRD_PARTY_NOTICES.md"] = ("# Dependency notices\n\n"
        "Conservative locked Windows normal/build dependency inventory, including compiler tooling. "
        "License declarations and copied upstream texts require distribution review; this is not a legal clearance.\n\n"
        + "\n".join(f"- {p['name']} {p['version']}: {p['license']}; " + ", ".join(p["notices"]) for p in inventory)
        + "\n\nRust standard-library copyright index and conservative bundled license catalogue: "
        + ", ".join(runtime_notices) + "\n").encode()
    files["START.md"] = ("# Internal candidate — not qualified for publication\n\n"
        "Start `minicon-srcmgr.exe` for the GUI. Open/Create are explicit; Git is separately installed "
        "(minimum baseline 2.43, capability checks still apply). You may select Git in the application. "
        "No account, admin installation, Rust or developer SDK is intended for normal use.\n\n"
        "Target floor: Windows 11 24H2 x64. Clean-image runtime DLL resolution, native text/accessibility, "
        "resource/response/stability, signature and scan gates remain unqualified. Direct imported DLLs "
        "are listed in the manifest; they do not prove installed prerequisites. This ZIP is a private "
        "preparation artifact, not a supported or signed release. See the public repository's PRD 12/14 "
        "and version plan for the selected evidence and remaining requirements.\n").encode()
    document = {
        "schema": "minicon-srcmgr.private-candidate/1", "package": package_id, "version": version,
        "source_sha": binding["commit"], "lockfile_sha256": sha((ROOT / "Cargo.lock").read_bytes()),
        "toolchain": tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"],
        "target": TARGET, "build_profile": args.build_profile, "enabled_features": nodes[cli]["features"],
        "build_provenance": "observed local recipe build; no signed build attestation",
        "build_receipt_sha256": sha(build_raw), "build_recipe": recipe,
        "compiler": build["compiler"], "remap_targets": build["remap_targets"],
        "known_build_root_occurrences": build["known_build_root_occurrences"],
        "minicon_revision": manifest["workspace"]["dependencies"]["minicon-platform"]["rev"],
        "artifact_sha256": digest, "direct_dll_imports": imports,
        "runtime_resolution": "NOT-RUN on clean supported image", "git": "separately installed; baseline >=2.43",
        "authenticode_observed": signature, "release_qualification": "UNQUALIFIED",
        "signing_gate": "NOT-QUALIFIED", "scan_gate": "NOT-RUN", "notice_review": "PENDING",
        "evidence": receipts, "dependencies": inventory, "rust_standard_library_notices": runtime_notices,
        "packager_sha256": sha(Path(__file__).read_bytes()),
        "files": {name: sha(data) for name, data in sorted(files.items())},
    }
    files["manifest.json"] = (json.dumps(document, indent=2) + "\n").encode()
    archive = out / (package_id + ".zip")
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for name, data in sorted(files.items()):
            info = zipfile.ZipInfo(package_id + "/" + name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            bundle.writestr(info, data)
    with zipfile.ZipFile(archive) as bundle:
        require(set(bundle.namelist()) == {package_id + "/" + n for n in files}, "archive entry mismatch")
        for name, data in files.items():
            require(bundle.read(package_id + "/" + name) == data, "archive bytes differ from reviewed inputs")
    receipt = {"verdict": "PASS: private archive identity only", "release_qualification": "UNQUALIFIED",
               "archive_sha256": sha(archive.read_bytes()), "artifact_sha256": digest, "source_sha": binding["commit"],
               "archive_bytes": archive.stat().st_size, "dependency_count": len(inventory),
               "authenticode_observed": signature, "packager_sha256": document["packager_sha256"]}
    (out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        if FAILURE_ROOT is not None:
            failure = {"verdict": "FAIL", "type": type(error).__name__, "failure": str(error)}
            if isinstance(error, sp.CalledProcessError):
                (FAILURE_ROOT / "command-stdout.log").write_bytes(error.stdout or b"")
                (FAILURE_ROOT / "command-stderr.log").write_bytes(error.stderr or b"")
            (FAILURE_ROOT / "failure.json").write_text(json.dumps(failure, indent=2), encoding="utf-8")
        raise
