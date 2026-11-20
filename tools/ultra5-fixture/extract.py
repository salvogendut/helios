#!/usr/bin/env python3
"""Reconstruct the Ultra 5/10 flash from the OBP and POST records in patch 106121-18."""

import argparse
import hashlib
import json
import struct
from itertools import combinations
from pathlib import Path

UTILITY_SHA256 = "e0ef541b64de9ab23b9423fce71fd56a7ddcc29cb28de8b66a4313d48516a29e"
IMAGE_SHA256 = "fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616"
HALF_SIZE = 0x80000
POR_OFFSET = 0x20

# Board-name/OBP/size/POST tuples in the pinned updater's ELF data segment.
DESCRIPTORS = {"SUNW,375-0009": 0x59608, "SUNW,375-0066": 0x59630}
DATA_VA = 0x449A98
DATA_OFFSET = 0x39A98


def read_board(utility, name, descriptor):
    board, obp, size, post = struct.unpack_from(">4I", utility, descriptor)
    board_offset = board - DATA_VA + DATA_OFFSET
    if size != 2 * HALF_SIZE or not utility.startswith(name.encode() + b"\0", board_offset):
        raise ValueError(f"unexpected descriptor for {name}")

    halves = []
    offsets = {}
    for label, pointer in (("OBP", obp), ("POST", post)):
        offset = pointer - DATA_VA + DATA_OFFSET
        half = utility[offset:offset + HALF_SIZE]
        if (
            len(half) != HALF_SIZE
            or half[:4] != b"OBMD"
            or not half.startswith(label.encode() + b"\0", 16)
        ):
            raise ValueError(f"invalid {label} record for {name} at {offset:#x}")
        halves.append(half)
        offsets[label.lower() + "_offset"] = hex(offset)
    return b"".join(halves), offsets


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("utility", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()
    manifest_path = args.manifest or args.output.with_suffix(".json")

    for left, right in combinations((args.utility, args.output, manifest_path), 2):
        if left.resolve() == right.resolve() or (
            left.exists() and right.exists() and left.samefile(right)
        ):
            parser.error("the updater, image and manifest require distinct paths")

    utility = args.utility.read_bytes()
    source_sha256 = hashlib.sha256(utility).hexdigest()
    if source_sha256 != UTILITY_SHA256:
        raise ValueError("source SHA-256 differs from patch 106121-18's flash-update-Ultra510-18")

    images = []
    boards = {}
    for name, descriptor in DESCRIPTORS.items():
        image, offsets = read_board(utility, name, descriptor)
        images.append(image)
        boards[name] = {"descriptor_offset": hex(descriptor), **offsets}
    if images[0] != images[1]:
        raise ValueError("the two board descriptors produced different flash images")
    image = images[0]
    digest = hashlib.sha256(image).hexdigest()
    word = int.from_bytes(image[POR_OFFSET:POR_OFFSET + 4], "big")
    if digest != IMAGE_SHA256 or word != 0xc4d808a0:
        raise ValueError("reconstructed flash digest or reset vector differs from OBP 3.31.0")

    manifest = {
        "source": str(args.utility),
        "source_sha256": source_sha256,
        "boards": boards,
        "half_size": HALF_SIZE,
        "size": len(image),
        "sha256": digest,
        "por_offset": hex(POR_OFFSET),
        "por_word": f"0x{word:08x}",
        "obp": "3.31.0",
        "post": "3.1.0",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(image)
    report = json.dumps(manifest, indent=2)
    manifest_path.write_text(report + "\n", encoding="utf-8")
    print(report)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error
