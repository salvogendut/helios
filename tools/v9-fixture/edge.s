    .text
    .global _start
_start:
    sethi %hi(0x500000), %g3
    or %g0, -1, %g1
    addcc %g1, 1, %g1
    rd %ccr, %g6
    stx %g6, [%g3]

    or %g0, -1, %g1
    srlx %g1, 1, %g1
    addcc %g1, 1, %g2
    rd %ccr, %g6
    stx %g6, [%g3 + 8]

    or %g0, 1, %g1
    sllx %g1, 32, %g1
    srl %g1, 0, %g2
    stx %g2, [%g3 + 16]
    or %g0, 32, %g4
    or %g0, 1, %g1
    sll %g1, %g4, %g2
    stx %g2, [%g3 + 24]

    addcc %g0, 0, %g0
    be target
     add %g5, 7, %g5
    add %g5, 32, %g5
target:
    stx %g5, [%g3 + 32]
    bne,a next
     add %g5, 32, %g5
    add %g5, 1, %g5
next:
    stx %g5, [%g3 + 40]
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
buf: .skip 48
