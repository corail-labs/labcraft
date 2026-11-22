# Figures after the blocks of core

Written once by the code of the blocks of `core` (M1 step 9, the D of the primitives 16 × 32 and 16 × 16, `top 1`, the
conversions 24 × 16, the D cut 4 above and below from three places): `cargo xtask figures`, on 2026-11-22, on the
decision of the owner. Those sizes change the figures of stability by design: narrower primitives move the columns of
`projectile`, whose mean of columns changed goes from 5.3 to 5.6. They replace `m1-primitives.md` as the threshold of
scenario 5 of `docs/acceptance-m1.md`, point 2: `cargo xtask figures --against tests/figures/m1-core-blocks.md` fails
when a figure of stability of an example is worse now. Never written again from the code under test.

| Function | Nodes | Size | Zoom to fit | Wires probed | Moved | Column changed |
|---|---|---|---|---|---|---|
| `bench_selftest` | 2 | 406 × 172 | 100 % | 0 | – / 0 | – / 0 |
| `channel_voltage` | 1 | 406 × 416 | 100 % | 0 | – / 0 | – / 0 |
| `check_voltage` | 9 | 1046 × 392 | 100 % | 6 | 4.5 / 7 | 1.3 / 2 |
| `projectile` | 24 | 1334 × 640 | 87 % | 17 | 10.6 / 22 | 5.6 / 11 |
| `read_voltage` | 4 | 526 × 191 | 100 % | 1 | 4.0 / 4 | 2.0 / 2 |
| `run_step` | 7 | 1062 × 268 | 100 % | 6 | 3.5 / 7 | 1.0 / 2 |
| `settle` | 11 | 1038 × 360 | 100 % | 7 | 3.4 / 7 | 1.4 / 5 |
| `station` | 17 | 1392 × 1320 | 42 % | 9 | 4.4 / 12 | 1.4 / 4 |
| `two_benches` | 23 | 1238 × 672 | 83 % | 12 | 10.1 / 22 | 3.2 / 5 |
| `wait_dut` | 12 | 1190 × 312 | 100 % | 10 | 10.0 / 11 | 3.2 / 8 |
| `dmm_driver.read_dc_volts` | 2 | 398 × 160 | 100 % | 1 | 2.0 / 2 | 0.0 / 0 |
| `modbus_tcp.read_from` | 3 | 456 × 232 | 100 % | 0 | – / 0 | – / 0 |
| `modbus_tcp.read_holding_registers` | 40 | 3000 × 632 | 44 % | 35 | 21.9 / 40 | 0.7 / 3 |
| `psu.set_voltage` | 3 | 462 × 180 | 100 % | 2 | 1.5 / 2 | 0.0 / 0 |
| `relays.apply` | 2 | 398 × 180 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
