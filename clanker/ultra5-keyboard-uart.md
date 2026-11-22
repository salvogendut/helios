# Ultra 5 cold-start keyboard UART

The original image programs FER/FAR/PTR to `0x0f/0xb0/0x80`. At
EBus offset `0x3083f8` its ten UART writes, as (offset, byte) in hex, are
`(1,00) (4,00) (2,00) (2,06) (2,01) (3,83) (0,60) (1,00) (3,03) (4,00)`.
The original-ROM regression records each write from the executing guest,
then reads back the divisor, FIFO, line/modem control and line status.
The ordinary regression checks separation of DLAB and IER/THR, COM1 decode,
configuration changes, rejected accesses and console isolation.

With a zero marker, OBP retires 173,486 instructions with diagnostics enabled
or 171,775 with them disabled. The next instruction is `0xc0f243a0`
at flash offset `0x630`. Its address operand is `0x01fe010008b0` and the
timeout threshold in %o3 is `0x03f941f8`. Linux identifies configuration
offset `0xb0` as APB_TICK_REGISTER. Check its counter width, clock and
write semantics before implementing the timeout. The firmware's keyboard
reset byte at offset `0x5e0` is downstream of this first wait.
[Hardware and register references](../docs/ultra5.md#cold-start-keyboard-probe)

The soft-marker diagnostic and quiet runs stop at flash offset `0x780`
after 174,083 and 171,971 instructions respectively. All four bounded
transcripts are under `local/ultra5-keyboard-uart-2026-11-22/`.
Each matches the serial bytes from the E-cache checkpoint. Validation
passed 892 standard test executions, 15 explicit local-ROM checks,
formatting, all-target Clippy with warnings denied and all four demos.
