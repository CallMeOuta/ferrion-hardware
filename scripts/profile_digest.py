# SPDX-License-Identifier: GPL-2.0-only

import struct

MAX_INPUT = 1_048_576
CHUNK_BYTES = 1024
BLOCK_BYTES = 64
MASK = 0xFFFFFFFF
IV = (0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19)
PERMUTE = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
ROUNDS = 7


def rotate(word, amount):
    return ((word >> amount) | (word << (32 - amount))) & MASK


def mix(state, a, b, c, d, x, y):
    state[a] = (state[a] + state[b] + x) & MASK
    state[d] = rotate(state[d] ^ state[a], 16)
    state[c] = (state[c] + state[d]) & MASK
    state[b] = rotate(state[b] ^ state[c], 12)
    state[a] = (state[a] + state[b] + y) & MASK
    state[d] = rotate(state[d] ^ state[a], 8)
    state[c] = (state[c] + state[d]) & MASK
    state[b] = rotate(state[b] ^ state[c], 7)


def compress(chain, words, counter, size, flags):
    assert len(chain) == 8 and len(words) == 16
    state = list(chain) + list(IV[:4]) + [counter & MASK, counter >> 32, size, flags]
    for _ in range(ROUNDS):
        mix(state, 0, 4, 8, 12, words[0], words[1])
        mix(state, 1, 5, 9, 13, words[2], words[3])
        mix(state, 2, 6, 10, 14, words[4], words[5])
        mix(state, 3, 7, 11, 15, words[6], words[7])
        mix(state, 0, 5, 10, 15, words[8], words[9])
        mix(state, 1, 6, 11, 12, words[10], words[11])
        mix(state, 2, 7, 8, 13, words[12], words[13])
        mix(state, 3, 4, 9, 14, words[14], words[15])
        words = [words[index] for index in PERMUTE]
    return [state[index] ^ state[index + 8] for index in range(8)] + [state[index + 8] ^ chain[index] for index in range(8)]


def chaining(output):
    return compress(*output)[:8]


def blake3(data):
    if len(data) > MAX_INPUT:
        raise ValueError('BLAKE3 input exceeds 1048576 bytes')
    chunks = max(1, (len(data) + CHUNK_BYTES - 1) // CHUNK_BYTES)
    stack = []
    for chunk in range(chunks):
        payload = data[chunk * CHUNK_BYTES:(chunk + 1) * CHUNK_BYTES]
        blocks = max(1, (len(payload) + BLOCK_BYTES - 1) // BLOCK_BYTES)
        chain = IV
        for block in range(blocks):
            raw = payload[block * BLOCK_BYTES:(block + 1) * BLOCK_BYTES]
            flags = (1 if block == 0 else 0) | (2 if block == blocks - 1 else 0)
            output = (chain, struct.unpack('<16I', raw.ljust(BLOCK_BYTES, b'\0')), chunk, len(raw), flags)
            if block != blocks - 1:
                chain = chaining(output)
        if chunk == chunks - 1:
            break
        chain = chaining(output)
        total = chunk + 1
        for _ in range(MAX_INPUT.bit_length()):
            if total & 1:
                break
            assert stack
            chain = chaining((IV, stack.pop() + chain, 0, BLOCK_BYTES, 4))
            total >>= 1
        stack.append(chain)
    for left in reversed(stack):
        output = (IV, left + chaining(output), 0, BLOCK_BYTES, 4)
    chain, words, _, size, flags = output
    return struct.pack('<16I', *compress(chain, words, 0, size, flags | 8))[:32].hex()
