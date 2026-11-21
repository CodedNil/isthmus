"""Extract static API references from local Spotify client artifacts."""

import argparse
import hashlib
import json
import re
from collections import defaultdict
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile

SPOTIFY_HOST = rb"(?:[a-z0-9-]+\.)*(?:spotify\.com|spotify\.net|scdn\.co|spotifycdn\.com|pscdn\.co)"
PATH = rb"[a-zA-Z0-9_./{}~%@+:-]*"
IDENTIFIER = rb"[a-zA-Z_][a-zA-Z0-9_]*"
QUALIFIED_NAME = IDENTIFIER + rb"(?:\." + IDENTIFIER + rb")*"
LITERAL_START = rb"""(?:^|[\x00"`'\s=(])"""
ROUTES = {
    "url": re.compile(
        rb"\b(?:https?|wss?)://"
        + SPOTIFY_HOST
        + rb"(?![a-zA-Z0-9.-])(?::\d+)?(?:/"
        + PATH
        + rb")?"
    ),
    "topic": re.compile(rb"\b(?:hm|sp)://[a-zA-Z0-9_.-]+/" + PATH),
    "route": re.compile(
        LITERAL_START
        + rb"(/?[a-z][a-z0-9_.-]*(?:/[a-zA-Z0-9_.-]+){0,5}/v[0-9]+[a-z]?(?:/"
        + PATH
        + rb")?)(?![a-zA-Z0-9_.-])"
    ),
    "rpc_path": re.compile(
        LITERAL_START
        + rb"(/?[a-zA-Z0-9_.-]+/spotify\."
        + QUALIFIED_NAME
        + rb"/"
        + IDENTIFIER
        + rb")"
    ),
}
DESCRIPTOR_START = re.compile(
    rb"\x0a[\x80-\xff]{0,2}[\x01-\x7f]([a-zA-Z0-9_./-]+\.proto)"
)


def varint(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError("invalid protobuf varint")


def fields(data, offset=0, *, embedded=False):
    """Read the wire types used by google/protobuf/descriptor.proto."""
    start = offset
    try:
        while offset < len(data):
            key, after_key = varint(data, offset)
            field, wire = divmod(key, 8)
            # The binary can store adjacent, unframed file descriptors.
            if field == 0 or (
                embedded and (field > 15 or (field == 1 and offset != start))
            ):
                return
            offset = after_key
            if wire == 2:
                size, offset = varint(data, offset)
                end = offset + size
                if end > len(data):
                    return
                yield field, data[offset:end]
                offset = end
            elif wire == 0:
                value, offset = varint(data, offset)
                yield field, value
            elif wire in (1, 5):
                offset += 8 if wire == 1 else 4
            else:
                return
    except (IndexError, ValueError):
        return


def descriptor_records(data):
    for match in DESCRIPTOR_START.finditer(data):
        size, _ = varint(data, match.start() + 1)
        if len(match[1]) != size:
            continue
        entries = list(fields(data, match.start(), embedded=True))
        package = next(
            (value for tag, value in entries if tag == 2 and isinstance(value, bytes)),
            b"",
        )
        if not re.fullmatch(QUALIFIED_NAME, package):
            continue
        for tag, value in entries:
            if tag not in (5, 6) or not isinstance(value, bytes):
                continue
            parts = list(fields(value))
            name = dict(parts).get(1, b"")
            if not isinstance(name, bytes) or not re.fullmatch(IDENTIFIER, name):
                continue
            if tag == 5 and name != b"ExtensionKind":
                continue
            for key, child in parts:
                if key != 2 or not isinstance(child, bytes):
                    continue
                item = dict(fields(child))
                label = item.get(1, b"")
                if not isinstance(label, bytes) or not re.fullmatch(IDENTIFIER, label):
                    continue
                if tag == 6:
                    yield (
                        "service",
                        (package + b"." + name).decode("ascii"),
                        label.decode("ascii"),
                    )
                elif isinstance(item.get(2), int):
                    yield "extension", item[2], label.decode("ascii")


def static_references(desktop, soloist, bundles):
    routes = defaultdict(lambda: (set(), set()))
    services = defaultdict(lambda: (set(), set()))
    extensions = {}
    artifacts = {}

    def scan_routes(data, location):
        for group, pattern in ROUTES.items():
            for match in pattern.finditer(data):
                route = match[match.lastindex or 0].decode("ascii")
                if group in ("route", "rpc_path"):
                    route = "/" + route.lstrip("/")
                if len(route) <= 256 and not route.endswith(".proto"):
                    routes[route][0].add(group)
                    routes[route][1].add(location)

    def read_artifact(path, label):
        data = path.read_bytes()
        artifacts[label] = {
            "file": path.name,
            "sha256": hashlib.sha256(data).hexdigest(),
        }
        return data

    for label, path in [("desktop", desktop), ("soloist", soloist)]:
        if path is None:
            continue
        data = read_artifact(path, label)
        if not data.startswith(b"\x7fELF"):
            raise ValueError(f"{path} is not an ELF executable")
        scan_routes(data, label)
        for kind, key, name in descriptor_records(data):
            if kind == "service":
                services[key][0].add(name)
                services[key][1].add(label)
            else:
                extensions.setdefault(key, name)

    for path in sorted(bundles):
        data = read_artifact(path, f"bundle:{path.name}")
        # SPA bundles are ZIP archives. Inspect members in memory.
        with ZipFile(BytesIO(data)) as archive:
            for member in archive.namelist():
                if member.endswith((".js", ".html")):
                    scan_routes(archive.read(member), f"{path.name}/{member}")

    locations = sorted({
        location
        for entries in (routes, services)
        for _, sources in entries.values()
        for location in sources
    })
    indices = {name: index for index, name in enumerate(locations)}

    def records(entries):
        return {
            name: [sorted(values), sorted(indices[location] for location in sources)]
            for name, (values, sources) in sorted(entries.items())
        }

    return {
        "schema_version": 2,
        "notes": "Static client literals and descriptors. They establish references, without verifying network use or availability.",
        "artifacts": artifacts,
        "locations": locations,
        "route_fields": ["groups", "location_indices"],
        "routes": records(routes),
        "service_fields": ["methods", "location_indices"],
        "service_methods": records(services),
        "metadata_extension_names": dict(sorted(extensions.items())),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("desktop", type=Path, help="Spotify Desktop ELF executable")
    parser.add_argument(
        "--soloist", type=Path, help="optional Spotify Soloist ELF executable"
    )
    parser.add_argument(
        "--bundle",
        type=Path,
        action="append",
        help="SPA bundle to scan, repeat for each bundle",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).with_name("API_INVENTORY.json"),
        help="report path (defaults to API_INVENTORY.json beside this script)",
    )
    args = parser.parse_args()

    bundles = args.bundle or [
        path
        for directory in (args.desktop.parent, args.desktop.parent / "Apps")
        for path in directory.glob("*.spa")
    ]
    report = static_references(args.desktop, args.soloist, bundles)
    args.output.write_text(json.dumps(report, separators=(",", ":")) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
