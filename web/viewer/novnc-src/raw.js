/*
 * noVNC: HTML5 VNC client
 * Copyright (C) 2019 The noVNC authors
 * Licensed under MPL 2.0 (see LICENSE.txt)
 *
 * Itda: draw available scanlines in blocks instead of one canvas operation per row.
 *
 * See README.md for usage and integration instructions.
 *
 */

export default class RawDecoder {
    constructor() {
        this._lines = 0;
    }

    decodeRect(x, y, width, height, sock, display, depth) {
        if ((width === 0) || (height === 0)) {
            return true;
        }

        if (this._lines === 0) {
            this._lines = height;
        }

        const pixelSize = depth == 8 ? 1 : 4;
        const bytesPerLine = width * pixelSize;

        while (this._lines > 0) {
            if (sock.rQwait("RAW", bytesPerLine)) {
                return false;
            }

            const lines = Math.min(this._lines, Math.floor(sock.rQlen() / bytesPerLine), 128);
            const pixels = width * lines;
            const curY = y + (height - this._lines);

            let data = sock.rQshiftBytes(bytesPerLine * lines, false);

            // Convert data if needed
            if (depth == 8) {
                const newdata = new Uint8Array(pixels * 4);
                for (let i = 0; i < pixels; i++) {
                    newdata[i * 4 + 0] = ((data[i] >> 0) & 0x3) * 255 / 3;
                    newdata[i * 4 + 1] = ((data[i] >> 2) & 0x3) * 255 / 3;
                    newdata[i * 4 + 2] = ((data[i] >> 4) & 0x3) * 255 / 3;
                    newdata[i * 4 + 3] = 255;
                }
                data = newdata;
            }

            // Max sure the image is fully opaque
            for (let i = 0; i < pixels; i++) {
                data[i * 4 + 3] = 255;
            }

            display.blitImage(x, curY, width, lines, data, 0);
            this._lines -= lines;
        }

        return true;
    }
}
