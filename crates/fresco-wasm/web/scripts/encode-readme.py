"""Encode deterministic PNG captures as compact WebP (loop=0 means forever)."""
import json
import sys
from pathlib import Path

from PIL import Image, features


def main():
    if not features.check("webp"):
        raise RuntimeError("Pillow must have WebP support")
    directory, output, duration = sys.argv[1:]
    paths = sorted(Path(directory).glob("frame-*.png"))
    if not paths:
        raise ValueError("No captured frames")
    frames = []
    for path in paths:
        with Image.open(path) as image:
            frames.append(image.convert("RGBA"))
    if any(frame.size != frames[0].size for frame in frames):
        raise ValueError("Frame dimensions changed during capture")
    # Integer timestamps retain the requested total duration at fractional-ms FPS.
    total_ms = float(duration) * 1000
    durations = [round((i + 1) * total_ms / len(frames)) - round(i * total_ms / len(frames))
                 for i in range(len(frames))]
    animated = len(frames) > 1
    # Let the animation encoder choose lossy or lossless per frame and avoid
    # periodic keyframes. Still images remain lossless; timing is unchanged.
    frames[0].save(output, format="WEBP", save_all=True, append_images=frames[1:],
                   lossless=not animated, allow_mixed=animated, quality=85,
                   minimize_size=True, method=6, duration=durations, loop=0)
    with Image.open(output) as result:
        if len(frames) > 1 and result.n_frames < 2:
            raise ValueError("Animated sample produced identical frames; check time propagation")
        if result.n_frames > 1 and result.info.get("loop") != 0:
            raise ValueError("Animation must loop forever")
        print(json.dumps({"frames": result.n_frames, "width": result.width,
                          "height": result.height, "loop": result.info.get("loop", 0),
                          "encoding": "mixed" if animated else "lossless",
                          "quality": 85}))


if __name__ == "__main__":
    main()
