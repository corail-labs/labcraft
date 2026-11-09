"""Expected values of the tests of `labcraft-visa` (libs/visa/src/tests.rs), worked out from libs/visa/simulation.md,
section 3, alone: SplitMix64, the twelve draws, the ramp. Throwaway, as all of tools/: `python tools/visa-expected/expected.py`."""
MASK = (1 << 64) - 1


class Gen:
    def __init__(self, seed):
        self.s = seed

    def draw(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & MASK
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def g(self):
        s = sum(self.draw() >> 11 for _ in range(12))
        return float(s - 6 * (1 << 53)) * 2.0 ** -53


def true(initial, value, start, settle, t):
    if t < start:
        return initial
    if settle <= 0 or t - start >= settle:
        return value
    return initial + (value - initial) * (float(t - start) / float(settle))


g1 = Gen(1)
a, b, c = g1.g(), g1.g(), g1.g()
print("seed1 g1", repr(a), "g2", repr(b), "g3", repr(c))
print("dmm230 reads", "%.6f" % (230.0 + 0.05 * a), "%.6f" % (230.0 + 0.05 * b))
g = Gen(1)
x0 = true(0.0, 230.0, 0, 300_000_000, 0) + 0.05 * g.g()
x1 = true(0.0, 230.0, 0, 300_000_000, 150_000_000) + 0.05 * g.g()
x2 = true(0.0, 230.0, 0, 300_000_000, 300_000_000) + 0.05 * g.g()
print("settle300 at 0, 150ms, 300ms", "%.6f" % x0, "%.6f" % x1, "%.6f" % x2)
g2 = Gen(2)
print("seed2 g1", repr(g2.g()))
gv, gc = Gen(1), Gen(2)
print("psu1W5", "%.6f" % (5.02 + 0.002 * gv.g()), "%.6f" % (0.30 + 0.001 * gc.g()))
# bench: PSU0 VOLT 5 written at t=5ms (latency 5 ms); DMM0 reads source at t=10ms (its own latency, after), seed 7.
g7 = Gen(7)
v = true(0.0, 5.0, 5_000_000, 300_000_000, 10_000_000)
print("bench dmm at 10ms after VOLT 5 at 5ms", "%.6f" % (v + 0.05 * g7.g()), "true", repr(v))
print("-0 format", "%.6f" % (-0.0), "%.6f" % (-1e-9))
