"""Generate the macOS bundle icon on any platform with Pillow installed."""

from io import BytesIO
from pathlib import Path
import struct

from PIL import Image


def chunk(kind, data):
    return struct.pack(">4sI", kind, len(data) + 8) + data


def main():
    source = Path(__file__).with_name("cheaprouter.png")
    # ICNS distinguishes logical sizes even when their pixel sizes match.
    entries = (
        (b"icp4", 16), (b"ic11", 32),   # 16pt, 1x and 2x
        (b"icp5", 32), (b"ic12", 64),   # 32pt
        (b"icp6", 64),
        (b"ic07", 128), (b"ic13", 256),  # 128pt
        (b"ic08", 256), (b"ic14", 512),  # 256pt
        (b"ic09", 512), (b"ic10", 1024),  # 512pt
    )
    images = {1024: source.read_bytes()}
    with Image.open(source) as image:
        if image.size != (1024, 1024):
            raise ValueError(f"{source} must be 1024x1024")
        for size in sorted({size for _, size in entries} - images.keys()):
            output = BytesIO()
            image.resize((size, size), Image.Resampling.LANCZOS).save(output, "PNG")
            images[size] = output.getvalue()
    source.with_suffix(".icns").write_bytes(
        chunk(b"icns", b"".join(chunk(kind, images[size]) for kind, size in entries))
    )


if __name__ == "__main__":
    main()
