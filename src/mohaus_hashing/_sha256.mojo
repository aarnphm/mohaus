# Pure-Mojo SHA256 implementation.
#
# Specification: FIPS 180-4 (Section 6.2). The 32-bit word sizes, 64 rounds,
# and round-constant tables are exactly what the spec dictates. We use Mojo's
# native UInt32 arithmetic with explicit wrap-around so the byte sequence
# matches the Rust `sha2` crate digest for any input. Differential parity is
# pinned via `src/tests/test_hashing.mojo` against fixtures shared with the
# Rust crate.

from std.collections import List
from std.memory import Span


def _round_constant(i: Int) -> UInt32:
    var table = [
        UInt32(0x428A2F98),
        UInt32(0x71374491),
        UInt32(0xB5C0FBCF),
        UInt32(0xE9B5DBA5),
        UInt32(0x3956C25B),
        UInt32(0x59F111F1),
        UInt32(0x923F82A4),
        UInt32(0xAB1C5ED5),
        UInt32(0xD807AA98),
        UInt32(0x12835B01),
        UInt32(0x243185BE),
        UInt32(0x550C7DC3),
        UInt32(0x72BE5D74),
        UInt32(0x80DEB1FE),
        UInt32(0x9BDC06A7),
        UInt32(0xC19BF174),
        UInt32(0xE49B69C1),
        UInt32(0xEFBE4786),
        UInt32(0x0FC19DC6),
        UInt32(0x240CA1CC),
        UInt32(0x2DE92C6F),
        UInt32(0x4A7484AA),
        UInt32(0x5CB0A9DC),
        UInt32(0x76F988DA),
        UInt32(0x983E5152),
        UInt32(0xA831C66D),
        UInt32(0xB00327C8),
        UInt32(0xBF597FC7),
        UInt32(0xC6E00BF3),
        UInt32(0xD5A79147),
        UInt32(0x06CA6351),
        UInt32(0x14292967),
        UInt32(0x27B70A85),
        UInt32(0x2E1B2138),
        UInt32(0x4D2C6DFC),
        UInt32(0x53380D13),
        UInt32(0x650A7354),
        UInt32(0x766A0ABB),
        UInt32(0x81C2C92E),
        UInt32(0x92722C85),
        UInt32(0xA2BFE8A1),
        UInt32(0xA81A664B),
        UInt32(0xC24B8B70),
        UInt32(0xC76C51A3),
        UInt32(0xD192E819),
        UInt32(0xD6990624),
        UInt32(0xF40E3585),
        UInt32(0x106AA070),
        UInt32(0x19A4C116),
        UInt32(0x1E376C08),
        UInt32(0x2748774C),
        UInt32(0x34B0BCB5),
        UInt32(0x391C0CB3),
        UInt32(0x4ED8AA4A),
        UInt32(0x5B9CCA4F),
        UInt32(0x682E6FF3),
        UInt32(0x748F82EE),
        UInt32(0x78A5636F),
        UInt32(0x84C87814),
        UInt32(0x8CC70208),
        UInt32(0x90BEFFFA),
        UInt32(0xA4506CEB),
        UInt32(0xBEF9A3F7),
        UInt32(0xC67178F2),
    ]
    return table[i]


def _hex_digit(value: Int) -> String:
    var digits = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "a", "b", "c", "d", "e", "f"]
    return digits[value]


@fieldwise_init
struct Sha256(Movable):
    """Streaming SHA256 with `update`/`hexdigest` shape."""

    var _h0: UInt32
    var _h1: UInt32
    var _h2: UInt32
    var _h3: UInt32
    var _h4: UInt32
    var _h5: UInt32
    var _h6: UInt32
    var _h7: UInt32
    var _buffer: List[UInt8]
    var _length: UInt64

    @staticmethod
    def new() -> Self:
        return Sha256(
            UInt32(0x6A09E667),
            UInt32(0xBB67AE85),
            UInt32(0x3C6EF372),
            UInt32(0xA54FF53A),
            UInt32(0x510E527F),
            UInt32(0x9B05688C),
            UInt32(0x1F83D9AB),
            UInt32(0x5BE0CD19),
            List[UInt8](),
            UInt64(0),
        )

    def update(mut self, data: Span[mut=False, Byte, _]):
        var n = len(data)
        for i in range(n):
            self._buffer.append(data[i])
            if len(self._buffer) == 64:
                self._process_block()
                self._buffer = List[UInt8]()
        self._length += UInt64(n)

    def update_bytes(mut self, data: List[UInt8]):
        for i in range(len(data)):
            self._buffer.append(data[i])
            if len(self._buffer) == 64:
                self._process_block()
                self._buffer = List[UInt8]()
        self._length += UInt64(len(data))

    def update_string(mut self, value: String):
        var span = value.as_bytes()
        self.update(span)

    def finalize(mut self) -> List[UInt8]:
        var bit_length = self._length * UInt64(8)
        self._buffer.append(UInt8(0x80))
        while len(self._buffer) % 64 != 56:
            self._buffer.append(UInt8(0))
            if len(self._buffer) == 64:
                self._process_block()
                self._buffer = List[UInt8]()
        for i in range(8):
            var shift = UInt64(8 * (7 - i))
            self._buffer.append(UInt8((bit_length >> shift) & UInt64(0xFF)))
        self._process_block()
        self._buffer = List[UInt8]()

        var out = List[UInt8]()
        var words = [
            self._h0,
            self._h1,
            self._h2,
            self._h3,
            self._h4,
            self._h5,
            self._h6,
            self._h7,
        ]
        for word_index in range(8):
            var word = words[word_index]
            for byte_index in range(4):
                var shift = UInt32(8 * (3 - byte_index))
                out.append(UInt8((word >> shift) & UInt32(0xFF)))
        return out^

    def hexdigest(mut self) -> String:
        var digest = self.finalize()
        var result = String()
        for i in range(len(digest)):
            var byte = digest[i]
            result += _hex_digit(Int((byte >> UInt8(4)) & UInt8(0x0F)))
            result += _hex_digit(Int(byte & UInt8(0x0F)))
        return result

    def _process_block(mut self):
        var w = List[UInt32]()
        for i in range(16):
            var word = (
                (UInt32(self._buffer[i * 4]) << UInt32(24))
                | (UInt32(self._buffer[i * 4 + 1]) << UInt32(16))
                | (UInt32(self._buffer[i * 4 + 2]) << UInt32(8))
                | UInt32(self._buffer[i * 4 + 3])
            )
            w.append(word)
        for i in range(16, 64):
            var s0 = _rotr(w[i - 15], UInt32(7)) ^ _rotr(w[i - 15], UInt32(18)) ^ (w[i - 15] >> UInt32(3))
            var s1 = _rotr(w[i - 2], UInt32(17)) ^ _rotr(w[i - 2], UInt32(19)) ^ (w[i - 2] >> UInt32(10))
            w.append(w[i - 16] + s0 + w[i - 7] + s1)

        var a = self._h0
        var b = self._h1
        var c = self._h2
        var d = self._h3
        var e = self._h4
        var f = self._h5
        var g = self._h6
        var h = self._h7

        for i in range(64):
            var big_s1 = _rotr(e, UInt32(6)) ^ _rotr(e, UInt32(11)) ^ _rotr(e, UInt32(25))
            var ch = (e & f) ^ ((~e) & g)
            var t1 = h + big_s1 + ch + _round_constant(i) + w[i]
            var big_s0 = _rotr(a, UInt32(2)) ^ _rotr(a, UInt32(13)) ^ _rotr(a, UInt32(22))
            var maj = (a & b) ^ (a & c) ^ (b & c)
            var t2 = big_s0 + maj
            h = g
            g = f
            f = e
            e = d + t1
            d = c
            c = b
            b = a
            a = t1 + t2

        self._h0 += a
        self._h1 += b
        self._h2 += c
        self._h3 += d
        self._h4 += e
        self._h5 += f
        self._h6 += g
        self._h7 += h


def _rotr(value: UInt32, amount: UInt32) -> UInt32:
    return (value >> amount) | (value << (UInt32(32) - amount))


def sha256_hex(data: Span[mut=False, Byte, _]) -> String:
    var hasher = Sha256.new()
    hasher.update(data)
    return hasher.hexdigest()


def sha256_hex_string(value: String) -> String:
    var hasher = Sha256.new()
    hasher.update_string(value)
    return hasher.hexdigest()
