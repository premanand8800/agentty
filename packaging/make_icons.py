"""Generate agentty.png (Linux, 256px) and agentty.icns (macOS). Run: uv run --with pillow python packaging/make_icons.py"""
import io, struct
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
MONO_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
SANS_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


def icon(size: int) -> Image.Image:
    k = size / 256
    im = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((8 * k, 8 * k, size - 8 * k, size - 8 * k), radius=48 * k, fill=(28, 28, 30, 255), outline=(70, 70, 74, 255), width=max(1, round(3 * k)))
    d.rounded_rectangle((8 * k, 8 * k, size - 8 * k, 64 * k), radius=48 * k, fill=(44, 44, 46, 255))
    d.rectangle((8 * k, 40 * k, size - 8 * k, 64 * k), fill=(44, 44, 46, 255))
    for i, c in enumerate([(255, 95, 87), (254, 188, 46), (40, 200, 64)]):
        x = (40 + i * 30) * k
        d.ellipse((x - 10 * k, 26 * k, x + 10 * k, 46 * k), fill=c)
    d.text((38 * k, 92 * k), "❯", font=ImageFont.truetype(SANS_BOLD, round(88 * k)), fill=(48, 209, 88, 255))
    d.text((122 * k, 92 * k), "_", font=ImageFont.truetype(MONO_BOLD, round(88 * k)), fill=(242, 242, 242, 255))
    d.ellipse((186 * k, 186 * k, 222 * k, 222 * k), fill=(255, 159, 10, 255))
    return im


def png_bytes(im: Image.Image) -> bytes:
    buf = io.BytesIO()
    im.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


icon(256).save(HERE / "agentty.png", optimize=True)
# icns: 'ic08' 256px, 'ic09' 512px, 'ic10' 1024px (512@2x); each entry is type + length + PNG data.
chunks = b"".join(t + struct.pack(">I", 8 + len(p)) + p for t, p in [(b"ic08", png_bytes(icon(256))), (b"ic09", png_bytes(icon(512))), (b"ic10", png_bytes(icon(1024)))])
(HERE / "macos" / "agentty.icns").write_bytes(b"icns" + struct.pack(">I", 8 + len(chunks)) + chunks)
print("wrote agentty.png and macos/agentty.icns")
