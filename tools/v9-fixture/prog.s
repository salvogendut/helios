    .section .text
    .global _start
_start:
    sethi %hi(0xffffffff), %g1
    or    %g1, 0x3ff, %g1
    sllx  %g1, 32, %g1
    srlx  %g1, 32, %g1
    sethi %hi(0x500000), %g3
    addcc %g1, 1, %g2
    rd    %ccr, %g6
    stx   %g2, [%g3]
    ldx   [%g3], %g4
    ba    1f
    add   %g5, 7, %g5
1:
    stx   %g1, [%g3+32]
    stx   %g5, [%g3+8]
    stx   %g6, [%g3+16]
    stx   %g4, [%g3+24]
    stx   %g3, [%g3+40]
    mov   1, %o0
    sethi %hi(0x500000), %o1
    mov   48, %o2
    mov   4, %g1
    ta    0x6d
    mov   1, %g1
    clr   %o0
    ta    0x6d
    .section .data
    .align 8
buf: .skip 48
