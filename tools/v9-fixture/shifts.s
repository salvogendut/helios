    .text
    .global _start,checkpoint
_start:
    sethi %hi(0x500000), %g3
    or %g0, 1, %g1
    sllx %g1, 32, %g1
    sll %g1, 0, %g2
    stx %g2, [%g3]
    or %g0, 32, %g4
    sll %g1, %g4, %g2
    stx %g2, [%g3 + 8]
    or %g0, 1, %g1
    sllx %g1, 31, %g1
    sll %g1, 1, %g2
    stx %g2, [%g3 + 16]
    srl %g1, 0, %g2
    stx %g2, [%g3 + 24]
    sra %g1, 0, %g2
    stx %g2, [%g3 + 32]
    srax %g1, 0, %g2
    stx %g2, [%g3 + 40]
checkpoint:
    mov 1, %o0
    mov %g3, %o1
    mov 48, %o2
    mov 4, %g1
    ta 0x6d
    mov 1, %g1
    clr %o0
    ta 0x6d
    .data
    .align 8
    .skip 48
