#!/usr/bin/env python3
"""An independent reference for the pairing bytes of link revision 1.

It shares no code with the link crate: ristretto255 (RFC 9496), CPace
(draft-irtf-cfrg-cpace-21), HKDF, HMAC and ChaCha20-Poly1305 (RFC 8439) are
written out here over Python's integers and hashlib. It first reproduces the
published vectors of those documents, then prints climon2's own frozen
vectors, which `tests/pairing_vectors.rs` holds as literals. Run it to see
where those literals came from; nothing runs it automatically.
"""

import hashlib
import hmac
import struct

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, -1, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def is_negative(x):
    return x % P & 1 == 1


def absolute(x):
    return -x % P if is_negative(x) else x % P


def sqrt_ratio_m1(u, v):
    v3 = v * v * v % P
    v7 = v3 * v3 * v % P
    r = u * v3 * pow(u * v7, (P - 5) // 8, P) % P
    check = v * r * r % P
    correct = check == u % P
    flipped = check == -u % P
    flipped_i = check == -u * SQRT_M1 % P
    if flipped or flipped_i:
        r = SQRT_M1 * r % P
    return correct or flipped, absolute(r)


INVSQRT_A_MINUS_D = sqrt_ratio_m1(1, (-1 - D) % P)[1]
ONE_MINUS_D_SQ = (1 - D * D) % P
D_MINUS_ONE_SQ = (D - 1) * (D - 1) % P
SQRT_AD_MINUS_ONE = 25063068953384623474111414158702152701244531502492656460079210482610430750235
assert SQRT_AD_MINUS_ONE * SQRT_AD_MINUS_ONE % P == (-D - 1) % P


def decode(data):
    s = int.from_bytes(data, "little")
    if len(data) != 32 or s >= P or is_negative(s):
        return None
    ss = s * s % P
    u1, u2 = (1 - ss) % P, (1 + ss) % P
    u2_sqr = u2 * u2 % P
    v = (-(D * u1 * u1) - u2_sqr) % P
    was_square, invsqrt = sqrt_ratio_m1(1, v * u2_sqr % P)
    den_x = invsqrt * u2 % P
    den_y = invsqrt * den_x * v % P
    x = absolute(2 * s * den_x)
    y = u1 * den_y % P
    t = x * y % P
    if not was_square or is_negative(t) or y == 0:
        return None
    return (x, y, 1, t)


def encode(point):
    x0, y0, z0, t0 = point
    u1 = (z0 + y0) * (z0 - y0) % P
    u2 = x0 * y0 % P
    _, invsqrt = sqrt_ratio_m1(1, u1 * u2 * u2 % P)
    den1, den2 = invsqrt * u1 % P, invsqrt * u2 % P
    z_inv = den1 * den2 * t0 % P
    if is_negative(t0 * z_inv):
        x, y, den_inv = y0 * SQRT_M1 % P, x0 * SQRT_M1 % P, den1 * INVSQRT_A_MINUS_D % P
    else:
        x, y, den_inv = x0, y0, den2
    if is_negative(x * z_inv):
        y = -y % P
    return absolute(den_inv * (z0 - y)).to_bytes(32, "little")


def add(a, b):
    x1, y1, z1, t1 = a
    x2, y2, z2, t2 = b
    aa = (y1 - x1) * (y2 - x2) % P
    bb = (y1 + x1) * (y2 + x2) % P
    cc = 2 * D * t1 * t2 % P
    dd = 2 * z1 * z2 % P
    e, f, g, h = bb - aa, dd - cc, dd + cc, bb + aa
    return (e * f % P, g * h % P, f * g % P, e * h % P)


IDENTITY = (0, 1, 1, 0)


def multiply(scalar, point):
    result = IDENTITY
    while scalar:
        if scalar & 1:
            result = add(result, point)
        point = add(point, point)
        scalar >>= 1
    return result


def elligator(t):
    r = SQRT_M1 * t * t % P
    u = (r + 1) * ONE_MINUS_D_SQ % P
    v = (-1 - r * D) * (r + D) % P
    was_square, s = sqrt_ratio_m1(u, v)
    c = -1
    if not was_square:
        s = -absolute(s * t) % P
        c = r
    n = (c * (r - 1) * D_MINUS_ONE_SQ - v) % P
    w0 = 2 * s * v % P
    w1 = n * SQRT_AD_MINUS_ONE % P
    w2 = (1 - s * s) % P
    w3 = (1 + s * s) % P
    return (w0 * w3 % P, w2 * w1 % P, w1 * w3 % P, w0 * w2 % P)


def from_uniform_bytes(data):
    assert len(data) == 64
    halves = [int.from_bytes(half, "little") % 2**255 % P for half in (data[:32], data[32:])]
    return add(elligator(halves[0]), elligator(halves[1]))


# CPace, draft-irtf-cfrg-cpace-21, suite CPACE-RISTR255-SHA512.

DSI = b"CPaceRistretto255"


def leb128(n):
    out = bytearray()
    while True:
        byte, n = n & 0x7F, n >> 7
        out.append(byte | (0x80 if n else 0))
        if not n:
            return bytes(out)


def lv_cat(*parts):
    return b"".join(leb128(len(part)) + part for part in parts)


def generator_string(prs, ci, sid):
    zpad = 128 - 1 - len(lv_cat(prs)) - len(lv_cat(DSI))
    return lv_cat(DSI, prs, bytes(max(0, zpad)), ci, sid)


def generator(prs, ci, sid):
    return from_uniform_bytes(hashlib.sha512(generator_string(prs, ci, sid)).digest())


def share(scalar, prs, ci, sid):
    return encode(multiply(scalar, generator(prs, ci, sid)))


def isk(scalar, theirs, sid, ya, ad_a, yb, ad_b):
    point = decode(theirs)
    assert point is not None
    k = encode(multiply(scalar, point))
    assert k != bytes(32)
    transcript = lv_cat(ya, ad_a) + lv_cat(yb, ad_b)
    return hashlib.sha512(lv_cat(DSI + b"_ISK", sid, k) + transcript).digest()


def check_cpace_vectors():
    """Appendix B.3 of the draft."""
    prs, sid = b"Password", bytes.fromhex("7e4b4791d6a8ef019b936c79fb7f2c57")
    ci = b"\x0bA_initiator\x0bB_responder"
    assert len(generator_string(prs, ci, sid)) == 170
    g = bytes.fromhex("222b6b195fe84b1652badb6f6a3ae3d24341e7306967f0b8115b40d5698c7e56")
    assert encode(generator(prs, ci, sid)) == g
    ya = int.from_bytes(
        bytes.fromhex("da3d23700a9e5699258aef94dc060dfda5ebb61f02a5ea77fad53f4ff0976d08"), "little"
    )
    yb = int.from_bytes(
        bytes.fromhex("d2316b454718c35362d83d69df6320f38578ed5984651435e2949762d900b80d"), "little"
    )
    big_ya, big_yb = share(ya, prs, ci, sid), share(yb, prs, ci, sid)
    assert big_ya.hex() == "d6bac480f2c386c394efc7c47adb9925dcd2630b64f240c50f8d0eec482b9157"
    assert big_yb.hex() == "3ea7e0b19560d7c0b0f5734f63b955286dfa8232b5ebe63324e2d9e7433f7258"
    expected = (
        "b69effbf61b51d56401c0f65601abe428de8206feaaf0e32198896dcae7b35cd"
        "2b38950a39dfd5d4a79164614c2984f7daa460b588c1e80c3fa2068af7900447"
    )
    assert isk(ya, big_yb, sid, big_ya, b"ADa", big_yb, b"ADb").hex() == expected
    assert isk(yb, big_ya, sid, big_ya, b"ADa", big_yb, b"ADb").hex() == expected
    invalid = bytes.fromhex("2b3c6b8c4f3800e7aef6864025b4ed79bd599117e427c41bd47d93d654b4a51c")
    assert decode(invalid) is None


# HKDF (RFC 5869) with SHA-512.


def hkdf(ikm, info, length):
    prk = hmac.new(b"", ikm, hashlib.sha512).digest()
    out, block, counter = b"", b"", 1
    while len(out) < length:
        block = hmac.new(prk, block + info + bytes([counter]), hashlib.sha512).digest()
        out, counter = out + block, counter + 1
    return out[:length]


# ChaCha20-Poly1305 (RFC 8439).


def rotl(v, n):
    return (v << n | v >> 32 - n) & 0xFFFFFFFF


def quarter(s, a, b, c, d):
    s[a] = s[a] + s[b] & 0xFFFFFFFF
    s[d] = rotl(s[d] ^ s[a], 16)
    s[c] = s[c] + s[d] & 0xFFFFFFFF
    s[b] = rotl(s[b] ^ s[c], 12)
    s[a] = s[a] + s[b] & 0xFFFFFFFF
    s[d] = rotl(s[d] ^ s[a], 8)
    s[c] = s[c] + s[d] & 0xFFFFFFFF
    s[b] = rotl(s[b] ^ s[c], 7)


def chacha_block(key, counter, nonce):
    init = list(struct.unpack("<16I", b"expand 32-byte k" + key + struct.pack("<I", counter) + nonce))
    s = init[:]
    for _ in range(10):
        quarter(s, 0, 4, 8, 12)
        quarter(s, 1, 5, 9, 13)
        quarter(s, 2, 6, 10, 14)
        quarter(s, 3, 7, 11, 15)
        quarter(s, 0, 5, 10, 15)
        quarter(s, 1, 6, 11, 12)
        quarter(s, 2, 7, 8, 13)
        quarter(s, 3, 4, 9, 14)
    return struct.pack("<16I", *(a + b & 0xFFFFFFFF for a, b in zip(s, init)))


def chacha(key, counter, nonce, data):
    out = bytearray()
    for index in range(0, len(data), 64):
        block = chacha_block(key, counter + index // 64, nonce)
        out += bytes(a ^ b for a, b in zip(data[index : index + 64], block))
    return bytes(out)


def poly1305(key, message):
    r = int.from_bytes(key[:16], "little") & 0x0FFFFFFC0FFFFFFC0FFFFFFC0FFFFFFF
    s = int.from_bytes(key[16:], "little")
    acc = 0
    for index in range(0, len(message), 16):
        chunk = message[index : index + 16] + b"\x01"
        acc = (acc + int.from_bytes(chunk, "little")) * r % (2**130 - 5)
    return (acc + s & (2**128 - 1)).to_bytes(16, "little")


def pad16(data):
    return bytes(-len(data) % 16)


def seal(key, nonce, aad, plaintext):
    ciphertext = chacha(key, 1, nonce, plaintext)
    mac_data = aad + pad16(aad) + ciphertext + pad16(ciphertext)
    mac_data += struct.pack("<QQ", len(aad), len(ciphertext))
    return ciphertext + poly1305(chacha_block(key, 0, nonce)[:32], mac_data)


def check_aead_vector():
    """RFC 8439, section 2.8.2."""
    key = bytes(range(0x80, 0xA0))
    nonce = bytes.fromhex("070000004041424344454647")
    aad = bytes.fromhex("50515253c0c1c2c3c4c5c6c7")
    text = (
        b"Ladies and Gentlemen of the class of '99: If I could offer you "
        b"only one tip for the future, sunscreen would be it."
    )
    sealed = seal(key, nonce, aad, text)
    assert sealed[:16].hex() == "d31a8d34648e60db7b86afbc53ef7ec2"
    assert sealed[-16:].hex() == "1ae10b594f09e26a7e902ecbd0600691"


# The harness's supplied randomness: SplitMix64, one byte a round.


class Seeded:
    def __init__(self, seed):
        self.state = seed

    def fill(self, count):
        out = bytearray()
        for _ in range(count):
            self.state = (self.state + 0x9E3779B97F4A7C15) % 2**64
            m = self.state
            m = (m ^ m >> 30) * 0xBF58476D1CE4E5B9 % 2**64
            m = (m ^ m >> 27) * 0x94D049BB133111EB % 2**64
            out.append((m ^ m >> 31) & 0xFF)
        return bytes(out)


def scalar(randomness):
    """A scalar as the link code draws one: 64 bytes, little-endian, reduced."""
    return int.from_bytes(randomness.fill(64), "little") % L


def pin(randomness):
    """A PIN as the link code mints one: four bytes, big-endian, drawn again
    when they would favour some PINs over others."""
    while True:
        drawn = int.from_bytes(randomness.fill(4), "big")
        if drawn < 4_200_000_000:
            return b"%08d" % (drawn % 100_000_000)


def confirmation_message(isk_bytes, role, transcript, hub_share, relay_share, key, name):
    confirm_key = hkdf(isk_bytes, b"climon2 link pairing " + role + b" confirm", 32)
    box_key = hkdf(isk_bytes, b"climon2 link pairing " + role + b" box", 32)
    confirmation = hmac.new(confirm_key, transcript + hub_share + relay_share, hashlib.sha512).digest()
    box = seal(box_key, bytes(12), confirmation, key + bytes([len(name)]) + name)
    rest = confirmation + box
    return confirm_key, box_key, struct.pack(">H", len(rest)) + rest


def show(label, data):
    print(f"{label} ({len(data)} bytes)")
    for index in range(0, len(data), 32):
        print("    " + data[index : index + 32].hex())


def climon2_vectors():
    request = b"c2link\r\n" + bytes([1, 1, 0, 1]) + bytes(14)
    reply = bytes([1, 0, 1, 1, 0, 1]) + bytes(14)
    transcript = request + reply
    assert len(transcript) == 46
    digits = pin(Seeded(3))
    channel = b"climon2 link pairing" + transcript
    hub_scalar, relay_scalar = scalar(Seeded(1)), scalar(Seeded(2))
    hub_share = share(hub_scalar, digits, channel, b"")
    relay_share = share(relay_scalar, digits, channel, b"")
    key = isk(hub_scalar, relay_share, b"", hub_share, b"", relay_share, b"")
    assert key == isk(relay_scalar, hub_share, b"", hub_share, b"", relay_share, b"")
    hub_key, hub_name = bytes(range(0x40, 0x60)), "laptop".encode()
    relay_key, relay_name = bytes(range(0xA0, 0xC0)), "büild-box".encode()
    hub = confirmation_message(key, b"hub", transcript, hub_share, relay_share, hub_key, hub_name)
    relay = confirmation_message(
        key, b"relay", transcript, hub_share, relay_share, relay_key, relay_name
    )
    show("PIN, minted from seed 3", digits)
    show("opening transcript", transcript)
    show("hub monitor's share message, scalar from seed 1", b"\x00\x20" + hub_share)
    show("relay's share message, scalar from seed 2", b"\x00\x20" + relay_share)
    show("CPace key", key)
    show("hub confirmation key", hub[0])
    show("relay confirmation key", relay[0])
    show("hub box key", hub[1])
    show("relay box key", relay[1])
    show("hub monitor's confirmation message", hub[2])
    show("relay's confirmation message", relay[2])


if __name__ == "__main__":
    check_cpace_vectors()
    check_aead_vector()
    print("published vectors reproduced: CPace draft 21 B.3, RFC 8439 2.8.2")
    climon2_vectors()
