#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
out=${1:-/tmp/helios-v9-qemu}
mkdir -p "$out"
sparc64-linux-gnu-as -64 -o "$out/prog.o" "$here/prog.s"
sparc64-linux-gnu-ld -m elf64_sparc -static -nostdlib -T "$here/link.ld" -o "$out/prog" "$out/prog.o"
qemu-sparc64 "$out/prog" > "$out/output.bin"
od -An -tx8 -v "$out/output.bin"
sparc64-linux-gnu-as -64 -o "$out/edge.o" "$here/edge.s"
sparc64-linux-gnu-ld -m elf64_sparc -static -nostdlib -T "$here/link.ld" -o "$out/edge" "$out/edge.o"
qemu-sparc64 "$out/edge" > "$out/edge-output.bin"
od -An -tx8 -v "$out/edge-output.bin"
sparc64-linux-gnu-as -64 -o "$out/shifts.o" "$here/shifts.s"
sparc64-linux-gnu-ld -m elf64_sparc -static -nostdlib -T "$here/link.ld" -o "$out/shifts" "$out/shifts.o"
qemu-sparc64 "$out/shifts" > "$out/shifts-output.bin"
echo "QEMU shifts output (high-word SLL discrepancy is expected):"
od -An -tx8 -v "$out/shifts-output.bin"
