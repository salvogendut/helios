! Synthetic reset PROM; see machine::serial_demo_image for the exact bytes.
! Poll TTYA, check all 16 IDPROM bytes, print HELIOS and echo one input byte.
! g3 = machine type (0x80), g5 = checksum (0), g7 = completion (1).
! Boot-mode ASI 9 supplies the string; ordinary data accesses reach devices.

    sethi %hi(0x71100000), %g1
    or %g1, 4, %g1
    or %g0, 0x9, %g2
    stb %g2, [%g1]
    or %g0, 0xc0, %g2
    stb %g2, [%g1]
    or %g0, 0x4, %g2
    stb %g2, [%g1]
    or %g0, 0x44, %g2
    stb %g2, [%g1]
    or %g0, 0xe, %g2
    stb %g2, [%g1]
    or %g0, 0x3, %g2
    stb %g2, [%g1]
    or %g0, 0x3, %g2
    stb %g2, [%g1]
    or %g0, 0xc1, %g2
    stb %g2, [%g1]
    or %g0, 0x5, %g2
    stb %g2, [%g1]
    or %g0, 0xea, %g2
    stb %g2, [%g1]
    sethi %hi(0x71201fd8), %g4
    or %g4, 0x3d8, %g4
    ldub [%g4+1], %g3
    or %g0, 0x0, %g5
    or %g0, 0x10, %g6
checksum:
    ldub [%g4], %g2
    xor %g5, %g2, %g5
    add %g4, 1, %g4
    subcc %g6, 1, %g6
    bne checksum
    nop
    or %g0, 0x200, %g4
    or %g0, 0x6, %g6
transmit:
    ldub [%g1], %g2
    andcc %g2, 4, %g0
    be transmit
    nop
    lduba [%g4] 0x09, %g2
    stb %g2, [%g1+2]
    add %g4, 1, %g4
    subcc %g6, 1, %g6
    bne transmit
    nop
receive:
    ldub [%g1], %g2
    andcc %g2, 1, %g0
    be receive
    nop
    ldub [%g1+2], %g2
    stb %g2, [%g1+2]
    or %g0, 0xa, %g2
    stb %g2, [%g1+2]
    or %g0, 0x1, %g7
done:
    ba,a done

    .org 0x200
message:
    .ascii "HELIOS"
