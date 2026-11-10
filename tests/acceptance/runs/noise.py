"""The readings of a simulated quantity, worked out from libs/visa/simulation.md, sections 3 and 4, alone.

It gives the expected values of the cases of this corpus that read a noisy quantity: the answers of the
instrument, the values the program reads from them, and a mean made as `two_benches` makes it, a sum in the
order of the readings, then a division. It imports nothing of LabCraft or of the proof of concept.

    python tests/acceptance/runs/noise.py SEED VALUE NOISE COUNT
"""

import sys
from decimal import ROUND_HALF_EVEN, Decimal

MASK = (1 << 64) - 1


def splitmix64(seed):
    """Section 3, point 1: the draws of the generator started at `seed`."""
    state = seed & MASK
    while True:
        state = (state + 0x9E3779B97F4A7C15) & MASK
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        yield z ^ (z >> 31)


def factors(seed):
    """Section 3, point 2: 12 draws per read, their top 53 bits summed, centred, scaled by 2^-53 (exact)."""
    draws = splitmix64(seed)
    while True:
        s = sum(next(draws) >> 11 for _ in range(12))
        yield float(s - 6 * 2**53) * 2.0**-53


def answer(value):
    """Section 4, point 4: the exact value rounded to 6 decimals, ties to even, `-0.000000` included."""
    # `Decimal` of a float is its exact value, and keeps the sign of -0.0.
    return str(Decimal(value).quantize(Decimal("0.000001"), rounding=ROUND_HALF_EVEN))


def readings(seed, value, noise, count):
    """Section 3, point 3: `true + noise × g`, a product then a sum, each rounded; the true value is constant here."""
    out = []
    for g, _ in zip(factors(seed), range(count)):
        read = value + noise * g
        text = answer(read)
        out.append((text, float(text)))
    return out


def main(argv):
    seed, value, noise, count = int(argv[0]), float(argv[1]), float(argv[2]), int(argv[3])
    total = 0.0
    for index, (text, number) in enumerate(readings(seed, value, noise, count)):
        total = total + number
        print(f"{index}: answer {text}, read {number!r}")
    print(f"mean of {count}: {total / float(count)!r}")


if __name__ == "__main__":
    main(sys.argv[1:])
