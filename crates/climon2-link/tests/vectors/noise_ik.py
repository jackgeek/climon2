"""climon2's frozen vector for the pinned-key handshake (SEC-08), computed by
an implementation of Noise_IK_25519_ChaChaPoly_SHA256 (Noise revision 34)
that shares no code with the crate under test: X25519 from RFC 7748,
ChaCha20-Poly1305 from RFC 8439 and SHA-256 and HMAC from Python's standard
library. It is not for secrets and never ships.

    python3 -I noise_ik.py <cacophony.txt>

reproduces the published cacophony vector for the pattern first, then prints
the bytes `tests/handshake.rs` freezes. The cacophony file is in the source
of the pinned `snow` crate, `tests/vectors/cacophony.txt`.
"""
import hashlib, hmac, json, sys

P = 2**255 - 19

def x25519(k, u):
    k = bytearray(k); k[0] &= 248; k[31] &= 127; k[31] |= 64
    k = int.from_bytes(k, 'little'); u = int.from_bytes(u, 'little') & ((1 << 255) - 1)
    x1, x2, z2, x3, z3, swap = u, 1, 0, u, 1, 0
    for t in reversed(range(255)):
        kt = (k >> t) & 1; swap ^= kt
        if swap: x2, x3, z2, z3 = x3, x2, z3, z2
        swap = kt
        A = x2 + z2; AA = A * A; B = x2 - z2; BB = B * B; E = AA - BB
        C = x3 + z3; D = x3 - z3; DA = D * A; CB = C * B
        x3 = (DA + CB) ** 2 % P; z3 = x1 * (DA - CB) ** 2 % P
        x2 = AA * BB % P; z2 = E * (AA + 121665 * E) % P
    if swap: x2, x3, z2, z3 = x3, x2, z3, z2
    return (x2 * pow(z2, P - 2, P) % P).to_bytes(32, 'little')

def pub(k): return x25519(k, (9).to_bytes(32, 'little'))

def rotl(v, c): return ((v << c) & 0xffffffff) | (v >> (32 - c))

def qr(s, a, b, c, d):
    s[a] = (s[a] + s[b]) & 0xffffffff; s[d] = rotl(s[d] ^ s[a], 16)
    s[c] = (s[c] + s[d]) & 0xffffffff; s[b] = rotl(s[b] ^ s[c], 12)
    s[a] = (s[a] + s[b]) & 0xffffffff; s[d] = rotl(s[d] ^ s[a], 8)
    s[c] = (s[c] + s[d]) & 0xffffffff; s[b] = rotl(s[b] ^ s[c], 7)

def block(key, counter, nonce):
    s = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]
    s += [int.from_bytes(key[i:i+4], 'little') for i in range(0, 32, 4)]
    s += [counter] + [int.from_bytes(nonce[i:i+4], 'little') for i in range(0, 12, 4)]
    w = s[:]
    for _ in range(10):
        qr(w,0,4,8,12); qr(w,1,5,9,13); qr(w,2,6,10,14); qr(w,3,7,11,15)
        qr(w,0,5,10,15); qr(w,1,6,11,12); qr(w,2,7,8,13); qr(w,3,4,9,14)
    return b''.join(((w[i] + s[i]) & 0xffffffff).to_bytes(4, 'little') for i in range(16))

def chacha(key, counter, nonce, data):
    out = bytearray()
    for i in range(0, len(data), 64):
        ks = block(key, counter + i // 64, nonce)
        out += bytes(a ^ b for a, b in zip(data[i:i+64], ks))
    return bytes(out)

def poly1305(key, msg):
    r = int.from_bytes(key[:16], 'little') & 0x0ffffffc0ffffffc0ffffffc0fffffff
    s = int.from_bytes(key[16:], 'little'); a = 0; p = (1 << 130) - 5
    for i in range(0, len(msg), 16):
        n = int.from_bytes(msg[i:i+16] + b'\x01', 'little')
        a = (a + n) * r % p
    return ((a + s) & ((1 << 128) - 1)).to_bytes(16, 'little')

def pad16(b): return b'\x00' * (-len(b) % 16)

def aead(key, n, ad, pt):
    nonce = b'\x00' * 4 + n.to_bytes(8, 'little')
    otk = block(key, 0, nonce)[:32]
    ct = chacha(key, 1, nonce, pt)
    mac = ad + pad16(ad) + ct + pad16(ct) + len(ad).to_bytes(8, 'little') + len(ct).to_bytes(8, 'little')
    return ct + poly1305(otk, mac)

def h(b): return hashlib.sha256(b).digest()
def hm(k, b): return hmac.new(k, b, hashlib.sha256).digest()
def hkdf(ck, ikm):
    t = hm(ck, ikm); o1 = hm(t, b'\x01'); o2 = hm(t, o1 + b'\x02'); return o1, o2

class Sym:
    def __init__(self, name, prologue):
        self.h = name.ljust(32, b'\x00') if len(name) <= 32 else h(name)
        self.ck = self.h; self.k = None; self.n = 0
        self.mix_hash(prologue)
    def mix_hash(self, d): self.h = h(self.h + d)
    def mix_key(self, ikm): self.ck, self.k = hkdf(self.ck, ikm); self.n = 0
    def enc(self, pt):
        if self.k is None: c = pt
        else: c = aead(self.k, self.n, self.h, pt); self.n += 1
        self.mix_hash(c); return c
    def split(self): return hkdf(self.ck, b'')

NAME = b'Noise_IK_25519_ChaChaPoly_SHA256'

def ik(prologue, i_s, i_e, r_s, r_e, payloads):
    """Messages of one IK run, as the initiator and responder write them."""
    ini = Sym(NAME, prologue); res = Sym(NAME, prologue)
    rs_pub = pub(r_s)
    for st in (ini, res): st.mix_hash(rs_pub)
    # -> e, es, s, ss
    e_pub = pub(i_e); m = e_pub
    ini.mix_hash(e_pub); ini.mix_key(x25519(i_e, rs_pub))
    m += ini.enc(pub(i_s)); ini.mix_key(x25519(i_s, rs_pub)); m += ini.enc(payloads[0])
    # responder mirrors with its own computation
    res.mix_hash(e_pub); res.mix_key(x25519(r_s, e_pub))
    assert res.enc(pub(i_s)) == m[32:80]
    res.mix_key(x25519(r_s, pub(i_s))); res.enc(payloads[0])
    msgs = [m]
    # <- e, ee, se
    re_pub = pub(r_e); m = re_pub
    res.mix_hash(re_pub); res.mix_key(x25519(r_e, e_pub)); res.mix_key(x25519(r_e, pub(i_s)))
    m += res.enc(payloads[1]); msgs.append(m)
    ini.mix_hash(re_pub); ini.mix_key(x25519(i_e, re_pub)); ini.mix_key(x25519(i_s, re_pub))
    assert ini.enc(payloads[1]) == m[32:]
    assert ini.h == res.h
    k1, k2 = ini.split(); n = [0, 0]
    for i, p in enumerate(payloads[2:]):
        which = i % 2  # initiator first
        key = k1 if which == 0 else k2
        msgs.append(aead(key[:32], n[which], b'', p)); n[which] += 1
    return msgs, ini.h

def cacophony(path):
    for v in json.load(open(path))['vectors']:
        if v.get('protocol_name') == NAME.decode():
            x = bytes.fromhex
            msgs, hh = ik(x(v['init_prologue']), x(v['init_static']), x(v['init_ephemeral']),
                          x(v['resp_static']), x(v['resp_ephemeral']),
                          [x(m['payload']) for m in v['messages']])
            assert pub(x(v['resp_static'])) == x(v['init_remote_static'])
            for m, w in zip(msgs, v['messages']):
                assert m.hex() == w['ciphertext'], (m.hex(), w['ciphertext'])
            assert hh.hex() == v['handshake_hash']
            print('cacophony IK vector reproduced')

if __name__ == '__main__':
    cacophony(sys.argv[1])
    # The opening exchange of the protocol pages' examples is the prologue.
    request = bytes.fromhex('63326c696e6b0d0a02010001') + bytes(14)
    reply = bytes.fromhex('0100010100010000000000000000000000000000')
    i_s = bytes([0x11]) * 32; r_s = bytes([0x22]) * 32
    i_e = bytes(range(0x40, 0x60)); r_e = bytes(range(0x60, 0x80))
    hub_hello = bytes.fromhex('0100000005322e342e300000000561746c61730000000000100000')
    relay_hello = (b'\x01' + (5).to_bytes(4, 'big') + b'2.5.1' + (5).to_bytes(4, 'big')
                   + b'forge' + bytes(4) + (4096).to_bytes(4, 'big'))
    # Each Hello is one unit on stream 0: four bytes of stream, no flags.
    msgs, _ = ik(request + reply, i_s, i_e, r_s, r_e,
                 [b'', b'', bytes(5) + hub_hello, bytes(5) + relay_hello])
    print('HUB_PUBLIC', pub(i_s).hex())
    print('RELAY_PUBLIC', pub(r_s).hex())
    for name, m in zip(['FROZEN_INITIATION', 'FROZEN_REPLY', 'FROZEN_HUB_HELLO',
                        'FROZEN_RELAY_HELLO'], msgs):
        print(name, len(m).to_bytes(2, 'big').hex() + m.hex())
