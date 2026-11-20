# V9 fixture oracle

Requires SPARC64 binutils and QEMU user-mode SPARC64. From the repository root:

    tools/v9-fixture/run-qemu.sh /tmp/helios-v9-qemu

`prog.s` produces the 48-byte original fixture buffer. `edge.s` produces the
CCR/shift/branch edge outputs checked by `tests/v9_edges.rs`. The linker maps
code at `0x400000` and test data at `0x500000`.
