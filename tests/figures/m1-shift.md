# Figures after the shift of the whole drawing

Written once by the code of #266 (M1 step 9: a shift of the whole drawing, which the editor follows, is not a move;
notation 2.1): `cargo xtask figures`, on 2026-11-23, on the decision of the owner. The figures of stability are lower
than in `m1-core-blocks.md`, none higher: a node that kept its place on the screen no longer counts as moved (`read_voltage`
goes from 4.0 to 2.0, `wait_dut` from 10.0 to 5.4). They replace `m1-core-blocks.md` as the threshold of scenario 5
of `docs/acceptance-m1.md`, point 2: `cargo xtask figures --against tests/figures/m1-shift.md` fails when a figure
of stability of an example is worse now. Never written again from the code under test.

| Function | Nodes | Size | Zoom to fit | Wires probed | Moved | Column changed |
|---|---|---|---|---|---|---|
| `bench_selftest` | 2 | 406 × 172 | 100 % | 0 | – / 0 | – / 0 |
| `channel_voltage` | 1 | 406 × 416 | 100 % | 0 | – / 0 | – / 0 |
| `check_voltage` | 9 | 1046 × 392 | 100 % | 6 | 4.5 / 7 | 1.3 / 2 |
| `projectile` | 24 | 1334 × 640 | 87 % | 17 | 10.6 / 22 | 5.6 / 11 |
| `read_voltage` | 4 | 526 × 191 | 100 % | 1 | 2.0 / 2 | 2.0 / 2 |
| `run_step` | 7 | 1062 × 268 | 100 % | 6 | 2.5 / 4 | 1.0 / 2 |
| `settle` | 11 | 1038 × 360 | 100 % | 7 | 3.4 / 7 | 1.4 / 5 |
| `station` | 17 | 1392 × 1320 | 42 % | 9 | 3.8 / 10 | 1.4 / 4 |
| `two_benches` | 23 | 1238 × 672 | 83 % | 12 | 8.4 / 14 | 3.2 / 5 |
| `wait_dut` | 12 | 1190 × 312 | 100 % | 10 | 5.4 / 9 | 3.2 / 8 |
| `dmm_driver.read_dc_volts` | 2 | 398 × 160 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
| `modbus_tcp.read_from` | 3 | 456 × 232 | 100 % | 0 | – / 0 | – / 0 |
| `modbus_tcp.read_holding_registers` | 40 | 3000 × 632 | 44 % | 35 | 21.1 / 39 | 0.7 / 3 |
| `psu.set_voltage` | 3 | 462 × 180 | 100 % | 2 | 1.5 / 2 | 0.0 / 0 |
| `relays.apply` | 2 | 398 × 180 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
