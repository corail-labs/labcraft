# Figures after the wires between neighbours are straightened

Written once by the code of #279 (M1 step 9: the wire into the first input that a node feeds is straightened while the
body grows by 48 at most, and a track takes the height of its readers when that one is free; notation 2.1 and 2.2):
`cargo xtask figures`, on 2026-11-23, on the decision of the owner, who accepted figures of stability worse than in
`m1-shift.md` for a drawing that reads better: a neutral node put on a wire is now aligned on what feeds it, and what
it feeds on it. Moved (mean): `projectile` from 10.6 to 11.7, `station` from 3.8 to 5.8 (most from 10 to 12),
`wait_dut` from 5.4 to 5.8, `settle` from 3.4 to 4.0; better: `two_benches` from 8.4 to 8.3, `read_holding_registers`
from 21.1 to 20.1; the others equal. They replace `m1-shift.md` as the threshold of scenario 5 of
`docs/acceptance-m1.md`, point 2: `cargo xtask figures --against tests/figures/m1-straight.md` fails when a figure of
stability of an example is worse now. Never written again from the code under test.

| Function | Nodes | Size | Zoom to fit | Wires probed | Moved | Column changed |
|---|---|---|---|---|---|---|
| `bench_selftest` | 2 | 406 × 172 | 100 % | 0 | – / 0 | – / 0 |
| `channel_voltage` | 1 | 406 × 416 | 100 % | 0 | – / 0 | – / 0 |
| `check_voltage` | 9 | 1046 × 392 | 100 % | 6 | 4.5 / 7 | 1.3 / 2 |
| `projectile` | 24 | 1278 × 640 | 87 % | 17 | 11.7 / 22 | 5.6 / 11 |
| `read_voltage` | 4 | 526 × 191 | 100 % | 1 | 2.0 / 2 | 2.0 / 2 |
| `run_step` | 7 | 1078 × 268 | 100 % | 6 | 2.5 / 4 | 1.0 / 2 |
| `settle` | 11 | 1022 × 376 | 100 % | 7 | 4.0 / 7 | 1.4 / 5 |
| `station` | 17 | 1384 × 1328 | 42 % | 9 | 5.8 / 12 | 1.4 / 4 |
| `two_benches` | 23 | 1174 × 672 | 83 % | 12 | 8.3 / 14 | 3.2 / 5 |
| `wait_dut` | 12 | 1190 × 344 | 100 % | 10 | 5.8 / 9 | 3.2 / 8 |
| `dmm_driver.read_dc_volts` | 2 | 398 × 160 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
| `modbus_tcp.read_from` | 3 | 456 × 232 | 100 % | 0 | – / 0 | – / 0 |
| `modbus_tcp.read_holding_registers` | 40 | 2960 × 632 | 45 % | 35 | 20.1 / 39 | 0.7 / 3 |
| `psu.set_voltage` | 3 | 462 × 180 | 100 % | 2 | 1.5 / 2 | 0.0 / 0 |
| `relays.apply` | 2 | 398 × 180 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
