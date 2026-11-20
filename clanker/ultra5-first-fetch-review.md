# Ultra 5 first-fetch review

**Review of the initial submission on `fb49784`.** The updater and archive hashes checked out. Both the original test and QEMU consumed the same incorrect slice, and the fixture assigned the physical flash address to the architectural PC. The [corrected checkpoint](ultra5-first-fetch.md) records the subsequent implementation and validation on the V9 base.

## Extract the flash halves from Sun patch 106121-18

The ELF data segment maps file offset `0x39a98` to VA `0x449a98`. The updater contains two board descriptors with pointers to a board name, an OBP record, a flash size of `0x100000`, and a POST record. Resolving these pointers through the file-backed ELF segment gives the following offsets.

| Board descriptor | File offset of board-name pointer | OBP offset | POST offset |
| --- | --- | --- | --- |
| SUNW,375-0009 | `0x59608` | `0x5f078` | `0x15f0b8` |
| SUNW,375-0066 | `0x59630` | `0xdf098` | `0x1df0d8` |

Take exactly `0x80000` bytes at each pointer, including the `OBMD` header, in OBP then POST order. Each board yields the same bytes. The image SHA-256 is `fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616`. This is a reconstruction from the vendor updater. The board manual independently describes the two flash halves. [Sun Ultra 5 Service Manual, §C.1.6.3](https://docs.oracle.com/cd/E19127-01/ultra5.ws/805-7763-12/805-7763-12.pdf#page=224)

The submitted start at `0xe0000` is `0xf68` bytes into the second OBP copy. Its apparent reset word `0x053c5c98` is at OBP offset `0xf88`. Searching for banners also succeeds on this incorrect slice. Update the extractor and manifest to record both half offsets, validated headers, size and digest. Continue to keep extracted firmware under ignored `local/`.

## Preserve both addresses on reset

| Observed quantity | Expected |
| --- | --- |
| Architectural PC | `0xffff_ffff_f000_0020` |
| Architectural nPC | `0xffff_ffff_f000_0024` |
| Physical instruction read, PA[40:0] | `0x01ff_f000_0020` |
| Flash offset | `0x20` |
| Big-endian word | `0xc4d808a0` |

The IIi reset table specifies the virtual vector and its physical equivalent. PSTATE.AM is clear. Keep the full virtual PC/nPC in the CPU and use the equivalent physical address for the flash read. Paused QEMU 10.2.2 with `-M sun4u -cpu TI-UltraSparc-IIi` read the word above and decoded `ldxa [%g0] #ASI_LSU_CONTROL_REG, %g2`. QEMU's generic machine reports the low physical value in its PC register. Use its disassembly to check bytes and Sun's table to check architectural PC/nPC. [Sun UltraSPARC IIi User’s Manual, §17.3.2 and tables 17-3/17-5](https://bebop.cs.berkeley.edu/resources/arch-manuals/ultra-IIi/ultra2i-manual-805-0087.pdf#page=303)

## Return for acceptance after first fetch

1. Preserve the dirty branch and compose it with accepted V9 commit `cb3468b`. The submitted branch starts at `fb49784`. In a temporary checkout the combined, **as-submitted** suites passed 832 tests with one firmware test ignored. The reported 817 excludes the 15 V9 tests.
2. Correct the extractor, core and existing tests. Assert literal virtual and physical expectations independently and check the complete reconstructed image digest. Replace the stale expected word, manifest and first-fetch/QEMU evidence in the docs and handoff.
3. Run the combined normal suite and the explicit local-ROM test. Check formatting, Clippy and the four demos. Stop at the first read. Implementing the fetched instruction belongs to the next slice.

The local evidence directory `local/review-ultra5-first-fetch-2026-11-20/` contains the read-only `check_image.py` verifier, an example core/test patch, three review fixtures, and the paused QEMU output. The fixtures failed 3/3 against the submission and passed 3/3 in the isolated correction. All four original first-fetch tests also passed there, including the local-ROM test against the reconstruction. The example patch covers the Rust files only.
