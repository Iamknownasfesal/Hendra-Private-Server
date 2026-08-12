#!/usr/bin/env python3
"""Extract the game assets out of the ActionScript client into the Godot project.

The AS3 client keeps its assets as Flash ``[Embed]`` stubs: a one-class ``.as`` file whose
``source="..."`` attribute names a sibling binary. Those binaries are ordinary files already
sitting in the repository, so "extraction" is mostly a copy — but *which* file belongs to which
logical sheet, and how each sheet is sliced, is only recorded in ActionScript. Rather than
transcribing that by hand (and letting it rot), this script parses the three files that hold the
mapping:

  AssetLoader.as    - addImages()             -> sheet name, embed, tile size
                    - addAnimatedCharacters() -> character sheet name, embed, mask, geometry
  EmbeddedAssets.as - embed variable -> stub class, plus the 3D model name table
  EmbeddedData.as   - the ordered lists of ground/object/region XML, which matter because
                      ObjectLibrary treats the first 25 object files as base data and everything
                      after as per-dungeon overrides

Outputs into ``godot-client/assets``:

  sheets/*.png       spritesheets, copied verbatim
  models/*.obj       the ``.dat`` 3D models, which are plain Wavefront OBJ text
  xml/*.xml          the ``.dat`` game data, which is plain XML
  manifest.json      everything the runtime needs to slice and load the above

Re-runnable and idempotent. Nothing here writes outside ``assets/``.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
from dataclasses import dataclass, asdict, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
AS3_SRC = REPO_ROOT / "Client-Side" / "src"
ASSET_DIR = AS3_SRC / "kabam" / "rotmg" / "assets"
ASSET_LOADER = AS3_SRC / "com" / "company" / "assembleegameclient" / "util" / "AssetLoader.as"
EMBEDDED_ASSETS = ASSET_DIR / "EmbeddedAssets.as"
EMBEDDED_DATA = ASSET_DIR / "EmbeddedData.as"

OUT_ROOT = Path(__file__).resolve().parents[1] / "assets"

# AnimatedChar.RIGHT / DOWN etc. The runtime needs the numeric value.
DIRECTIONS = {"RIGHT": 0, "LEFT": 1, "DOWN": 2, "UP": 3}

# ObjectLibrary.parseObjectFiles treats the first 25 entries of objectFiles as base definitions and
# everything after them as dungeon overrides parsed through a different path.
BASE_OBJECT_FILE_COUNT = 25


class ExtractionError(Exception):
    pass


@dataclass
class Sheet:
    """A spritesheet sliced into a uniform grid."""

    file: str
    tile_w: int
    tile_h: int
    # True for the one synthetic sheet ("invisible"), which the AS3 client built in code rather
    # than embedding. We generate it instead of copying it.
    synthetic: bool = False


@dataclass
class AnimatedCharSheet:
    """A character sheet: a grid of per-character strips, each 7 frames per direction row."""

    file: str
    mask: str | None
    frame_w: int
    frame_h: int
    strip_w: int
    strip_h: int
    first_dir: int


@dataclass
class Manifest:
    sheets: dict[str, dict] = field(default_factory=dict)
    animated_chars: dict[str, dict] = field(default_factory=dict)
    # Whole images used as-is rather than sliced into a grid.
    images: dict[str, str] = field(default_factory=dict)
    models: dict[str, str] = field(default_factory=dict)
    particles: str | None = None
    xml: dict[str, object] = field(default_factory=dict)


# Assets the AS3 code references by name off EmbeddedAssets rather than registering with
# AssetLibrary, so they never appear in an addImageSet call and have to be listed explicitly.
STANDALONE_IMAGES = ("StarburstSpinner", "EvolveBackground", "DarknessBackground")

# Loose interface art that is not embedded through EmbeddedAssets at all: each of these is a PNG
# sitting next to the class that embeds it, so it is copied by path rather than resolved through
# the embed table. The title graphic is the whole 800x600 title screen.
LOOSE_IMAGES = {
    # The original's title art, kept under its own name. The client does not use it -- see
    # OWN_IMAGES -- but extracting it means the reference is there to compare against.
    "OriginalTitleScreen": "kabam/rotmg/ui/view/TitleView_TitleScreenGraphic.png",
}

# Artwork that belongs to this client rather than the AS3 tree. It already lives in assets/sheets
# and is only listed here so the manifest knows its name; nothing copies over it.
OWN_IMAGES = {
    "TitleScreen": "TitleScreenGraphic.png",
}


def read(path: Path) -> str:
    if not path.is_file():
        raise ExtractionError(f"Expected to find {path}, but it does not exist.")
    # The AS3 sources are a mix of UTF-8 and UTF-8-with-BOM; a few have stray high bytes.
    return path.read_text(encoding="utf-8-sig", errors="replace")


# --------------------------------------------------------------------------------------------
# Parsing the ActionScript
# --------------------------------------------------------------------------------------------

_EMBED_VAR_RE = re.compile(
    r"(?:public|private)\s+static\s+(?:var|const)\s+(\w+)\s*:\s*Class\s*=\s*(\w+)\s*;"
)
_EMBED_SOURCE_RE = re.compile(r'\[Embed\(\s*source\s*=\s*"([^"]+)"')
_MODEL_ENTRY_RE = re.compile(r'"([^"]+)"\s*:\s*new\s+(\w+)\(\)')


def parse_embed_vars(text: str) -> dict[str, str]:
    """``EmbeddedAssets.foo`` -> the stub class name it aliases."""
    return {m.group(1): m.group(2) for m in _EMBED_VAR_RE.finditer(text)}


def resolve_embed_file(stub_class: str) -> Path:
    """Follow a stub class to the binary it embeds."""
    stub_path = ASSET_DIR / f"{stub_class}.as"
    match = _EMBED_SOURCE_RE.search(read(stub_path))
    if not match:
        raise ExtractionError(f"{stub_path.name} has no [Embed(source=...)] attribute.")
    source = ASSET_DIR / match.group(1)
    if not source.is_file():
        raise ExtractionError(f"{stub_path.name} embeds {match.group(1)}, which is missing.")
    return source


_ADD_IMAGE_SET_RE = re.compile(
    r'AssetLibrary\.addImageSet\(\s*"([^"]+)"\s*,\s*'
    r"new\s+EmbeddedAssets\.(\w+)\(\)\.bitmapData\s*,\s*(\d+)\s*,\s*(\d+)\s*\)"
)
_ADD_SYNTHETIC_SET_RE = re.compile(
    r'AssetLibrary\.addImageSet\(\s*"([^"]+)"\s*,\s*'
    r"new\s+BitmapDataSpy\([^)]*\)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)"
)
_ADD_ANIMATED_RE = re.compile(
    r'AnimatedChars\.add\(\s*"([^"]+)"\s*,\s*'
    r"new\s+EmbeddedAssets\.(\w+)\(\)\.bitmapData\s*,\s*"
    r"(null|new\s+EmbeddedAssets\.\w+\(\)\.bitmapData)\s*,\s*"
    r"(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*"
    r"AnimatedChar\.(\w+)\s*\)"
)
_MASK_VAR_RE = re.compile(r"new\s+EmbeddedAssets\.(\w+)\(\)\.bitmapData")


def parse_sheets(loader_text: str, embed_vars: dict[str, str]) -> dict[str, Sheet]:
    sheets: dict[str, Sheet] = {}

    for m in _ADD_IMAGE_SET_RE.finditer(loader_text):
        name, var, w, h = m.group(1), m.group(2), int(m.group(3)), int(m.group(4))
        if var not in embed_vars:
            raise ExtractionError(f'Sheet "{name}" references EmbeddedAssets.{var}, which is not declared.')
        source = resolve_embed_file(embed_vars[var])
        sheets[name] = Sheet(file=source.name, tile_w=w, tile_h=h)

    # The "invisible" sheet is a transparent bitmap constructed in code, not an embed.
    for m in _ADD_SYNTHETIC_SET_RE.finditer(loader_text):
        name, w, h = m.group(1), int(m.group(2)), int(m.group(3))
        sheets[name] = Sheet(file=f"{name}.png", tile_w=w, tile_h=h, synthetic=True)

    if not sheets:
        raise ExtractionError("Parsed no sheets out of AssetLoader.addImages(); the format must have changed.")
    return sheets


def parse_animated_chars(loader_text: str, embed_vars: dict[str, str]) -> dict[str, AnimatedCharSheet]:
    chars: dict[str, AnimatedCharSheet] = {}

    for m in _ADD_ANIMATED_RE.finditer(loader_text):
        name, var, mask_expr = m.group(1), m.group(2), m.group(3)
        frame_w, frame_h = int(m.group(4)), int(m.group(5))
        strip_w, strip_h = int(m.group(6)), int(m.group(7))
        direction = m.group(8)

        if var not in embed_vars:
            raise ExtractionError(f'Character sheet "{name}" references EmbeddedAssets.{var}, which is not declared.')
        if direction not in DIRECTIONS:
            raise ExtractionError(f'Character sheet "{name}" has unknown first direction {direction}.')

        mask_name = None
        if mask_expr != "null":
            mask_var = _MASK_VAR_RE.search(mask_expr).group(1)
            if mask_var not in embed_vars:
                raise ExtractionError(f'Character sheet "{name}" references mask {mask_var}, which is not declared.')
            mask_name = resolve_embed_file(embed_vars[mask_var]).name

        chars[name] = AnimatedCharSheet(
            file=resolve_embed_file(embed_vars[var]).name,
            mask=mask_name,
            frame_w=frame_w,
            frame_h=frame_h,
            strip_w=strip_w,
            strip_h=strip_h,
            first_dir=DIRECTIONS[direction],
        )

    if not chars:
        raise ExtractionError("Parsed no character sheets out of AssetLoader.addAnimatedCharacters().")
    return chars


def parse_models(assets_text: str, embed_vars: dict[str, str]) -> dict[str, str]:
    """The ``models_`` table maps an in-XML model name to its embed stub."""
    start = assets_text.find("models_")
    if start < 0:
        raise ExtractionError("EmbeddedAssets.as has no models_ table.")
    body = assets_text[start:]
    body = body[: body.find("};") + 1]

    models: dict[str, str] = {}
    for m in _MODEL_ENTRY_RE.finditer(body):
        display_name, alias = m.group(1), m.group(2)
        # The table refers to the private alias vars declared above it, not to the stub classes
        # directly; fall through to the bare name in case a future entry skips the alias.
        stub_class = embed_vars.get(alias, alias)
        models[display_name] = resolve_embed_file(stub_class).name
    if not models:
        raise ExtractionError("Parsed no entries out of the EmbeddedAssets.models_ table.")
    return models


_ARRAY_RE = re.compile(r"public\s+static\s+const\s+(\w+)\s*:\s*Array\s*=\s*\[(.*?)\]\s*;", re.DOTALL)
_NEW_CALL_RE = re.compile(r"new\s+(\w+)\s*(?:\(\s*\))?")
_PRIVATE_CONST_RE = re.compile(r"(?:private|public)\s+static\s+const\s+(\w+)\s*:\s*Class\s*=\s*(\w+)\s*;")


def parse_xml_lists(data_text: str) -> dict[str, object]:
    """The ordered ground/object/region XML lists, resolved to output filenames."""
    aliases = {m.group(1): m.group(2) for m in _PRIVATE_CONST_RE.finditer(data_text)}
    arrays = {m.group(1): m.group(2) for m in _ARRAY_RE.finditer(data_text)}

    def resolve_list(array_name: str) -> list[str]:
        if array_name not in arrays:
            raise ExtractionError(f"EmbeddedData.as has no {array_name} array.")
        names: list[str] = []
        for m in _NEW_CALL_RE.finditer(arrays[array_name]):
            alias = m.group(1)
            stub_class = aliases.get(alias, alias)
            names.append(resolve_embed_file(stub_class).stem + ".xml")
        return names

    return {
        "ground": resolve_list("groundFiles"),
        "objects": resolve_list("objectFiles"),
        "regions": resolve_list("regionFiles"),
        "spawnRegions": resolve_list("spawnRegionFiles"),
        # Everything past this index in "objects" is a dungeon override rather than base data.
        "baseObjectCount": BASE_OBJECT_FILE_COUNT,
    }


# --------------------------------------------------------------------------------------------
# Writing the output
# --------------------------------------------------------------------------------------------


def write_transparent_png(path: Path, width: int, height: int) -> None:
    """A fully transparent PNG, written by hand so the script keeps its zero-dependency property."""
    import struct
    import zlib

    raw = b"".join(b"\x00" + b"\x00\x00\x00\x00" * width for _ in range(height))

    def chunk(tag: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + tag
            + payload
            + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
        )

    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(png)


def copy_if_changed(source: Path, destination: Path) -> bool:
    """Copies only when the content differs, so Godot does not re-import untouched assets."""
    if destination.is_file() and destination.stat().st_size == source.stat().st_size:
        if destination.read_bytes() == source.read_bytes():
            return False
    shutil.copyfile(source, destination)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--clean", action="store_true", help="delete the output directory first")
    args = parser.parse_args()

    if args.clean and OUT_ROOT.exists():
        shutil.rmtree(OUT_ROOT)

    sheets_out = OUT_ROOT / "sheets"
    models_out = OUT_ROOT / "models"
    xml_out = OUT_ROOT / "xml"
    for directory in (sheets_out, models_out, xml_out):
        directory.mkdir(parents=True, exist_ok=True)

    loader_text = read(ASSET_LOADER)
    assets_text = read(EMBEDDED_ASSETS)
    data_text = read(EMBEDDED_DATA)

    embed_vars = parse_embed_vars(assets_text)
    sheets = parse_sheets(loader_text, embed_vars)
    animated = parse_animated_chars(loader_text, embed_vars)
    models = parse_models(assets_text, embed_vars)
    xml_lists = parse_xml_lists(data_text)

    manifest = Manifest(
        sheets={name: asdict(sheet) for name, sheet in sheets.items()},
        animated_chars={name: asdict(entry) for name, entry in animated.items()},
        models=models,
        xml=xml_lists,
    )

    copied = 0

    # Spritesheets, including the masks the character sheets reference.
    wanted_images = {sheet.file for sheet in sheets.values() if not sheet.synthetic}
    wanted_images |= {entry.file for entry in animated.values()}
    wanted_images |= {entry.mask for entry in animated.values() if entry.mask}
    for filename in sorted(wanted_images):
        copied += copy_if_changed(ASSET_DIR / filename, sheets_out / filename)

    for logical_name, relative in LOOSE_IMAGES.items():
        source = AS3_SRC / relative
        if not source.is_file():
            continue
        copied += copy_if_changed(source, sheets_out / source.name)
        manifest.images[logical_name] = source.name

    for logical_name, filename in OWN_IMAGES.items():
        if (sheets_out / filename).is_file():
            manifest.images[logical_name] = filename

    for logical_name in STANDALONE_IMAGES:
        stub_class = embed_vars.get(logical_name)
        if stub_class is None:
            continue
        source = resolve_embed_file(stub_class)
        copied += copy_if_changed(source, sheets_out / source.name)
        manifest.images[logical_name] = source.name

    for name, sheet in sheets.items():
        if sheet.synthetic:
            target = sheets_out / sheet.file
            if not target.is_file():
                write_transparent_png(target, sheet.tile_w, sheet.tile_h)
                copied += 1

    # 3D models. The .dat payloads are Wavefront OBJ text, which Godot imports natively once the
    # extension says so.
    for display_name, filename in models.items():
        target = models_out / (Path(filename).stem + ".obj")
        copied += copy_if_changed(ASSET_DIR / filename, target)
        manifest.models[display_name] = target.name

    # Particle definitions, and the ground/object/region XML the libraries parse.
    particles_stub = embed_vars.get("particlesEmbed")
    if particles_stub:
        source = resolve_embed_file(particles_stub)
        target = xml_out / (source.stem + ".xml")
        copied += copy_if_changed(source, target)
        manifest.particles = target.name

    for source in sorted(ASSET_DIR.glob("EmbeddedData_*.dat")):
        copied += copy_if_changed(source, xml_out / (source.stem + ".xml"))

    (OUT_ROOT / "manifest.json").write_text(
        json.dumps(asdict(manifest), indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    # A missing XML file here means the runtime would silently load an incomplete object library,
    # so surface it rather than letting it turn into "that monster has no sprite" later.
    missing = [
        name
        for group in ("ground", "objects", "regions", "spawnRegions")
        for name in xml_lists[group]
        if not (xml_out / name).is_file()
    ]
    if missing:
        raise ExtractionError(
            "These XML files are referenced by EmbeddedData.as but were not extracted:\n  "
            + "\n  ".join(missing)
        )

    print(
        f"{len(sheets)} sheets, {len(animated)} character sheets, {len(models)} models, "
        f"{len(list(xml_out.glob('*.xml')))} xml files -> {OUT_ROOT}"
    )
    print(f"{copied} file(s) written; the rest were already up to date.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ExtractionError as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
