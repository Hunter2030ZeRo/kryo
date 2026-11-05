"""Check the verbatim CPU-random bridge with host-only native-call doubles."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / "userspace/libc/mlibc/sysdeps/cryolite/rust.cpp"
ERRNO = ROOT / "userspace/libc/abi/include/cryolite/errno.h"
NATIVE = ROOT / "userspace/libsys/include/cryolite/native.h"
SIGNATURE = 'extern "C" int __cryolite_rust_random('
GROUPS = [
    "empty requests do not access null or unreadable pointers or call the source",
    "oversized and nonempty-null requests fail before accessing the source",
    "all lengths from zero through 256 copy only the requested prefix",
    "partial native-word tails and unaligned destinations preserve guards",
    "the 256-byte limit uses exactly sixteen native calls",
    "successful all-zero words are accepted",
    "Busy and Unsupported preserve the destination at every batch position",
    "unexpected native errors map to EIO without publishing partial bytes",
    "a later independent request can succeed after a source failure",
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def function_body(source):
    if source.count(SIGNATURE) != 1:
        raise RuntimeError("Expected one CPU-random bridge definition")
    start = source.index(SIGNATURE)
    opening = source.index("{", start)
    # Skip comments and quoted literals while counting the function's braces.
    tokens = re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|[{}]', re.S)
    depth = 0
    for match in tokens.finditer(source, opening):
        token = match.group()
        if token == "{":
            depth += 1
        elif token == "}":
            depth -= 1
            if depth == 0:
                return source[start:match.end()] + "\n"
    raise RuntimeError("Unterminated CPU-random bridge definition")


PREFIX = r"""
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <cryolite/errno.h>

#define kryo_random_words unused_native_random_words
#include <cryolite/native.h>
#undef kryo_random_words

static_assert(EINVAL == 28 && EAGAIN == 6 && ENOTSUP == 61 && EIO == 29);
static_assert(sizeof(kryo_random_result::words) == 16);

static unsigned calls;
static unsigned failure_call;
static kryo_status failure_status;
static bool zero_words;
static unsigned char input[256];

static kryo_random_result kryo_random_words() {
    ++calls;
    kryo_random_result result{};
    if (calls == failure_call) {
        result.status = failure_status;
        result.words[0] = result.words[1] = UINT64_MAX;
    } else if (!zero_words) {
        if (calls > 16)
            __builtin_trap();
        memcpy(result.words, input + (calls - 1) * 16, sizeof(result.words));
    }
    return result;
}

static void reset() {
    calls = failure_call = 0;
    failure_status = 0;
    zero_words = false;
    for (size_t i = 0; i < sizeof(input); ++i)
        input[i] = static_cast<unsigned char>(i * 73 + 19);
}

struct Guarded {
    unsigned char before[19];
    unsigned char bytes[256];
    unsigned char after[23];
};

static unsigned case_count;

static bool check(bool passed, const char *name) {
    ++case_count;
    if (!passed)
        fprintf(stderr, "random bridge case %u failed: %s\n", case_count, name);
    return passed;
}

static bool all_equal(const void *data, size_t count, unsigned char expected) {
    auto bytes = static_cast<const unsigned char *>(data);
    for (size_t i = 0; i < count; ++i)
        if (bytes[i] != expected)
            return false;
    return true;
}
"""

MAIN = r"""
int main() {
    reset();
    if (!check(__cryolite_rust_random(nullptr, 0) == 0 && calls == 0, "empty null"))
        return 1;
    if (!check(__cryolite_rust_random(reinterpret_cast<void *>(uintptr_t{1}), 0) == 0
               && calls == 0, "empty unreadable pointer"))
        return 1;

    Guarded destination;
    memset(&destination, 0xa5, sizeof(destination));
    const size_t oversized[] = {257, SIZE_MAX};
    for (size_t size : oversized) {
        reset();
        if (!check(__cryolite_rust_random(destination.bytes, size) == EINVAL
                   && calls == 0 && all_equal(&destination, sizeof(destination), 0xa5),
                   "oversized request"))
            return 1;
    }
    const size_t null_sizes[] = {1, 256, 257};
    for (size_t size : null_sizes) {
        reset();
        if (!check(__cryolite_rust_random(nullptr, size) == EINVAL && calls == 0,
                   "nonempty null"))
            return 1;
    }

    for (size_t size = 0; size <= 256; ++size) {
        reset();
        memset(&destination, 0xa5, sizeof(destination));
        int error = __cryolite_rust_random(destination.bytes, size);
        if (!check(error == 0 && calls == (size + 15) / 16
                   && memcmp(destination.bytes, input, size) == 0
                   && all_equal(destination.bytes + size, sizeof(destination.bytes) - size, 0xa5)
                   && all_equal(destination.before, sizeof(destination.before), 0xa5)
                   && all_equal(destination.after, sizeof(destination.after), 0xa5),
                   "bounded prefix, native-word tail and guards"))
            return 1;
    }

    reset();
    zero_words = true;
    memset(&destination, 0xa5, sizeof(destination));
    if (!check(__cryolite_rust_random(destination.bytes, sizeof(destination.bytes)) == 0
               && calls == 16 && all_equal(destination.bytes, sizeof(destination.bytes), 0)
               && all_equal(destination.before, sizeof(destination.before), 0xa5)
               && all_equal(destination.after, sizeof(destination.after), 0xa5),
               "successful zero output"))
        return 1;

    struct Failure { kryo_status status; int error; } failures[] = {
        {KRYO_STATUS_BUSY, EAGAIN},
        {KRYO_STATUS_UNSUPPORTED, ENOTSUP},
        {KRYO_STATUS_PERMISSION_DENIED, EIO},
    };
    for (auto failure : failures) {
        for (unsigned position = 1; position <= 16; ++position) {
            reset();
            failure_call = position;
            failure_status = failure.status;
            memset(&destination, 0xa5, sizeof(destination));
            if (!check(__cryolite_rust_random(destination.bytes, sizeof(destination.bytes))
                       == failure.error && calls == position
                       && all_equal(&destination, sizeof(destination), 0xa5),
                       "source failure publishes no partial output and is not retried"))
                return 1;
        }
    }

    reset();
    memset(&destination, 0xa5, sizeof(destination));
    if (!check(__cryolite_rust_random(destination.bytes, 17) == 0 && calls == 2
               && memcmp(destination.bytes, input, 17) == 0
               && all_equal(destination.bytes + 17, sizeof(destination.bytes) - 17, 0xa5)
               && all_equal(destination.before, sizeof(destination.before), 0xa5)
               && all_equal(destination.after, sizeof(destination.after), 0xa5),
               "fresh request recovers after failure"))
        return 1;

    printf("random bridge: %u cases passed\n", case_count);
    return case_count == 314 ? 0 : 2;
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outputdir", type=Path, required=True)
    args = parser.parse_args()
    inputs = {path: path.read_bytes() for path in (SOURCE, ERRNO, NATIVE)}
    body = function_body(inputs[SOURCE].decode())
    output = args.outputdir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    harness = output / "random-runtime.cpp"
    harness.write_text(PREFIX + body + MAIN)
    checks = []
    environment = os.environ.copy()
    environment["ASAN_OPTIONS"] = "detect_leaks=0"
    for optimization in ("-O0", "-O2"):
        binary = output / f"random-{optimization[1:]}"
        command = ["clang++", "-std=c++23", optimization, "-Wall", "-Wextra", "-Werror",
                   "-fno-builtin", "-fno-omit-frame-pointer", "-fsanitize=address,undefined",
                   "-I", str(ROOT / "userspace/libc/abi/include"),
                   "-I", str(ROOT / "userspace/libsys/include"),
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
    if any(path.read_bytes() != content for path, content in inputs.items()):
        raise RuntimeError("Bridge or ABI input changed during execution; rerun the checks")
    evidence = {
        "source_inputs_sha256": {str(path.relative_to(ROOT)): hashlib.sha256(content).hexdigest()
                                 for path, content in inputs.items()},
        "function_body_sha256": hashlib.sha256(body.encode()).hexdigest(),
        "harness_sha256": digest(harness), "groups": GROUPS, "cases": 314, "checks": checks,
        "leak_sanitizer": "disabled: this bounded bridge allocates no heap storage",
        "scope": "host-only execution of the verbatim runtime bridge with deterministic native-call doubles; no CPU instruction, native ABI execution or entropy-quality proof",
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({"cases": 314, "optimized_runs": 2, "sanitizers": ["address", "undefined"],
                      "evidence": str(output / "evidence.json")}, indent=2))


if __name__ == "__main__":
    main()
