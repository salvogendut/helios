# Issue #5: typed parking-device digest

Status: accepted typed-digest checkpoint, committed as Helios `939a7bc`;
Heliacal remains `8e8d82a`. Baseline was Helios `316ea8a`; issue #5 remains
open. See the [review](performance-issue5-typed-digest-review.md). No caching, proof relaxation, wait change or
guest semantic change was made. The 2 ms cap, admission order, BREAK/input/
network checks, default-off parking and SMP exclusion are unchanged.

## Change

The converted device models now use allocation-free `Fingerprint`/`Digest`
encoding instead of Debug strings. Every previously fingerprinted field in
those models is encoded, including:

- AuxIO, CS4231, ECC and floppy registers, queues and motion phases;
- IOMMU control, table, slots, flush queue and async/locked state;
- NVRAM bytes and clock fields;
- both SCC ports, receive/transmit queues, break latches and epochs;
- keyboard, mouse, interrupt banks and timer bank state;
- reset, SCSI DMA, Ethernet DMA and Ethernet CSR/ring/packet/phase state;
- ESP and SCSI diagnostic payload digests (kept as their existing bounded
  fingerprints);
- RTC phase, video generations and MXCC data/registers.

Sequence lengths, option tags and enum discriminants are encoded explicitly.
The old Debug encoding is retained only as the `#[cfg(test)]`
`devices_digest_reference` oracle. A tracked mutation fixture covers each
device class and asserts both the reference and typed digest change; two token
fixtures confirm plain-device and staged-serial mutations invalidate tokens.
MXCC Debug hashing in diagnostic state was also replaced by the typed encoder.
The unchanged ESP/SCSI subdigests still perform Debug formatting: ESP formats
its controller state, and attached SCSI targets format their kind and media
information. The complete parking digest therefore still allocates; this
slice removes formatting from the converted models while preserving those
existing subdigests.

## Measurements

Fixed core token work: 20,000 successful token+advance cycles (two digests per
cycle, no explicit frontend validation), eight alternating release pairs.

| Arm | Median wall | Median worker CPU |
| --- | ---: | ---: |
| Checkpoint `316ea8a` | 4.511017 s | 4.490 s |
| Typed digest | 0.464657 s | 0.460 s |
| Change | **-89.70%** | **-89.76%** |

Matched frontend runs without profiling, parking enabled, two runs per arm,
two 5-second idle windows and 30 command trials per command phase:

Latency entries measure host write to the second output token. Each table
entry summarizes the two runs' per-window median or p95, rather than pooling
all samples into one percentile.

| Phase | Checkpoint | Typed digest |
| --- | ---: | ---: |
| Pre-command idle CPU | 44.08% median (39.18/48.98) | 16.19% median (8.79/23.59) |
| Post-command idle CPU | 30.39% median (35.59/25.19) | 45.58% median (55.57/35.58) |
| Command CPU | 65.37% median | 65.82% median |
| First command median / p95 | 32.56 / 41.57 ms | 28.19 / 35.04 ms |
| Post-command median / p95 | 21.40 / 31.43 ms | 21.46 / 28.87 ms |

The fixed token result is decisive. Frontend results are mixed: pre-command
idle CPU and first-window output latency improve, post-command idle CPU rises,
and command CPU is effectively unchanged. These measurements do not establish
why the later idle window consumes more CPU; guest-work and host variation
remain possible explanations. No broad end-to-end speedup is claimed from the
digest result alone.

## Validation

- Core successful executions: 783 default, 794 `parking-profile`, 797
  `parking-profile,perf-counters`; formatting, Clippy and all four demos pass.
- Frontend: 114 default / 117 profiling tests, formatting, both Clippy
  configurations, SDL smoke and SS-20 OBP boot smoke pass.
- The nine review fixtures plus the two token mutation fixtures pass in the
  profiling configuration; the default build retains the ordinary parking
  tests and the mutation oracle.

Evidence: `local/issue5-typed-digest-2026-11-18/` (`summary.json`,
`token-bench.json`, `measure.py`, benchmark source/runner, logs and binary
hashes). Temporary copies and builds were removed after the runs.
