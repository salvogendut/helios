# Ultra 5/10 first firmware fetch

Helios reads the first instruction from the original Ultra 5/10 OBP 3.31.0 flash at power-on. This headless fixture stops after the read, before instruction execution.

## Reset and flash mapping

The UltraSPARC-IIi enters RED_state at the default virtual reset vector plus `0x20`. PSTATE.AM is clear, so PC and nPC retain all 64 bits. The instruction-side physical read uses PA[40:0]. The board has 1 MiB of flash divided equally between OBP and POST/OBDiag. [UltraSPARC IIi User’s Manual, §17.3.2 and tables 17-3/17-5](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=303) · [Sun Ultra 5 Service Manual, §C.1.6.3](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=224)

| Quantity | Value |
| --- | --- |
| Default virtual RED_state vector base | `0xffff_ffff_f000_0000` |
| Architectural PC / nPC | `0xffff_ffff_f000_0020` / `0xffff_ffff_f000_0024` |
| Physical flash read | `0x01ff_f000_0020` |
| Offset in the 1 MiB flash | `0x20` |
| Big-endian instruction | `0xc4d808a0` (`ldxa [%g0] #ASI_LSU_CONTROL_REG, %g2`) |

## Reconstruct and test the original firmware

Use the `flash-update-Ultra510-18` utility from Sun patch 106121-18. The extractor checks the updater SHA-256 and follows both board descriptors to the OBP and POST records. For each board it takes exactly `0x80000` bytes per half, including the `OBMD` headers, then joins OBP followed by POST. Both descriptors must yield identical bytes and the expected image digest.

| Board | OBP file offset | POST file offset |
| --- | --- | --- |
| `SUNW,375-0009` | `0x5f078` | `0x15f0b8` |
| `SUNW,375-0066` | `0xdf098` | `0x1df0d8` |

| Artifact | SHA-256 |
| --- | --- |
| `106121-18.zip` | `21561476a550286a0fc28fd331f231b59f9db2ab774779bb3aec164d02d8153f` |
| `flash-update-Ultra510-18` | `e0ef541b64de9ab23b9423fce71fd56a7ddcc29cb28de8b66a4313d48516a29e` |
| Reconstructed 1 MiB image | `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616` |

From the repository root, run the extractor and the explicit local-ROM test. The extractor writes a JSON manifest next to the image. The test uses Python 3 to verify the full image digest before checking the first fetch.

```sh
python3 tools/ultra5-fixture/extract.py \
  local/firmware/ultra5/106121-18/flash-update-Ultra510-18 \
  --output local/firmware/ultra5/ultra5-obp331-post310-1mb.bin
cargo test --locked --offline --test ultra5_first_fetch -- --ignored --nocapture
```

## QEMU comparison

QEMU 10.2.2, paused at startup with `-S -M sun4u -cpu TI-UltraSparc-IIi` and the reconstructed image, gives the following monitor results.

```text
(qemu) xp /1wx 0x1fff0000020
000001fff0000020: 0xc4d808a0
(qemu) x /1i 0x1fff0000020
0x1fff0000020:  ldxa  [ %g0 ] #ASI_LSU_CONTROL_REG, %g2
```

QEMU’s generic sun4u machine reports PC/nPC in the low physical range. This comparison verifies the bytes and disassembly. The Sun reset table supplies the architectural PC/nPC used by Helios.
