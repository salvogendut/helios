! Reference source for the hand-encoded built-in demo; SPARC V8, big-endian.
! Loaded at address zero. The host stops after 43 completed instructions.
.text
.global _start
_start:
    or      %g0, 10, %g1
    or      %g0, 0, %g2
loop:
    add     %g2, %g1, %g2
    subcc   %g1, 1, %g1
    bne     loop
     nop
    st      %g2, [%g0 + 0x100]
done:
    ba,a    done
