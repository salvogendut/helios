"""Independently expand the OBP 3.31 stream from a local Ultra 5 flash to stdout."""

import hashlib
import sys
from pathlib import Path


def expand(stream):
    if stream[:3] != bytes.fromhex("1f9e0e"):
        raise ValueError("unexpected OBP compression header")
    dictionary = []
    output = bytearray()
    bit = 24
    while True:
        if not dictionary:
            dictionary = [bytes([i]) for i in range(256)] + [b"", b""]
            previous = b""
            width = 9
        if bit + width > len(stream) * 8:
            raise ValueError("missing end code")
        chunk = stream[bit // 8 : bit // 8 + 3]
        code = (int.from_bytes(chunk, "little") >> (bit % 8)) & ((1 << width) - 1)
        bit += width
        if code == 257:
            return output
        if code == 256:
            dictionary.clear()
            continue
        if code < len(dictionary):
            entry = dictionary[code]
        elif code == len(dictionary) and previous:
            entry = previous + previous[:1]
        else:
            raise ValueError("invalid dictionary code")
        output.extend(entry)
        if len(output) > 512 * 1024:
            raise ValueError("output exceeds mapped RAM page")
        if previous and len(dictionary) < 1 << 14:
            dictionary.append(previous + entry[:1])
            if len(dictionary) == 1 << width and width < 14:
                width += 1
        previous = entry


if __name__ == "__main__":
    rom = Path(sys.argv[1]).read_bytes()
    if hashlib.sha256(rom).hexdigest() != "fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616":
        sys.exit("expected the reconstructed 1 MiB Ultra 5 OBP 3.31.0 / POST 3.10 flash")
    result = expand(rom[0x2a80:0x3b664])
    if hashlib.sha256(result).hexdigest() != "4084f24a619f2c14cd0a87c6a5f82f29f7513627c74a85d759c6271de184b366":
        sys.exit("expanded image does not match the reference digest")
    sys.stdout.buffer.write(result)
