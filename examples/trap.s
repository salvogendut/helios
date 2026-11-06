! Reference source for the built-in trap-demo; raw image loaded at address 0.
! Host stops after 14 steps: 13 executed instructions and one trap entry.
.text
.global _start
_start:
    sethi   %hi(0x1000), %g1
    wr      %g1, 0, %tbr
    nop
    nop
    nop
    wr      %g0, 0x20, %psr       ! user mode, traps enabled, window 0
    nop
    nop
    nop
    ta      7
    add     %g3, 1, %g3          ! executes after returning to user mode
done:
    ba,a    done

.org 0x1870                     ! TBA | (software trap 0x87 << 4)
handler:
    add     %g2, 1, %g2
    jmpl    %l2, %g0            ! saved nPC, skip the trapping instruction
     rett   %l2 + 4             ! JMPL delay slot; restore CWP, ET and S
