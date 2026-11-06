#!/usr/bin/env python3
"""Fetch a pinned Sun SS-5 PROM, TurboGX ROM or QEMU OpenBIOS for local testing.

OpenBIOS conversion handles one verified image, not arbitrary ELF executables.
Only Python's standard library is needed; firmware stays in ignored local/.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import urllib.request
import zipfile

REVISION = "f71afb409a87c4ecd0f9d63a5963eb645dfab780"
URL = f"https://raw.githubusercontent.com/qemu/qemu/{REVISION}/pc-bios/openbios-sparc32"
SHA256 = "bf6e231eeea72b17f97055a20de1195005cf79cbd6c118b7ca89287b4a77fd1f"
PROM_VADDR = 0xFFD00000
PROM_SIZE = 1024 * 1024
SUN_REVISION = "a6f20b0740cfb4440d36ac04d7ad67e0207a73d4"
SUN_URL = f"https://raw.githubusercontent.com/andarazoroflove/sparc/{SUN_REVISION}/ss5.bin"
SUN_SHA256 = "e7f40845504c65f4011278aa3e97a9810aa36775e6c199b715839fbc25eec45e"
SUN_BLOB = "98aa7fef3c66b720081950b09e7609e9b178213a"
TGX_REVISION = "ae7feba5398fb1258b381d2a0a1c7779f7630d5f"
TGX_URL = f"https://raw.githubusercontent.com/abdess/retrobios/{TGX_REVISION}/bios/Arcade/Arcade/turbogx.zip"
TGX_SHA256 = "3fb9a8dbe7c8621dd9a65246ac78d9353a492c41862c1dd61656fb03d1a16259"
TGX_MEMBER = "sunw,501-2325.bin"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def prepare(elf):
    if sha256(elf) != SHA256:
        raise ValueError("firmware SHA-256 differs from the pinned QEMU image")
    if elf[:7] != b"\x7fELF\x01\x02\x01":
        raise ValueError("expected ELF32 big-endian version 1")
    kind, machine = struct.unpack_from(">HH", elf, 16)
    entry, phoff = struct.unpack_from(">II", elf, 24)
    phsize, phnum = struct.unpack_from(">HH", elf, 42)
    if (kind, machine, entry, phsize, phnum) != (2, 2, PROM_VADDR, 32, 1):
        raise ValueError("unexpected SPARC ELF executable layout")
    segment, offset, vaddr, paddr, filesz, memsz, _, _ = struct.unpack_from(">8I", elf, phoff)
    if (segment, vaddr, paddr) != (1, PROM_VADDR, PROM_VADDR):
        raise ValueError("unexpected firmware load segment")
    if not (0 < filesz <= memsz <= PROM_SIZE and offset + filesz <= len(elf)):
        raise ValueError("firmware segment exceeds its file or PROM aperture")
    # QEMU maps paddr - 0xffd00000 to the physical PROM at 0x70000000.
    # Preserve ELF BSS as zero, rather than filling it with the ROM's 0xff padding.
    raw = elf[offset:offset + filesz] + bytes(memsz - filesz)
    return raw, {"entry": entry, "segment_file_offset": offset,
                 "segment_file_size": filesz, "segment_memory_size": memsz}


def write_or_verify(path, data):
    try:
        with path.open("xb") as file:
            file.write(data)
    except FileExistsError:
        if path.read_bytes() != data:
            raise FileExistsError(f"refusing to replace different existing file: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("firmware", choices=["sun-ss5", "turbogx", "openbios"])
    parser.add_argument("--source", type=Path, help="verify and use an already downloaded image")
    parser.add_argument("--output-dir", type=Path,
                        default=Path(__file__).resolve().parents[1] / "local" / "firmware")
    args = parser.parse_args()
    url = {"sun-ss5": SUN_URL, "turbogx": TGX_URL, "openbios": URL}[args.firmware]
    if args.source is None:
        with urllib.request.urlopen(url, timeout=30) as response:
            downloaded = response.read(2 * PROM_SIZE + 1)
    else:
        downloaded = args.source.read_bytes()
    if args.firmware == "sun-ss5":
        if len(downloaded) != 262144 or sha256(downloaded) != SUN_SHA256:
            raise ValueError("firmware differs from the pinned 256 KiB SS-5 PROM")
        blob = hashlib.sha1(b"blob 262144\0" + downloaded).hexdigest()
        if blob != SUN_BLOB:
            raise ValueError("SS-5 PROM differs from the repository's Git blob")
        raw = downloaded
        prefix = "sun-ss5"
        manifest = {"source_url": SUN_URL, "repository_revision": SUN_REVISION,
                    "git_blob": blob, "raw_sha256": sha256(raw), "size": len(raw),
                    "firmware": "Sun SS-5 PROM from user-specified archive (ss5.bin)",
                    "physical_prom_base": 0x70000000}
    elif args.firmware == "turbogx":
        # Read the named member only; never extract archive paths to disk.
        with zipfile.ZipFile(io.BytesIO(downloaded)) as archive:
            if archive.getinfo(TGX_MEMBER).file_size != 32768:
                raise ValueError("unexpected TurboGX ROM size")
            raw = archive.read(TGX_MEMBER)
        if sha256(raw) != TGX_SHA256:
            raise ValueError("firmware differs from the pinned TurboGX ROM")
        prefix = "turbogx"
        manifest = {"source_url": TGX_URL, "repository_revision": TGX_REVISION,
                    "archive_member": TGX_MEMBER, "raw_sha256": sha256(raw),
                    "raw_sha1": hashlib.sha1(raw).hexdigest(), "size": len(raw),
                    "firmware": "Sun TurboGX 1 MiB FCode ROM (MAME turbogx)",
                    "physical_rom_base": 0x30000000}
    else:
        raw, layout = prepare(downloaded)
        prefix = "openbios-sparc32"
        manifest = {"source_url": URL, "qemu_revision": REVISION,
                    "elf_sha256": sha256(downloaded), "raw_sha256": sha256(raw),
                    "firmware": "OpenBIOS SPARC32 (replacement firmware, not a Sun PROM)",
                    "elf_layout": layout, "physical_prom_base": 0x70000000}
    args.output_dir.mkdir(parents=True, exist_ok=True)
    if args.firmware == "openbios":
        write_or_verify(args.output_dir / f"{prefix}.elf", downloaded)
    write_or_verify(args.output_dir / f"{prefix}.bin", raw)
    write_or_verify(args.output_dir / f"{prefix}.json",
                    (json.dumps(manifest, indent=2) + "\n").encode())
    print(f"PROM: {len(raw)} bytes, SHA-256 {sha256(raw)}")
    print(f"Saved in {args.output_dir}")


if __name__ == "__main__":
    main()
