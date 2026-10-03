#!/usr/bin/env python3
"""Print actual ELF dynamic dependencies and required glibc symbol versions."""
import pathlib
import struct
import sys

for filename in sys.argv[1:]:
    data = pathlib.Path(filename).read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise SystemExit("expected ELF64 little-endian executable")
    shoff = struct.unpack_from("<Q", data, 40)[0]
    shsize, shnum, shstrings = struct.unpack_from("<HHH", data, 58)
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shsize) for i in range(shnum)]
    def contents(section):
        return data[section[4]:section[4] + section[5]]
    names = contents(sections[shstrings])
    def string(table, offset):
        return table[offset:table.index(b"\0", offset)].decode()
    by_name = {string(names, section[0]): section for section in sections}
    strings = contents(by_name[".dynstr"])
    versions = contents(by_name[".gnu.version_r"])
    requirements = set()
    offset = 0
    while offset < len(versions):
        _, count, library, auxiliary, following = struct.unpack_from("<HHIII", versions, offset)
        print(filename, "needs", string(strings, library))
        cursor = offset + auxiliary
        for _ in range(count):
            _, _, _, name, next_aux = struct.unpack_from("<IHHII", versions, cursor)
            version = string(strings, name)
            if version.startswith("GLIBC_"):
                requirements.add(version)
            cursor += next_aux
        if following == 0:
            break
        offset += following
    print(filename, "maximum required glibc", max(requirements, key=lambda value: tuple(map(int, value[6:].split(".")))))
