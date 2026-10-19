/*
 * noVNC: HTML5 VNC client
 * Copyright (C) 2021 The noVNC authors
 * Licensed under MPL 2.0 (see LICENSE.txt)
 *
 * See README.md for usage and integration instructions.
 *
 * Itda: rewritten for speed (see Itda's web/viewer/NOVNC.md). The original asked pako for
 * one byte at a time for every palette index and run length, which never reaches pako's
 * fast path, and drew each 64x64 tile separately. This inflates a rectangle's data in one
 * go, decodes it flat into one RGBA buffer and draws the rectangle once.
 */

import { inflateInit, inflate } from "../../vendor/pako/lib/zlib/inflate.js";
import ZStream from "../../vendor/pako/lib/zlib/zstream.js";

const TILE = 64;
const Z_SYNC_FLUSH = 2;

export default class ZRLEDecoder {
    constructor() {
        this._length = 0;
        this._strm = new ZStream();
        inflateInit(this._strm);
        this._out = new Uint8Array(1 << 16);
        this._rect = new Uint32Array(0);
        this._palette = new Uint32Array(128);
    }

    decodeRect(x, y, width, height, sock, display, depth) {
        if (this._length === 0) {
            if (sock.rQwait("ZLib data length", 4)) {
                return false;
            }
            this._length = sock.rQshift32();
        }
        if (sock.rQwait("Zlib data", this._length)) {
            return false;
        }

        const z = this._inflateAll(sock.rQshiftBytes(this._length, false));
        this._length = 0;

        const n = width * height;
        if (this._rect.length < n) {
            this._rect = new Uint32Array(n);
        }
        const out = this._rect;
        const pal = this._palette;
        // Little-endian RGBA (cpixel bytes are R, G, B in noVNC's pixel format).
        const px = (p) => 0xff000000 | (z[p + 2] << 16) | (z[p + 1] << 8) | z[p];
        let p = 0;

        for (let ty = 0; ty < height; ty += TILE) {
            const th = Math.min(TILE, height - ty);
            for (let tx = 0; tx < width; tx += TILE) {
                const tw = Math.min(TILE, width - tx);
                const sub = z[p++];
                if (sub === 0) {
                    // raw
                    for (let r = 0; r < th; r++) {
                        let o = (ty + r) * width + tx;
                        for (let c = 0; c < tw; c++, p += 3) {
                            out[o++] = px(p);
                        }
                    }
                } else if (sub === 1) {
                    // solid
                    const v = px(p);
                    p += 3;
                    for (let r = 0; r < th; r++) {
                        const o = (ty + r) * width + tx;
                        out.fill(v, o, o + tw);
                    }
                } else if (sub <= 16) {
                    // packed palette: rows of 1, 2 or 4 bit indices, MSB first
                    for (let k = 0; k < sub; k++, p += 3) {
                        pal[k] = px(p);
                    }
                    const bits = sub <= 2 ? 1 : sub <= 4 ? 2 : 4;
                    const mask = (1 << bits) - 1;
                    for (let r = 0; r < th; r++) {
                        let o = (ty + r) * width + tx;
                        let shift = 8 - bits;
                        let byte = z[p++];
                        for (let c = 0; c < tw; c++) {
                            if (shift < 0) {
                                shift = 8 - bits;
                                byte = z[p++];
                            }
                            const i = (byte >> shift) & mask;
                            if (i >= sub) {
                                throw new Error('Too big index in palette: ' + i + ', palette size: ' + sub);
                            }
                            out[o++] = pal[i];
                            shift -= bits;
                        }
                    }
                } else if (sub === 128 || sub >= 130) {
                    // plain RLE (128) or palette RLE (130..255)
                    const size = sub - 128;
                    for (let k = 0; k < size; k++, p += 3) {
                        pal[k] = px(p);
                    }
                    const total = tw * th;
                    let i = 0;
                    while (i < total) {
                        let v;
                        let len = 1;
                        if (size === 0) {
                            v = px(p);
                            p += 3;
                            len = 0;
                        } else {
                            let idx = z[p++];
                            if (idx & 128) {
                                idx &= 127;
                                len = 0;
                            }
                            if (idx >= size) {
                                throw new Error('Too big index in palette: ' + idx + ', palette size: ' + size);
                            }
                            v = pal[idx];
                        }
                        if (len === 0) {
                            let b;
                            do {
                                b = z[p++];
                                len += b;
                            } while (b === 255);
                            len += 1;
                        }
                        if (i + len > total) {
                            throw new Error('Too big rle length: ' + len + ', allowed length is: ' + (total - i));
                        }
                        // Fill the run row by row within the tile.
                        let r = (i / tw) | 0;
                        let c = i - r * tw;
                        i += len;
                        while (len > 0) {
                            const span = Math.min(len, tw - c);
                            const o = (ty + r) * width + tx + c;
                            out.fill(v, o, o + span);
                            len -= span;
                            c = 0;
                            r++;
                        }
                    }
                } else {
                    throw new Error('Unknown subencoding: ' + sub);
                }
                if (p > z.length) {
                    throw new Error("Incomplete zlib block");
                }
            }
        }
        if (p !== z.length) {
            throw new Error("Incomplete zlib block");
        }

        display.blitImage(x, y, width, height, new Uint8Array(out.buffer, 0, n * 4), 0, false);
        return true;
    }

    // All of one rectangle's zlib data (it ends with a sync flush), inflated in one call.
    _inflateAll(data) {
        /* eslint-disable camelcase */
        const s = this._strm;
        s.input = data;
        s.next_in = 0;
        s.avail_in = data.length;
        let total = 0;
        for (;;) {
            if (this._out.length - total < 32768) {
                const grown = new Uint8Array(this._out.length * 2);
                grown.set(this._out.subarray(0, total));
                this._out = grown;
            }
            s.output = this._out;
            s.next_out = total;
            s.avail_out = this._out.length - total;
            const ret = inflate(s, Z_SYNC_FLUSH);
            if (ret < 0) {
                throw new Error("zlib inflate failed");
            }
            total = s.next_out;
            if (s.avail_in === 0 && s.avail_out > 0) {
                break;
            }
        }
        /* eslint-enable camelcase */
        return this._out.subarray(0, total);
    }
}
