! Synthetic sun4m reset PROM. Both timers interrupt the main loop once.
! g3 counts main-loop work, g5 level-10 entries, g7 level-14 entries.
! Each handler reads its limit to acknowledge, then selects free-running mode.
! TBA=0x1000; WIM=0 permits the return to the original register window.

    sethi %hi(0x1000), %g2
    wr %g2, 0, %tbr
    wr %g0, 0, %wim
    nop
    nop
    nop
    sethi %hi(0x71d10000), %g1
    or %g0, 0xc00, %g2
    st %g2, [%g1]
    sethi %hi(0x71d00000), %g4
    sethi %hi(0x1800), %g2
    st %g2, [%g4]
    sethi %hi(0x71e10000), %g6
    sethi %hi(0x80080000), %g2
    st %g2, [%g6+8]
    wr %g0, 0xa0, %psr
    nop
    nop
    nop
work:
    add %g3, 1, %g3
    ba work
    nop
    .org 0x200
system_handler:
    ld [%g1], %l0
    st %g0, [%g1]
    add %g5, 1, %g5
    jmpl %l1, %g0
    rett %l2
    .org 0x240
processor_handler:
    ld [%g4], %l0
    st %g0, [%g4]
    add %g7, 1, %g7
    jmpl %l1, %g0
    rett %l2
    .org 0x11a0
    ba system_handler
    nop
    .org 0x11e0
    ba processor_handler
    nop
