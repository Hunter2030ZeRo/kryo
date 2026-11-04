"""Test pinned stdio function bodies with host-only allocator/Open/Read doubles."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

PORT = Path(__file__).resolve().parents[1]
REVISION = "07cfd943977737b8147c1289922c23479feef53d"
ARCHIVE_SHA = "6393f00bf7f3c6eead809f02e60a8f29fd031822566c3c050d7a80c299fb3558"
FUNCTIONS = {
    "fopen": ("options/ansi/generic/file-io.cpp", "FILE *fopen("),
    "buffer": ("options/ansi/generic/file-io.cpp", "int abstract_file::_ensure_allocation("),
    "read": ("options/ansi/generic/file-io.cpp", "int abstract_file::read("),
    "fread": ("options/ansi/generic/stdio.cpp", "size_t fread_unlocked("),
}
CASES = [
    "FILE allocation failure before Open", "Open error releases FILE storage",
    "FILE construction after successful Open", "buffer allocation failure",
    "buffer size overflow", "buffer allocation and reuse",
    "byte read error preserves completed bytes", "object read error preserves completed elements",
    "zero size skips Read", "zero count skips Read",
    "byte read EOF preserves completed bytes", "object read EOF preserves completed elements",
    "unbuffered pushback before EIO", "buffered pushback before EIO",
    "unbuffered pushback before EOF", "buffered pushback before EOF",
    "unbuffered incomplete object after pushback", "buffered incomplete object after pushback",
    "unbuffered complete object after pushback", "buffered complete object after pushback",
    "pushback before write-back failure", "pushback before reset failure",
    "buffered read allocation failure", "mode failure preserves pending pushback",
    "zero-size read preserves pending pushback",
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def function_body(source, signature):
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end] + "\n"


PREFIX = r"""
#include <stddef.h>
#include <stdint.h>
#include <errno.h>
#include <new>
#include <string.h>

#define __MLIBC_ERROR_BIT 1
#define __MLIBC_EOF_BIT 2
#define __ensure(condition) do { if (!(condition)) __builtin_trap(); } while (0)
namespace frg {
template<typename T> T min(T a, T b) { return a < b ? a : b; }
}
static constexpr bool globallyDisableBuffering = false;
struct Open {};
struct TestAllocator {
    alignas(max_align_t) unsigned char storage[512];
    bool fail = false;
    unsigned allocations = 0;
    unsigned releases = 0;
    void *allocate(size_t bytes) {
        ++allocations;
        return fail || bytes > sizeof(storage) ? nullptr : storage;
    }
    void free(void *pointer) {
        if (pointer == storage)
            ++releases;
        errno = ERANGE;
    }
};
static TestAllocator allocator;
static TestAllocator &getAllocator() { return allocator; }
static unsigned open_calls;
static unsigned constructions;
static int open_error;

namespace mlibc {
enum class buffer_mode { no_buffer, full_buffer };
struct abstract_file {
    char *__buffer_ptr = nullptr;
    char *__unget_ptr = nullptr;
    size_t __buffer_size = 32;
    unsigned __status_bits = 0;
    size_t __offset = 0;
    size_t __valid_limit = 0;
    size_t __io_offset = 0;
    int __io_mode = 0;
    buffer_mode _bufmode = buffer_mode::no_buffer;
    int mode_error = 0;
    int writeback_error = 0;
    int reset_error = 0;
    int chunks[4] = {};
    unsigned reads = 0;
    int _ensure_allocation();
    int read(char *buffer, size_t size, size_t *actual);
    int _init_bufmode() { return mode_error; }
    int _write_back() { return writeback_error; }
    int _reset() {
        if (reset_error)
            return reset_error;
        __offset = __valid_limit = __io_offset = 0;
        return 0;
    }
    int io_read(char *buffer, size_t size, size_t *actual) {
        int chunk = chunks[reads++];
        if (chunk < 0) {
            __status_bits |= __MLIBC_ERROR_BIT;
            return -chunk;
        }
        *actual = size < size_t(chunk) ? size : size_t(chunk);
        for (size_t i = 0; i < *actual; ++i)
            buffer[i] = 'Q';
        return 0;
    }
};
struct fd_file : abstract_file {
    int descriptor;
    fd_file(int fd, void (*)(abstract_file *)) : descriptor(fd) { ++constructions; }
    static int parse_modestring(const char *) { return 0; }
};
template<typename T> void file_dispose_cb(abstract_file *) {}
template<typename T> int sysdep(const char *, int, int, int *fd) {
    ++open_calls;
    if (open_error)
        return open_error;
    *fd = 37;
    return 0;
}
}
using FILE = mlibc::abstract_file;
using mlibc::abstract_file;
static constexpr size_t ungetBufferSize = 8;
static abstract_file pushed(char *storage, mlibc::buffer_mode mode) {
    abstract_file file;
    file.__unget_ptr = storage;
    file.__buffer_ptr = storage + 1;
    file._bufmode = mode;
    return file;
}
"""

MAIN = r"""
int main() {
    allocator.fail = true;
    errno = 0;
    if (fopen("test", "r") || errno != ENOMEM || open_calls || constructions)
        return 1;

    allocator = TestAllocator{};
    open_error = EIO;
    errno = 0;
    if (fopen("test", "r") || errno != EIO || open_calls != 1
            || allocator.releases != 1 || constructions)
        return 2;

    allocator = TestAllocator{};
    open_error = 0;
    FILE *opened = fopen("test", "r");
    if (!opened || constructions != 1 || open_calls != 2
            || static_cast<mlibc::fd_file *>(opened)->descriptor != 37)
        return 3;
    static_cast<mlibc::fd_file *>(opened)->~fd_file();

    allocator = TestAllocator{};
    allocator.fail = true;
    abstract_file failed;
    if (failed._ensure_allocation() != ENOMEM || failed.__buffer_ptr
            || failed.__unget_ptr || !(failed.__status_bits & __MLIBC_ERROR_BIT))
        return 4;

    allocator = TestAllocator{};
    abstract_file overflow;
    overflow.__buffer_size = SIZE_MAX - ungetBufferSize + 1;
    if (overflow._ensure_allocation() != ENOMEM || allocator.allocations
            || overflow.__buffer_ptr || overflow.__unget_ptr
            || !(overflow.__status_bits & __MLIBC_ERROR_BIT))
        return 5;

    abstract_file buffered;
    if (buffered._ensure_allocation() || buffered.__buffer_ptr !=
            reinterpret_cast<char *>(allocator.storage) + ungetBufferSize
            || buffered.__unget_ptr != buffered.__buffer_ptr
            || buffered._ensure_allocation() || allocator.allocations != 1)
        return 6;

    char buffer[16] = {};
    abstract_file bytes;
    bytes.chunks[0] = 3;
    bytes.chunks[1] = -EIO;
    errno = 0;
    if (fread_unlocked(buffer, 1, 8, &bytes) != 3 || errno != EIO
            || bytes.reads != 2 || !(bytes.__status_bits & __MLIBC_ERROR_BIT)
            || buffer[0] != 'Q' || buffer[2] != 'Q')
        return 7;

    abstract_file objects;
    objects.chunks[0] = 2;
    objects.chunks[1] = 1;
    objects.chunks[2] = -EIO;
    errno = 0;
    if (fread_unlocked(buffer, 2, 4, &objects) != 1 || errno != EIO
            || objects.reads != 3 || !(objects.__status_bits & __MLIBC_ERROR_BIT))
        return 8;

    abstract_file empty;
    if (fread_unlocked(buffer, 0, 8, &empty) || empty.reads)
        return 9;
    if (fread_unlocked(buffer, 8, 0, &empty) || empty.reads)
        return 10;

    abstract_file byte_eof;
    byte_eof.chunks[0] = 3;
    if (fread_unlocked(buffer, 1, 8, &byte_eof) != 3 || byte_eof.reads != 2)
        return 11;
    abstract_file object_eof;
    object_eof.chunks[0] = 2;
    object_eof.chunks[1] = 1;
    if (fread_unlocked(buffer, 2, 4, &object_eof) != 1 || object_eof.reads != 3)
        return 12;

    for (unsigned buffered = 0; buffered < 2; ++buffered) {
        auto mode = buffered ? mlibc::buffer_mode::full_buffer : mlibc::buffer_mode::no_buffer;
        char storage[2] = {'Q', 0};
        auto error = pushed(storage, mode);
        error.chunks[0] = -EIO;
        errno = 0;
        if (fread_unlocked(buffer, 1, 2, &error) != 1 || errno != EIO
                || error.__unget_ptr != error.__buffer_ptr || error.reads != 1
                || !(error.__status_bits & __MLIBC_ERROR_BIT) || buffer[0] != 'Q')
            return 13 + buffered;

        auto eof = pushed(storage, mode);
        errno = 0;
        if (fread_unlocked(buffer, 1, 2, &eof) != 1 || errno
                || !(eof.__status_bits & __MLIBC_EOF_BIT) || eof.reads != 1)
            return 15 + buffered;

        auto incomplete = pushed(storage, mode);
        incomplete.chunks[0] = -EIO;
        errno = 0;
        if (fread_unlocked(buffer, 2, 1, &incomplete) || errno != EIO
                || incomplete.__unget_ptr != incomplete.__buffer_ptr || buffer[0] != 'Q')
            return 17 + buffered;

        auto complete = pushed(storage, mode);
        complete.chunks[0] = 1;
        complete.chunks[1] = -EIO;
        errno = 0;
        if (fread_unlocked(buffer, 2, 2, &complete) != 1 || errno != EIO
                || complete.reads != 2 || buffer[0] != 'Q' || buffer[1] != 'Q')
            return 19 + buffered;
    }

    char storage[2] = {'Q', 0};
    auto writeback = pushed(storage, mlibc::buffer_mode::full_buffer);
    writeback.writeback_error = EIO;
    errno = 0;
    if (fread_unlocked(buffer, 1, 2, &writeback) != 1 || errno != EIO
            || writeback.reads || !(writeback.__status_bits & __MLIBC_ERROR_BIT))
        return 21;
    auto reset = pushed(storage, mlibc::buffer_mode::full_buffer);
    reset.reset_error = EIO;
    errno = 0;
    if (fread_unlocked(buffer, 1, 2, &reset) != 1 || errno != EIO
            || reset.reads || !(reset.__status_bits & __MLIBC_ERROR_BIT))
        return 22;

    allocator.fail = true;
    abstract_file read_oom;
    read_oom._bufmode = mlibc::buffer_mode::full_buffer;
    errno = 0;
    if (fread_unlocked(buffer, 1, 2, &read_oom) || errno != ENOMEM
            || read_oom.reads || !(read_oom.__status_bits & __MLIBC_ERROR_BIT))
        return 23;
    auto mode_error = pushed(storage, mlibc::buffer_mode::full_buffer);
    mode_error.mode_error = EIO;
    errno = 0;
    if (fread_unlocked(buffer, 1, 2, &mode_error) || errno != EIO
            || mode_error.__unget_ptr != storage || mode_error.reads)
        return 24;
    auto zero = pushed(storage, mlibc::buffer_mode::no_buffer);
    if (fread_unlocked(buffer, 0, 2, &zero) || zero.__unget_ptr != storage || zero.reads)
        return 25;
    return 0;
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True,
                        help="generated mlibc source with all Cryolite patches applied")
    parser.add_argument("--outputdir", type=Path, required=True)
    args = parser.parse_args()
    source = args.source.resolve()
    pins = json.loads((PORT / "sources.json").read_text())
    assert pins["mlibc"]["revision"] == REVISION
    assert pins["mlibc"]["archive_sha256"] == ARCHIVE_SHA
    assert (source.parent / "source-owner").read_text() == "cryolite-pinned-mlibc-v1\n"
    manifest = json.loads((source.parent / "source-manifest.json").read_text())
    bodies = {}
    inputs = {}
    for name, (path, signature) in FUNCTIONS.items():
        inputs[path] = digest(source / path)
        assert manifest[path]["sha256"] == inputs[path], path
        bodies[name] = function_body((source / path).read_text(), signature)
    header = "options/ansi/include/mlibc/file-io.hpp"
    inputs[header] = digest(source / header)
    assert manifest[header]["sha256"] == inputs[header]
    assert "int _ensure_allocation();" in (source / header).read_text()

    output = args.outputdir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    harness = output / "stdio-functions.cpp"
    harness.write_text(PREFIX + "\n".join(bodies.values()) + MAIN)
    checks = []
    environment = os.environ.copy()
    # The host runner restricts ptrace; explicit ownership counters cover cleanup.
    environment["ASAN_OPTIONS"] = "detect_leaks=0"
    for optimization in ("-O0", "-O2"):
        binary = output / f"stdio-{optimization[1:]}"
        command = ["clang++", "-std=c++23", optimization, "-Wall", "-Wextra", "-Werror",
                   "-fno-builtin", "-fno-omit-frame-pointer", "-fsanitize=address,undefined",
                   str(harness), "-o", str(binary)]
        for name, invocation in (("compile", command), ("run", [str(binary)])):
            completed = subprocess.run(invocation, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       env=environment)
            log = output / f"{optimization[1:]}-{name}.log"
            log.write_bytes(completed.stdout)
            checks.append({"name": f"{optimization}-{name}", "command": invocation,
                           "exit_code": completed.returncode, "log": log.name,
                           "log_sha256": digest(log)})
            if completed.returncode:
                raise RuntimeError(f"{optimization} {name} failed; see {log}")
    evidence = {
        "mlibc_revision": REVISION, "mlibc_archive_sha256": pins["mlibc"]["archive_sha256"],
        "patch_sha256": digest(PORT / "patches/0009-fallible-stdio-storage.patch"),
        "target_source_inputs": inputs,
        "function_body_sha256": {name: hashlib.sha256(body.encode()).hexdigest()
                                 for name, body in bodies.items()},
        "harness_sha256": digest(harness), "cases": CASES, "checks": checks,
        "leak_sanitizer": "disabled: restricted host ptrace; FILE cleanup checked by allocator counters",
        "scope": "host-only execution of verbatim target function bodies with test allocator/Open/Read doubles; not native file-service or hosted ABI integration",
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({"cases": len(CASES), "optimized_runs": 2, "sanitizers": ["address", "undefined"],
                      "evidence": str(output / "evidence.json")}, indent=2))


if __name__ == "__main__":
    main()
