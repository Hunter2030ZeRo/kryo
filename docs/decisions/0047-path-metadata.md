# Namespace metadata lookup

Date: 2026-11-05. Status: experimental implementation.

PathInfo extends the version-1 file-service protocol with operation 10. The
existing namespace gate supplies authority. Open-description gates cannot make
namespace requests, and namespace gates cannot perform descriptor operations.
No kernel path lookup or new syscall is introduced.

## Request and result

The four words are `[header(PathInfo), request_id, link_policy, path_length]`.
Follow=0 resolves the final symbolic link; NoFollow=1 reports that link itself.
All other policy values are invalid. Exactly one READ+MAP memory grant carries
1–4096 path bytes at offset zero, without a terminator. Embedded NUL is invalid;
paths are byte strings and need not be UTF-8.

The service snapshots the complete path into owned storage before lookup. It
ends all shared accesses and closes accepted grants before replying. Success
uses the existing Info reply: actual size and kind/readonly attributes, with no
capabilities. Error replies carry no value, attributes or capabilities.
SymlinkLoop=13 maps to Cryolite ELOOP=32. Existing operations and values remain
unchanged; there is still no feature negotiation with older services.

Metadata lookup does not open the object, create it, change a cursor or require
read access to its contents. The provider reserves a private loan-window owner
without allocating a descriptor table slot. Concurrent queries and open files
retain separate windows through the existing ownership list. The borrowed
namespace capability is never stored as an owned file handle or closed by this
temporary owner. Every completion is validated before publishing output.

The private C hook is `__cryolite_file_stat(path, nofollow, info)`. It does not
define or emulate the full public C stat structure. Rust stat and lstat share
the existing checked path conversion and FileAttr decoder. `exists` returns
false only for NotFound; permission, resource, loop and service errors remain
errors. The infallible Path::exists method retains Rust's upstream behavior of
returning false on errors.

## Fixture and limits

The volatile service has fixed names. `/probe` exists only after creation;
`/readonly` contains immutable bytes; `/` is a directory with zero payload
bytes and allows the fixture's `/probe` creation. Immutable links are
`/probe-link -> /probe`, `/dangling -> /missing`, and `/cycle -> /cycle`.
Following is bounded to 40 links. Open and PathInfo use the same resolver;
exclusive creation checks the final link's existence before following it.
`/denied` is an explicit policy-denial fixture. Unknown names return NotFound.

These names exercise the contract; they are not a production namespace,
hierarchical path traversal, mount routing or a current working directory.
Directory opening/iteration, readlink, namespace mutation, file locks,
timestamps and genuine storage synchronization remain separate work.
BetAFS's host FileDevice still needs locks and file/directory durability; this
change does not establish that it runs in Kryo. The independent engine and ICE
repositories remain unchanged.

The [validation record](../validation/2026-11-05-path-metadata/README.md)
separates sanitized provider fault tests, resolver tests and actual native
execution, including concurrent lookups and wait-quota rejection/recovery.
