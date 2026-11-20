# Figures after the primitives of 24

Written once by the code of #264 (`1e03b74`, M1 step 6, the D of the primitives 24 × 32 and 24 × 16): `cargo xtask
figures`, on 2026-11-20, on the decision of the owner. Those sizes change the figures of stability by design: the neutral
node `core.select(true, v, v)`, 32 high, now stands higher than a primitive of one value in, 16 high, so that the
frame grows upward and the whole drawing moves down. They replace
`v0.3.0-m0.md` as the threshold of scenario 5 of `docs/acceptance-m1.md`, point 2: `cargo xtask figures --against
tests/figures/m1-primitives.md` fails when a figure of stability of an example is worse now. Never written again from
the code under test.

| Function | Nodes | Size | Zoom to fit | Wires probed | Moved | Column changed |
|---|---|---|---|---|---|---|
| `bench_selftest` | 2 | 406 × 172 | 100 % | 0 | – / 0 | – / 0 |
| `channel_voltage` | 1 | 406 × 416 | 100 % | 0 | – / 0 | – / 0 |
| `check_voltage` | 9 | 1062 × 392 | 100 % | 6 | 4.5 / 7 | 1.3 / 2 |
| `projectile` | 24 | 1422 × 640 | 87 % | 17 | 10.6 / 22 | 5.3 / 11 |
| `read_voltage` | 4 | 526 × 191 | 100 % | 1 | 4.0 / 4 | 2.0 / 2 |
| `run_step` | 7 | 1078 × 268 | 100 % | 6 | 3.5 / 7 | 1.0 / 2 |
| `settle` | 11 | 1062 × 360 | 100 % | 7 | 3.4 / 7 | 1.4 / 5 |
| `station` | 17 | 1400 × 1316 | 42 % | 9 | 4.4 / 12 | 1.4 / 4 |
| `two_benches` | 23 | 1246 × 668 | 83 % | 12 | 10.1 / 22 | 3.2 / 5 |
| `wait_dut` | 12 | 1206 × 308 | 100 % | 10 | 10.0 / 11 | 3.2 / 8 |
| `dmm_driver.read_dc_volts` | 2 | 398 × 160 | 100 % | 1 | 2.0 / 2 | 0.0 / 0 |
| `psu.set_voltage` | 3 | 462 × 180 | 100 % | 2 | 1.5 / 2 | 0.0 / 0 |
| `relays.apply` | 2 | 398 × 180 | 100 % | 1 | 1.0 / 1 | 0.0 / 0 |
