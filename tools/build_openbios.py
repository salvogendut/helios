#!/usr/bin/env python3
"""Build the documented Helios OpenBIOS SPARC32 port without bundling firmware.

Requires git, make, gcc, xsltproc, toke and a SPARC cross compiler. Source,
build products and firmware remain in ignored local/. Existing build trees
and differing output images are never overwritten. See docs/firmware.md.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

REVISION = "e5ac46dd24e6216c36aa80462af25457e7029440"
URL = "https://github.com/openbios/openbios.git"
ROOT = Path(__file__).resolve().parents[1]


def run(args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def write_or_verify(path, data):
    try:
        with path.open("xb") as stream:
            stream.write(data)
    except FileExistsError:
        if path.is_symlink() or path.read_bytes() != data:
            raise FileExistsError(f"refusing to replace existing different file: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-dir", type=Path, default=ROOT / "local/openbios-build")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "local/firmware")
    parser.add_argument("--cross-prefix", default="sparc64-linux-gnu-")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--source-repository", type=Path,
                        help="clone a local repository containing the pinned upstream commit")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    for program in ["git", "make", "gcc", "xsltproc", "toke", args.cross_prefix + "gcc"]:
        if shutil.which(program) is None:
            parser.error(f"required program not found: {program}")
    if args.work_dir.exists():
        parser.error(f"build directory already exists; select a new --work-dir: {args.work_dir}")
    args.work_dir.parent.mkdir(parents=True, exist_ok=True)
    source = str(args.source_repository.resolve()) if args.source_repository else URL
    run(["git", "clone", "--no-checkout", "--filter=blob:none", source, str(args.work_dir)])
    run(["git", "checkout", "--detach", REVISION], cwd=args.work_dir)
    patch = ROOT / "tools/openbios/helios.patch"
    run(["git", "apply", "--check", str(patch)], cwd=args.work_dir)
    run(["git", "apply", str(patch)], cwd=args.work_dir)
    env = dict(os.environ, CROSS_COMPILE=args.cross_prefix)
    run(["./config/scripts/switch-arch", "sparc32"], cwd=args.work_dir, env=env)
    run(["make", f"-j{args.jobs}"], cwd=args.work_dir, env=env)
    image = (args.work_dir / "obj-sparc32/openbios-builtin.elf").read_bytes()
    manifest = {
        "firmware": "OpenBIOS SPARC32, Helios platform port",
        "upstream": URL,
        "revision": REVISION,
        "patch_sha256": hashlib.sha256(patch.read_bytes()).hexdigest(),
        "elf_sha256": hashlib.sha256(image).hexdigest(),
        "optimization": "-O2",
        "compiler": subprocess.check_output([args.cross_prefix + "gcc", "--version"], text=True).splitlines()[0],
        "changes": ["native dictionary-chain search with original Forth search-order semantics",
                    "speed-oriented target compilation (-O2)",
                    "full RAM clear in 64-byte blocks before console initialization",
                    "NCR counter-based delays", "bounded absent-floppy detection",
                    "ESP reset, selection, DMA and status/interrupt sequencing",
                    "SBus arbitration grants before I/O", "MBus MIDs 8/A, per-module reg properties and AER CPU admission",
                    "SS-20 APC device-tree correction", "85/75 MHz nominal module clocks",
                    "TurboGX historical intr FCode and sun4m SBus PIL mapping",
                    "sun4m CD boot slice d", "bounded 2 MiB firmware virtual I/O window",
                    "balanced fb8-install stack when color! is private or absent",
                    "Sun version-1 FCode counted-string terminator compatibility",
                    "1200-baud keyboard SCC setup and nonblocking station filtering"],
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    target = args.output_dir / "openbios-helios-sparc32.elf"
    write_or_verify(target, image)
    write_or_verify(target.with_suffix(".json"), (json.dumps(manifest, indent=2) + "\n").encode())
    print(f"Built {target}\nSHA-256 {manifest['elf_sha256']}")


if __name__ == "__main__":
    main()
