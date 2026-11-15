#!/usr/bin/env python3
"""Writes search-core/test-data/mixed-text-scan.pdf: pages 1 and 2 have a text layer, page 3 is only a JPEG of rendered text.
(yolowpdf calls a document scanned when half its pages are, so the text pages must outnumber the scan.)

Used by the `scanned_page_inside_a_text_pdf_is_recognised_with_its_page_number` test (feature `pdf-yolow` + `ocr`).
Needs Pillow and a TrueType font (default: macOS Arial); pass another font path as the first argument.
"""
import io
import sys

from PIL import Image, ImageDraw, ImageFont

FONT = sys.argv[1] if len(sys.argv) > 1 else "/System/Library/Fonts/Supplemental/Arial.ttf"
OUT = sys.argv[2] if len(sys.argv) > 2 else "search-core/test-data/mixed-text-scan.pdf"
LINES = ["Hydraulic pressure relief valve", "Inspect the torque stripe before flight", "Part number MS21250 rev B"]

img = Image.new("L", (1275, 1650), 255)
draw = ImageDraw.Draw(img)
font = ImageFont.truetype(FONT, 56)
for i, text in enumerate(LINES):
    draw.text((120, 200 + i * 140), text, fill=0, font=font)
jpeg = io.BytesIO()
img.save(jpeg, "JPEG", quality=90)
jpeg = jpeg.getvalue()

def text_page(lines):
    ops = b"BT /F1 18 Tf 72 700 Td " + b" 0 -28 Td ".join(b"(%s) Tj" % t for t in lines) + b" ET"
    return ops

stream1 = text_page([b"Page one has a real text layer about apple and banana.", b"Second line of the first page."])
stream2 = text_page([b"Page two also has text: gasket seating and flange bolts.", b"Another line of the second page."])
page = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents %d 0 R >>"
draw_scan = b"q 612 0 0 792 0 0 cm /Im0 Do Q"
objs = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R 4 0 R 6 0 R] /Count 3 >>",
    page % 8,
    page % 9,
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 7 0 R >> >> /Contents 10 0 R >>",
    b"<< /Type /XObject /Subtype /Image /Width 1275 /Height 1650 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode /Length %d >>\nstream\n" % len(jpeg) + jpeg + b"\nendstream",
    b"<< /Length %d >>\nstream\n" % len(stream1) + stream1 + b"\nendstream",
    b"<< /Length %d >>\nstream\n" % len(stream2) + stream2 + b"\nendstream",
    b"<< /Length %d >>\nstream\n" % len(draw_scan) + draw_scan + b"\nendstream",
]
out = bytearray(b"%PDF-1.4\n")
offsets = []
for n, body in enumerate(objs, 1):
    offsets.append(len(out))
    out += b"%d 0 obj\n" % n + body + b"\nendobj\n"
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
for off in offsets:
    out += b"%010d 00000 n \n" % off
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
open(OUT, "wb").write(out)
print(OUT, len(out), "bytes")
