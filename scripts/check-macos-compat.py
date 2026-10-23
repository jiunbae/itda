#!/usr/bin/env python3
"""Check every Mach-O slice without launching it or requesting capture permissions."""
import argparse
import json
from pathlib import Path
import plistlib
import re
import subprocess
import sys

# Existing strong dependencies predate the app's macOS 12.0 floor. A newly added
# framework needs an availability review rather than silently raising that floor.
STRONG_FRAMEWORKS = {
    "ApplicationServices", "ServiceManagement", "WebKit", "CoreGraphics", "Carbon",
    "CoreVideo", "CoreFoundation", "AppKit", "Foundation", "VideoToolbox",
    "CoreMedia", "ColorSync", "CoreServices",
}


def version(text):
    parts = text.split(".")
    if not parts or any(not part.isdecimal() for part in parts) or len(parts) > 3:
        raise ValueError("Invalid macOS version: " + text)
    return tuple(map(int, parts)) + (0,) * (3 - len(parts))


def check(binary, minimum, universal):
    arches = subprocess.check_output(["lipo", "-archs", str(binary)], text=True).split()
    if not arches or (universal and not {"arm64", "x86_64"}.issubset(arches)):
        raise ValueError("Missing required Mach-O architectures")
    plist = binary.parent.parent / "Info.plist"
    if plist.is_file():
        declared = plistlib.loads(plist.read_bytes()).get("LSMinimumSystemVersion", "")
        if version(declared) != version(minimum):
            raise ValueError("Bundle minimum macOS must remain " + minimum)
    result = []
    for arch in arches:
        load = subprocess.check_output(["otool", "-arch", arch, "-l", str(binary)], text=True)
        blocks = re.split(r"Load command \d+\n", load)
        for block in blocks:
            if re.search(r"^\s*cmd LC_(?:LOAD|REEXPORT)_DYLIB\s*$", block, re.M):
                framework = re.search(r"name /System/Library/Frameworks/([^/]+)\.framework/", block)
                if framework and framework.group(1) not in STRONG_FRAMEWORKS:
                    raise ValueError(arch + ": review the macOS floor of new strong framework " + framework.group(1))
        sck = [b for b in blocks if "ScreenCaptureKit.framework/" in b]
        if len(sck) != 1 or not re.search(r"^\s*cmd LC_LOAD_WEAK_DYLIB\s*$", sck[0], re.M):
            raise ValueError(arch + ": ScreenCaptureKit must have exactly one LC_LOAD_WEAK_DYLIB")
        deployment = re.search(r"^\s*minos (\d+(?:\.\d+){0,2})\s*$", load, re.M)
        if not deployment:
            legacy = next((b for b in blocks if "cmd LC_VERSION_MIN_MACOSX" in b), "")
            deployment = re.search(r"^\s*version (\d+(?:\.\d+){0,2})\s*$", legacy, re.M)
        if not deployment or version(deployment.group(1)) > version(minimum):
            raise ValueError(arch + ": executable deployment target exceeds macOS " + minimum)
        result.append({"architecture": arch, "minimum_macos": deployment.group(1), "screen_capture_kit": "weak"})
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--minimum-macos", default="12.0")
    parser.add_argument("--universal", action="store_true")
    args = parser.parse_args()
    try:
        result = check(args.binary.resolve(), args.minimum_macos, args.universal)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print("macOS compatibility check failed: " + str(error), file=sys.stderr)
        sys.exit(1)
    print(json.dumps({"status": "passed", "slices": result}))
