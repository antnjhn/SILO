import struct, zlib, sys

def load(path):
    d = open(path, 'rb').read()
    assert d[:8] == b'\x89PNG\r\n\x1a\n'
    i = 8; idat = b''; w = h = None; bd = ct = None
    while i < len(d):
        ln = struct.unpack('>I', d[i:i+4])[0]; typ = d[i+4:i+8]; data = d[i+8:i+8+ln]
        if typ == b'IHDR':
            w, h, bd, ct = struct.unpack('>IIBB', data[:10])
        elif typ == b'IDAT':
            idat += data
        elif typ == b'IEND':
            break
        i += 12 + ln
    raw = zlib.decompress(idat)
    nch = {0:1, 2:3, 4:2, 6:4}[ct]
    stride = w * nch
    out = bytearray(stride * h)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]; p += 1
        line = bytearray(raw[p:p+stride]); p += stride
        if f == 1:
            for x in range(nch, stride): line[x] = (line[x] + line[x-nch]) & 255
        elif f == 2:
            for x in range(stride): line[x] = (line[x] + prev[x]) & 255
        elif f == 3:
            for x in range(stride):
                a = line[x-nch] if x >= nch else 0
                line[x] = (line[x] + ((a + prev[x]) >> 1)) & 255
        elif f == 4:
            for x in range(stride):
                a = line[x-nch] if x >= nch else 0
                b = prev[x]; c = prev[x-nch] if x >= nch else 0
                pp = a + b - c
                pa, pb, pc = abs(pp-a), abs(pp-b), abs(pp-c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        out[y*stride:(y+1)*stride] = line
        prev = line
    return w, h, nch, out

def px(img, x, y):
    w, h, nch, buf = img
    o = (y*w + x)*nch
    return buf[o], buf[o+1], buf[o+2]

def classify(r, g, b):
    mx, mn = max(r, g, b), min(r, g, b)
    if mx < 60: return '.'          # dark / black
    if mx - mn < 25:                # neutral
        return '#' if mx > 200 else ('+' if mx > 120 else '-')
    # colored
    if r > 150 and g > 140 and b < 120: return 'A'   # yellow accent
    if r > 120 and g < 110 and b < 130 and r > g + 40: return 'R'
    if b >= r and b > 100: return 'B'
    return 'w'                       # wallpaper olive

if __name__ == '__main__':
    path = sys.argv[1]
    x0, x1, y = int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    step = int(sys.argv[5]) if len(sys.argv) > 5 else 4
    img = load(path)
    print(f'{path} size={img[0]}x{img[1]}')
    for yy in range(y-int(sys.argv[6]) if len(sys.argv) > 6 else y,
                   y+int(sys.argv[7]) if len(sys.argv) > 7 else y+1,
                   3):
        row = []
        for x in range(x0, x1, step):
            row.append(classify(*px(img, x, yy)))
        print(f'{yy:4d} ' + ''.join(row))
