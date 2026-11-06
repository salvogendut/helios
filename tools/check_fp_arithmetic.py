#!/usr/bin/env python3
"""Check guest arithmetic using exact Fractions and adjacent IEEE values.

Build: cargo build --locked --offline --example fp_arithmetic_probe
Run:   python3 tools/check_fp_arithmetic.py target/debug/examples/fp_arithmetic_probe
No host floating-point arithmetic, third-party packages or firmware are needed.
This oracle searches representable neighbors instead of using significand
alignment/division as the Rust implementation does. Seed and sample count are recorded.
"""
import argparse
from fractions import Fraction
import json
import random
import subprocess


class Format:
    def __init__(self, width):
        self.width = width
        self.fraction = 23 if width == 32 else 52
        self.bias = 127 if width == 32 else 1023
        self.sign = 1 << (width - 1)
        self.inf = self.sign - (1 << self.fraction)
        self.quiet = 1 << (self.fraction - 1)
        self.meiko_nan = 0x7FFF0000 if width == 32 else 0x7FFFE00000000000
        self.minimum = self.value(1 << self.fraction)
        self.limit = Fraction(1 << (self.bias + 1))

    def value(self, bits):
        """Positive encoding, with infinity used as the finite rounding boundary."""
        exponent, fraction = divmod(bits, 1 << self.fraction)
        significand = fraction if exponent == 0 else fraction + (1 << self.fraction)
        power = max(exponent, 1) - self.bias - self.fraction
        return Fraction(significand << power) if power >= 0 else Fraction(significand, 1 << -power)

    def result(self, a, b, rounding, uf_enabled, operation):
        sign_a, sign_b = bool(a & self.sign), bool(b & self.sign)
        sign = (a ^ b) & self.sign
        a, b = a & (self.sign - 1), b & (self.sign - 1)
        if a > self.inf or b > self.inf:
            if (a > self.inf and not a & self.quiet) or (b > self.inf and not b & self.quiet):
                return self.meiko_nan, 16
            return (a if a > self.inf else b), 0
        if operation == "div":
            if (a == b == 0) or (a == b == self.inf):
                return self.sign - 1, 16
            if a == self.inf:
                return sign | self.inf, 0
            if b == self.inf:
                return sign, 0
            if b == 0:
                return sign | self.inf, 2
            if a == 0:
                return sign, 0
            exact = self.value(a) / self.value(b)
        elif operation == "mul":
            if (a == 0 and b == self.inf) or (a == self.inf and b == 0):
                return self.sign - 1, 16
            if a == self.inf or b == self.inf:
                return sign | self.inf, 0
            if a == 0 or b == 0:
                return sign, 0
            exact = self.value(a) * self.value(b)
        else:
            sign_b ^= operation == "sub"
            if a == self.inf or b == self.inf:
                if a == b and sign_a != sign_b:
                    return self.sign - 1, 16
                negative = sign_a if a == self.inf else sign_b
                return self.inf | (self.sign if negative else 0), 0
            exact = (-1 if sign_a else 1) * self.value(a)
            exact += (-1 if sign_b else 1) * self.value(b)
            if exact == 0:
                negative = sign_a if sign_a == sign_b else rounding == 3
                return self.sign if negative else 0, 0
            sign = self.sign if exact < 0 else 0
            exact = abs(exact)
        directed_up = (rounding == 2 and not sign) or (rounding == 3 and bool(sign))
        if exact >= self.limit:
            return sign | (self.inf if rounding == 0 or directed_up else self.inf - 1), 9
        # Binary search all positive encodings, including subnormals. The upper
        # infinity encoding denotes 2^(emax+1), the next unbounded-exponent value.
        low, high = 0, self.inf
        while low + 1 < high:
            middle = (low + high) // 2
            if self.value(middle) <= exact:
                low = middle
            else:
                high = middle
        lower, upper = self.value(low), self.value(high)
        inexact = lower != exact
        rounded = low
        if inexact:
            if rounding == 0:
                below, above = exact - lower, upper - exact
                if above < below or (above == below and low & 1):
                    rounded = high
            elif directed_up:
                rounded = high
        if rounded == self.inf:
            return sign | self.inf, 9
        tiny = 0 < exact < self.minimum
        flags = int(inexact) | (4 if tiny and (inexact or uf_enabled) else 0)
        return sign | rounded, flags

    def expected(self, a, b, rounding, mask, operation):
        result, flags = self.result(a, b, rounding, bool(mask & 4), operation)
        enabled = flags & mask
        status = (rounding << 30) | (mask << 23) | 0x80000 | 0xC60
        if enabled:
            return (0xDEC0ADDE, 0xA5A55A5A, status | 0x6000 | (1 << (enabled.bit_length() - 1)), 1)
        first, second = (result >> 32, result & 0xFFFFFFFF) if self.width == 64 else (result, 0xA5A55A5A)
        return first, second, status | flags | (flags << 5), 0

    def cases(self, rng, samples, operation):
        hidden = 1 << self.fraction
        one, two, three = self.bias * hidden, (self.bias + 1) * hidden, (self.bias + 1) * hidden + hidden // 2
        specials = [0, self.sign, 1, hidden, one, self.inf, self.sign | self.inf,
                    self.inf | self.quiet | 7, self.sign | self.inf | self.quiet | 17,
                    self.inf | 1, self.sign | self.inf | 3]
        pairs = [(a, b) for a in specials for b in specials]
        # Rounding ties, very tiny quotients, min-normal crossover, overflow edges.
        pairs += [(1,two),(3,two),(hidden,three),(hidden,one+1),(hidden-1,one-1),
                  (hidden*2-1,two),(self.inf-1,one-1),(self.inf-1,one),
                  (self.inf-1,two),(self.inf-1,1),(1,self.inf-1),
                  (one,three),(two,three),(one,one+1),(one+1,one-1)]
        if operation == "mul":
            half = (self.bias - 1) * hidden
            one_and_half = one + hidden // 2
            pairs += [(1, half), (3, half), (hidden, half), (hidden * 2 - 1, half),
                      (hidden, one - 1), (hidden, one + 1), (hidden - 1, two),
                      (1, one + 1), (self.inf - 1, two), (self.inf - 2, one + 1),
                      (self.inf - 1, self.inf - 1), (1, 1),
                      (one + 1, one_and_half), (one + 3, one_and_half),
                      (one + 1, one_and_half - 1), (one + 1, one_and_half + 1),
                      (one + 1, one + 1), (one - 1, one - 1), (two - 1, two - 1)]
        if operation in ("add", "sub"):
            maximum = self.inf - 1
            half_ulp = (self.bias - self.fraction - 1) * hidden
            overflow_half = (self.bias * 2 - self.fraction - 1) * hidden
            pairs += [(one, half_ulp), (one + 1, half_ulp),
                      (one, half_ulp - 1), (one, half_ulp + 1),
                      (maximum, 1), (maximum, overflow_half),
                      (maximum, overflow_half - 1), (maximum, overflow_half + 1),
                      (maximum, maximum), (hidden, hidden - 1),
                      (hidden, self.sign | (hidden - 1)), (hidden - 1, 1)]
            # Force every alignment boundary, including shifts beyond u64 width.
            for gap in [0, 1, 2, 3, 4, self.fraction - 1, self.fraction,
                        self.fraction + 1, self.fraction + 2, 63, 64, 65, 127]:
                small = max(1, (self.bias - gap) * hidden + 7)
                pairs += [(one, small), (one, self.sign | small),
                          (small, one), (small, self.sign | one)]
        for a,b in pairs:
            for sign in [0,self.sign]:
                for rounding in range(4):
                    for mask in [0,1,2,4,8,16,31]:
                        yield a ^ sign, b, rounding, mask
        for sample in range(samples):
            a,b = rng.getrandbits(self.width), rng.getrandbits(self.width)
            if operation in ("add", "sub") and sample % 2 == 0:
                # Nearby magnitudes expose cancellation missed by uniform bit patterns.
                magnitude = min(a & (self.sign - 1), self.inf - 1)
                b = (b & self.sign) | max(0, min(self.inf - 1, magnitude + rng.randrange(-16, 17)))
            for rounding in range(4):
                for mask in [0,31]:
                    yield a,b,rounding,mask


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('probe', help='built fp_arithmetic_probe executable')
    parser.add_argument('--samples', type=int, default=1024, help='random operand pairs per precision')
    parser.add_argument('--seed', type=lambda s: int(s,0), default=0x535335)
    parser.add_argument('--operations', nargs='+', choices=['div', 'add', 'sub', 'mul'],
                        default=['div', 'add', 'sub', 'mul'])
    args = parser.parse_args()
    if args.samples < 0:
        parser.error('samples must be nonnegative')
    for operation in args.operations:
        rng = random.Random(args.seed)
        inputs, expected = [], []
        for width in [32,64]:
            fmt = Format(width)
            for a,b,rounding,mask in fmt.cases(rng,args.samples,operation):
                inputs.append(f'{width} {a:x} {b:x} {rounding:x} {mask:x}')
                expected.append(fmt.expected(a,b,rounding,mask,operation))
        process = subprocess.run([args.probe, operation], input='\n'.join(inputs)+'\n', text=True,
                                 capture_output=True, check=True)
        outputs = process.stdout.splitlines()
        if len(outputs) != len(expected):
            raise AssertionError(f'expected {len(expected)} results, got {len(outputs)}')
        for case, want, line in zip(inputs, expected, outputs):
            actual = tuple(int(field,16) for field in line.split())
            if actual != want:
                raise AssertionError(f'{operation} {case}: expected {want!r}, got {actual!r}')
        print(json.dumps({'operation':operation, 'seed':hex(args.seed),
                          'random_pairs_per_precision':args.samples,
                          'checked_guest_operations':len(expected),'result':'pass'}), flush=True)


if __name__ == '__main__':
    main()
