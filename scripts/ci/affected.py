#!/usr/bin/env python3
"""Select the workspace packages a pull request affects.

Usage: affected.py --paths FILE [--metadata FILE] [--github-output FILE]

FILE holds the changed paths relative to the repository root, separated by NUL (git diff -z);
"-" reads standard input. --metadata is the output of `cargo metadata --no-deps
--format-version 1` (run on demand when omitted).

Prints and optionally appends to $GITHUB_OUTPUT:
  full=true|false   test the whole workspace
  packages=a b c    the affected packages (empty when full, or when nothing needs building)

A changed file maps to the package whose directory holds it. The packages that depend on those
(directly or not, dev-dependencies included) are added. Anything that can change every package,
or that no package owns, selects the full workspace. Only documentation and packaging files are
ignored.
"""
import argparse
import json
import subprocess
import sys

# Changes here can affect every package.
FULL_PREFIXES = (".cargo/", ".config/", ".github/", "vendor/", "tests/", "scripts/")
FULL_FILES = ("Cargo.toml", "Cargo.lock")
# Changes here affect no code: documentation and packaging.
IGNORED_PREFIXES = ("docs/", "packaging/")
# Root files that a crate embeds (include_str!), so they are not ignorable documentation.
EMBEDDED = {"ATTRIBUTION.md": "solidcraft-ui-egui"}


def load_metadata(path):
    if path:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def read_paths(src):
    data = sys.stdin.buffer.read() if src == "-" else open(src, "rb").read()
    return [p.decode() for p in data.split(b"\0") if p]


def select(meta, paths):
    """Return (full, sorted package names)."""
    root = meta["workspace_root"].rstrip("/") + "/"
    pkgs = {}
    for p in meta["packages"]:
        manifest = p["manifest_path"]
        assert manifest.startswith(root), manifest
        pkgs[p["name"]] = manifest[len(root):].rsplit("/", 1)[0] + "/"
    by_dir = {d: n for n, d in pkgs.items()}
    # Reverse dependencies inside the workspace, matched by the dependency's path.
    dependents = {n: set() for n in pkgs}
    for p in meta["packages"]:
        for d in p["dependencies"]:
            path = d.get("path")
            if not path:
                continue
            rel = path.rstrip("/") + "/"
            target = by_dir.get(rel[len(root):]) if rel.startswith(root) else None
            if target:
                dependents[target].add(p["name"])

    hit = set()
    for path in paths:
        if path in FULL_FILES or path.startswith(FULL_PREFIXES) or path.startswith("rust-toolchain"):
            return True, []
        owners = [d for d in by_dir if path.startswith(d)]
        if owners:
            hit.add(by_dir[max(owners, key=len)])
        elif path in EMBEDDED:
            hit.add(EMBEDDED[path])
        elif path.startswith(IGNORED_PREFIXES) or ("/" not in path and path.endswith(".md")):
            continue
        else:
            return True, []  # not owned by any package

    todo, seen = list(hit), set(hit)
    while todo:
        for dep in dependents[todo.pop()]:
            if dep not in seen:
                seen.add(dep)
                todo.append(dep)
    return False, sorted(seen)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--paths", required=True)
    ap.add_argument("--metadata")
    ap.add_argument("--github-output")
    a = ap.parse_args()
    full, packages = select(load_metadata(a.metadata), read_paths(a.paths))
    lines = [f"full={'true' if full else 'false'}", f"packages={' '.join(packages)}"]
    print("\n".join(lines))
    if a.github_output:
        with open(a.github_output, "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
