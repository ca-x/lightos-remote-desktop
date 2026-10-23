#!/usr/bin/env python3
"""Generate deterministic compressed fixtures; never capture a real desktop."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("directory", type=Path)
parser.add_argument("--frames", type=int, default=120)
args = parser.parse_args()
assert 2 <= args.frames <= 600
for tool in ["ffmpeg", "ffprobe"]:
    if not shutil.which(tool):
        raise SystemExit(f"required benchmark tool is missing: {tool}")
directory = args.directory
directory.mkdir(parents=True, exist_ok=True)
source = "testsrc2=size=1280x720:rate=30"
def ffmpeg(*extra):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", source, *extra], check=True)
ffmpeg("-frames:v", "1", "-q:v", "3", str(directory / "frame.jpg"))
ffmpeg("-frames:v", str(args.frames), "-an", "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency", "-g", "60", "-bf", "0", "-b:v", "4000k", "-pix_fmt", "yuv420p", "-f", "h264", str(directory / "stream.h264"))
packets = json.loads(subprocess.check_output(["ffprobe", "-v", "error", "-f", "h264", "-show_packets", "-show_entries", "packet=pos,size,flags", "-of", "json", str(directory / "stream.h264")]))["packets"]
stream = (directory / "stream.h264").read_bytes()
files, keys = [], []
for i, packet in enumerate(packets):
    start, size = int(packet["pos"]), int(packet["size"])
    name = f"frame-{i:04d}.h264"
    (directory / name).write_bytes(stream[start:start+size])
    files.append(name)
    keys.append("K" in packet["flags"])
# SPS profile_idc, constraint flags and level_idc form the WebCodecs AVC string.
import re
nals = re.split(b"\x00\x00\x00?\x01", stream)
sps = next(nal for nal in nals if nal and nal[0] & 31 == 7)
manifest = {
    "generator": "ffmpeg lavfi testsrc2 1280x720, 30 fps; x264 ultrafast zerolatency 4 Mbit/s, GOP 60, no B frames",
    "ffmpeg": subprocess.check_output(["ffmpeg", "-version"], text=True).splitlines()[0],
    "jpeg": {"files": ["frame.jpg"] * args.frames, "configuration": "jpeg", "sha256": hashlib.sha256((directory / "frame.jpg").read_bytes()).hexdigest()},
    "h264": {"files": files, "keyframes": keys, "configuration": "avc1." + sps[1:4].hex(), "sha256": hashlib.sha256(stream).hexdigest()},
}
(directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"frames": len(files), "jpeg_bytes_per_frame": (directory / "frame.jpg").stat().st_size, "h264_total_bytes": len(stream), "configuration": manifest["h264"]["configuration"]}))
