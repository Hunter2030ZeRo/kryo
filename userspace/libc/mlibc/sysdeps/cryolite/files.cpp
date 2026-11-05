#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdarg.h>
#include <stdlib.h>
#include <cryolite/file-protocol.h>
#include <cryolite/file-runtime.h>
#include <cryolite/native.h>
#include <mlibc/all-sysdeps.hpp>

namespace {

constexpr uintptr_t loan_start = 0x200000000000;
constexpr uintptr_t loan_end = 0x210000000000;
constexpr uint64_t file_rights = KRYO_RIGHT_SEND | KRYO_RIGHT_DUP | KRYO_RIGHT_TRANSFER;

struct Description {
    Description *next;
    uint64_t handle;
    uint64_t request;
    uintptr_t window;
    size_t references;
    int flags;
    alignas(4) uint32_t operation_lock;
    bool poisoned;
};

struct Descriptor {
    Description *owner;
    int flags;
    bool published;
};

Description *files;
Descriptor *descriptors;
size_t descriptor_capacity;
alignas(4) uint32_t table_lock;

[[noreturn]] void fail() {
    kryo_exit(253);
}

int native_error(kryo_status status) {
    switch (status) {
        case KRYO_STATUS_OK: return 0;
        case KRYO_STATUS_OUT_OF_RESOURCES: return ENOMEM;
        case KRYO_STATUS_PERMISSION_DENIED: return EACCES;
        case KRYO_STATUS_PEER_CLOSED: return EPIPE;
        case KRYO_STATUS_CANCELLED: return ECANCELED;
        case KRYO_STATUS_TIMED_OUT: return ETIMEDOUT;
        case KRYO_STATUS_BUSY: return EBUSY;
        default: return EIO;
    }
}

int service_error(uint64_t status) {
    switch (status) {
        case KRYO_FILE_OK: return 0;
        case KRYO_FILE_MALFORMED: return EPROTO;
        case KRYO_FILE_DENIED: return EACCES;
        case KRYO_FILE_RANGE: return EINVAL;
        case KRYO_FILE_UNSUPPORTED: return ENOTSUP;
        case KRYO_FILE_NOT_FOUND: return ENOENT;
        case KRYO_FILE_EXISTS: return EEXIST;
        case KRYO_FILE_NO_SPACE: return ENOSPC;
        case KRYO_FILE_IO: return EIO;
        case KRYO_FILE_UNAVAILABLE: return EPIPE;
        case KRYO_FILE_OVERFLOW: return EOVERFLOW;
        case KRYO_FILE_TOO_MANY_OPEN: return EMFILE;
        case KRYO_FILE_BAD_FILE: return EBADF;
        default: return EPROTO;
    }
}

void lock(uint32_t *word) {
    uint32_t expected = 0;
    if (__atomic_compare_exchange_n(word, &expected, 1, false,
            __ATOMIC_ACQUIRE, __ATOMIC_RELAXED))
        return;
    while (__atomic_exchange_n(word, 2, __ATOMIC_ACQUIRE)) {
        auto result = kryo_park(reinterpret_cast<uintptr_t>(word), 2);
        if (result.status == KRYO_STATUS_OUT_OF_RESOURCES) {
            if (kryo_yield().status)
                fail();
        } else if (result.status) {
            fail();
        }
    }
}

void unlock(uint32_t *word) {
    if (__atomic_exchange_n(word, 0, __ATOMIC_RELEASE) == 2
        && kryo_wake(reinterpret_cast<uintptr_t>(word), UINT64_MAX).status)
        fail();
}

void close_handle(uint64_t handle) {
    if (handle && kryo_close(handle).status)
        fail();
}

// The caller holds the table lock; moving slots never moves a description.
int ensure_capacity(size_t descriptor) {
    constexpr size_t maximum = static_cast<size_t>(INT_MAX) + 1;
    if (descriptor >= maximum)
        return EMFILE;
    if (descriptor < descriptor_capacity)
        return 0;
    auto capacity = descriptor_capacity ? descriptor_capacity : 8;
    while (capacity <= descriptor)
        capacity = capacity <= maximum / 2 ? capacity * 2 : maximum;
    auto table = static_cast<Descriptor *>(realloc(descriptors, capacity * sizeof(Descriptor)));
    if (!table)
        return ENOMEM;
    for (size_t index = descriptor_capacity; index < capacity; ++index)
        table[index] = {};
    descriptors = table;
    descriptor_capacity = capacity;
    return 0;
}

int reserve_slot(int minimum, int *out) {
    size_t descriptor = static_cast<size_t>(minimum);
    while (descriptor < descriptor_capacity && descriptors[descriptor].owner)
        ++descriptor;
    if (auto error = ensure_capacity(descriptor))
        return error;
    *out = static_cast<int>(descriptor);
    return 0;
}

Descriptor *find_descriptor(int descriptor) {
    if (descriptor < 0 || static_cast<size_t>(descriptor) >= descriptor_capacity)
        return nullptr;
    auto entry = &descriptors[descriptor];
    return entry->owner && entry->published ? entry : nullptr;
}

Description *reserve_file(int flags, int *descriptor, int *error) {
    auto record = static_cast<Description *>(calloc(1, sizeof(Description)));
    if (!record) {
        *error = ENOMEM;
        return nullptr;
    }
    record->flags = flags & (O_ACCMODE | O_APPEND | O_NONBLOCK | O_DSYNC | O_RSYNC | O_SYNC);
    record->references = 1;
    record->request = 1;
    lock(&table_lock);
    uintptr_t window = loan_start;
    for (;;) {
        bool used = false;
        for (auto current = files; current; current = current->next)
            used |= current->window == window;
        if (!used)
            break;
        window += KRYO_FILE_PAGE_SIZE;
        if (window == loan_end) {
            unlock(&table_lock);
            free(record);
            *error = ENOMEM;
            return nullptr;
        }
    }
    *error = reserve_slot(3, descriptor);
    if (*error) {
        unlock(&table_lock);
        free(record);
        return nullptr;
    }
    record->window = window;
    record->next = files;
    files = record;
    descriptors[*descriptor] = {record, flags & O_CLOEXEC ? FD_CLOEXEC : 0, false};
    unlock(&table_lock);
    return record;
}

Description *pin_file(int descriptor) {
    lock(&table_lock);
    auto entry = find_descriptor(descriptor);
    auto record = entry ? entry->owner : nullptr;
    if (record) {
        if (record->references == SIZE_MAX)
            fail();
        ++record->references;
    }
    unlock(&table_lock);
    return record;
}

void release_file(Description *record) {
    lock(&table_lock);
    if (--record->references) {
        unlock(&table_lock);
        return;
    }
    auto link = &files;
    while (*link != record)
        link = &(*link)->next;
    *link = record->next;
    Descriptor *retired_table = nullptr;
    if (!files) {
        retired_table = descriptors;
        descriptors = nullptr;
        descriptor_capacity = 0;
    }
    unlock(&table_lock);
    free(retired_table);
    close_handle(record->handle);
    free(record);
}

struct Operation {
    explicit Operation(Description *record) : file{record} {
        if (file)
            lock(&file->operation_lock);
    }

    ~Operation() {
        if (file) {
            unlock(&file->operation_lock);
            release_file(file);
        }
    }

    Description *file;
};

struct Loan {
    explicit Loan(Description *record) : file{record} {}

    int create() {
        auto result = kryo_memory_create(1);
        if (result.status)
            return native_error(result.status);
        handle = result.value;
        result = kryo_memory_map(handle, file->window, 0, KRYO_MEMORY_READ_WRITE);
        if (result.status)
            return native_error(result.status);
        mapped = true;
        return 0;
    }

    ~Loan() {
        if (mapped && kryo_memory_unmap(file->window).status)
            fail();
        close_handle(handle);
    }

    void copy_in(const void *source, size_t bytes) {
        auto input = static_cast<const unsigned char *>(source);
        auto output = reinterpret_cast<volatile unsigned char *>(file->window);
        for (size_t index = 0; index < bytes; ++index)
            output[index] = input[index];
    }

    void copy_out(void *destination, size_t bytes) {
        auto input = reinterpret_cast<const volatile unsigned char *>(file->window);
        auto output = static_cast<unsigned char *>(destination);
        for (size_t index = 0; index < bytes; ++index)
            output[index] = input[index];
    }

    Description *file;
    uint64_t handle = 0;
    bool mapped = false;
};

int next_request(Description *file, uint64_t *id) {
    if (file->poisoned)
        return EIO;
    if (file->request == UINT64_MAX)
        return EOVERFLOW;
    *id = file->request++;
    return 0;
}

void discard_grants(const kryo_ipc_result &reply) {
    if (reply.status)
        return;
    for (size_t index = 0; index < KRYO_IPC_CAPS; ++index)
        close_handle(reply.handles[index]);
}

int validate_reply(Description *file, const kryo_ipc_result &reply, uint64_t id, bool opening) {
    if (reply.status) {
        if (kryo_ipc_accepted(reply))
            file->poisoned = true;
        return native_error(reply.status);
    }
    auto status = reply.message.words[1];
    bool success = status == KRYO_FILE_OK;
    bool grant = opening && success;
    bool valid = status <= KRYO_FILE_BAD_FILE
        && reply.message.words[0] == id && reply.message.words[3] == 0
        && reply.count == static_cast<uint64_t>(grant)
        && (grant ? reply.handles[0] != 0 : reply.handles[0] == 0)
        && reply.handles[1] == 0
        && (success || reply.message.words[2] == 0);
    if (opening && success)
        valid &= reply.message.words[2] == 0;
    if (!valid) {
        discard_grants(reply);
        file->poisoned = true;
        return EPROTO;
    }
    if (grant) {
        auto info = kryo_handle_info(reply.handles[0]);
        if (info.status || info.kind != KRYO_OBJECT_ENDPOINT_GATE || info.rights != file_rights) {
            discard_grants(reply);
            file->poisoned = true;
            return EPROTO;
        }
    }
    return service_error(status);
}

int open_flags(int flags, mode_t mode, uint64_t *wire) {
    auto access = flags & O_ACCMODE;
    if ((flags & ~0x1ffff) || (access != O_RDONLY && access != O_WRONLY
            && access != O_RDWR && access != O_EXEC && access != O_SEARCH)
        || ((flags & O_EXCL) && !(flags & O_CREAT))
        || ((flags & O_TRUNC) && access != O_WRONLY && access != O_RDWR)
        || ((flags & O_CREAT) && (flags & O_DIRECTORY))
        || ((flags & O_CREAT) && (mode & ~07777u)))
        return EINVAL;
    constexpr int supported = O_ACCMODE | O_CREAT | O_EXCL | O_TRUNC | O_APPEND | O_CLOEXEC;
    if ((flags & ~supported) || access == O_EXEC || access == O_SEARCH)
        return ENOTSUP;
    uint64_t result = access == O_RDONLY ? KRYO_FILE_READ_ONLY
        : access == O_WRONLY ? KRYO_FILE_WRITE_ONLY : KRYO_FILE_READ_WRITE;
    if (flags & O_CREAT) result |= KRYO_FILE_CREATE;
    if (flags & O_EXCL) result |= KRYO_FILE_EXCLUSIVE;
    if (flags & O_TRUNC) result |= KRYO_FILE_TRUNCATE;
    if (flags & O_APPEND) result |= KRYO_FILE_APPEND;
    *wire = result | (static_cast<uint64_t>((flags & O_CREAT) ? mode : 0) << 32);
    return 0;
}

int transfer(int descriptor, void *buffer, size_t bytes, ssize_t *actual, bool writing) {
    Operation operation{pin_file(descriptor)};
    auto file = operation.file;
    if (!file || (writing ? (file->flags & O_ACCMODE) == O_RDONLY
                         : (file->flags & O_ACCMODE) == O_WRONLY))
        return EBADF;
    if (bytes > static_cast<size_t>(LONG_MAX))
        return EINVAL;
    if (!bytes) {
        *actual = 0;
        return 0;
    }
    auto count = bytes < KRYO_FILE_PAGE_SIZE ? bytes : KRYO_FILE_PAGE_SIZE;
    uint64_t id;
    if (auto error = next_request(file, &id))
        return error;
    Loan loan{file};
    if (auto error = loan.create())
        return error;
    if (writing)
        loan.copy_in(buffer, count);
    kryo_message message{{KRYO_FILE_HEADER(writing ? KRYO_FILE_WRITE : KRYO_FILE_READ), id, 0, count}};
    kryo_grant grant{loan.handle, static_cast<uint64_t>(KRYO_RIGHT_READ | KRYO_RIGHT_MAP)
        | (writing ? uint64_t{0} : uint64_t{KRYO_RIGHT_WRITE})};
    auto reply = kryo_ipc_call_exchange(file->handle, message, &grant, 1, 0);
    if (auto error = validate_reply(file, reply, id, false))
        return error;
    auto completed = reply.message.words[2];
    if (completed > count || (writing && !completed)) {
        file->poisoned = true;
        return EPROTO;
    }
    if (!writing)
        loan.copy_out(buffer, completed);
    *actual = static_cast<ssize_t>(completed);
    return 0;
}

} // namespace

extern "C" int __cryolite_file_open(const char *path, int flags, mode_t mode, int *descriptor) {
    auto root = __cryolite_file_namespace();
    if (!root)
        return ENOSYS;
    uint64_t wire;
    if (auto error = open_flags(flags, mode, &wire))
        return error;
    size_t length = 0;
    while (length <= KRYO_FILE_PAGE_SIZE && path[length])
        ++length;
    if (!length)
        return ENOENT;
    if (length > KRYO_FILE_PAGE_SIZE)
        return ENAMETOOLONG;
    int error = 0;
    int reserved_descriptor;
    auto file = reserve_file(flags, &reserved_descriptor, &error);
    if (!file)
        return error;
    {
        Loan loan{file};
        error = loan.create();
        if (!error) {
            loan.copy_in(path, length);
            auto id = file->request++;
            kryo_message message{{KRYO_FILE_HEADER(KRYO_FILE_OPEN), id, wire, length}};
            kryo_grant grant{loan.handle, KRYO_RIGHT_READ | KRYO_RIGHT_MAP};
            auto reply = kryo_ipc_call_exchange(root, message, &grant, 1, 1);
            error = validate_reply(file, reply, id, true);
            if (!error)
                file->handle = reply.handles[0];
        }
    }
    if (error) {
        lock(&table_lock);
        descriptors[reserved_descriptor] = {};
        unlock(&table_lock);
        release_file(file);
        return error;
    }
    lock(&table_lock);
    descriptors[reserved_descriptor].published = true;
    unlock(&table_lock);
    *descriptor = reserved_descriptor;
    return 0;
}

extern "C" int __cryolite_file_read(int descriptor, void *buffer, size_t bytes, ssize_t *actual) {
    return transfer(descriptor, buffer, bytes, actual, false);
}

extern "C" int __cryolite_file_write(int descriptor, const void *buffer, size_t bytes, ssize_t *actual) {
    return transfer(descriptor, const_cast<void *>(buffer), bytes, actual, true);
}

extern "C" int __cryolite_file_seek(int descriptor, off_t offset, int whence, off_t *actual) {
    Operation operation{pin_file(descriptor)};
    auto file = operation.file;
    if (!file)
        return EBADF;
    if (whence != SEEK_SET && whence != SEEK_CUR && whence != SEEK_END)
        return EINVAL;
    uint64_t id;
    if (auto error = next_request(file, &id))
        return error;
    kryo_message message{{KRYO_FILE_HEADER(KRYO_FILE_SEEK), id,
        static_cast<uint64_t>(offset), static_cast<uint64_t>(whence)}};
    auto reply = kryo_ipc_call_exchange(file->handle, message, nullptr, 0, 0);
    if (auto error = validate_reply(file, reply, id, false))
        return error;
    if (reply.message.words[2] > static_cast<uint64_t>(LONG_MAX)) {
        file->poisoned = true;
        return EPROTO;
    }
    *actual = static_cast<off_t>(reply.message.words[2]);
    return 0;
}

extern "C" int __cryolite_file_close(int descriptor) {
    lock(&table_lock);
    auto entry = find_descriptor(descriptor);
    if (!entry) {
        unlock(&table_lock);
        return EBADF;
    }
    auto file = entry->owner;
    *entry = {};
    unlock(&table_lock);
    release_file(file);
    return 0;
}

extern "C" int __cryolite_file_dup(int descriptor, int minimum, int flags, int *out) {
    lock(&table_lock);
    auto entry = find_descriptor(descriptor);
    if (!entry) {
        unlock(&table_lock);
        return EBADF;
    }
    if (minimum < 0 || (flags & ~FD_CLOEXEC)) {
        unlock(&table_lock);
        return EINVAL;
    }
    auto file = entry->owner;
    if (file->references == SIZE_MAX) {
        unlock(&table_lock);
        return ENOMEM;
    }
    int result;
    if (auto error = reserve_slot(minimum, &result)) {
        unlock(&table_lock);
        return error;
    }
    ++file->references;
    descriptors[result] = {file, flags, true};
    unlock(&table_lock);
    *out = result;
    return 0;
}

extern "C" int __cryolite_file_dup2(int descriptor, int flags, int destination) {
    lock(&table_lock);
    auto entry = find_descriptor(descriptor);
    if (!entry || destination < 0) {
        unlock(&table_lock);
        return EBADF;
    }
    if ((flags & ~O_CLOEXEC) || (descriptor == destination && flags)) {
        unlock(&table_lock);
        return EINVAL;
    }
    if (descriptor == destination) {
        unlock(&table_lock);
        return 0;
    }
    auto file = entry->owner;
    if (file->references == SIZE_MAX) {
        unlock(&table_lock);
        return ENOMEM;
    }
    if (auto error = ensure_capacity(static_cast<size_t>(destination))) {
        unlock(&table_lock);
        return error;
    }
    auto &target = descriptors[destination];
    // An in-progress Open owns its unpublished numerical reservation.
    if (target.owner && !target.published) {
        unlock(&table_lock);
        return EBUSY;
    }
    auto victim = target.owner;
    ++file->references;
    target = {file, flags & O_CLOEXEC ? FD_CLOEXEC : 0, true};
    unlock(&table_lock);
    if (victim)
        release_file(victim);
    return 0;
}

extern "C" int __cryolite_file_control(int descriptor, int command, int argument, int *out) {
    if (command == F_DUPFD || command == F_DUPFD_CLOEXEC)
        return __cryolite_file_dup(descriptor, argument,
            command == F_DUPFD_CLOEXEC ? FD_CLOEXEC : 0, out);
    lock(&table_lock);
    auto entry = find_descriptor(descriptor);
    if (!entry) {
        unlock(&table_lock);
        return EBADF;
    }
    int result = 0;
    int error = 0;
    switch (command) {
        case F_GETFD:
            result = entry->flags;
            break;
        case F_SETFD:
            if (argument & ~FD_CLOEXEC)
                error = EINVAL;
            else
                entry->flags = argument;
            break;
        case F_GETFL:
            result = entry->owner->flags;
            break;
        case F_SETFL:
            if (argument & ~0x1ffff)
                error = EINVAL;
            else if ((argument ^ entry->owner->flags)
                    & (O_APPEND | O_NONBLOCK | O_DSYNC | O_RSYNC | O_SYNC))
                error = ENOTSUP;
            break;
        case F_GETLK:
        case F_SETLK:
        case F_SETLKW:
        case F_GETOWN:
        case F_SETOWN:
            error = ENOTSUP;
            break;
        default:
            error = EINVAL;
            break;
    }
    unlock(&table_lock);
    if (!error)
        *out = result;
    return error;
}

extern "C" int __cryolite_file_isatty(int descriptor) {
    auto file = pin_file(descriptor);
    if (!file)
        return EBADF;
    release_file(file);
    return ENOTTY;
}

namespace mlibc {

int Sysdeps<Open>::operator()(const char *path, int flags, mode_t mode, int *descriptor) {
    return __cryolite_file_open(path, flags, mode, descriptor);
}

int Sysdeps<Read>::operator()(int descriptor, void *buffer, size_t bytes, ssize_t *actual) {
    return __cryolite_file_read(descriptor, buffer, bytes, actual);
}

int Sysdeps<Write>::operator()(int descriptor, const void *buffer, size_t bytes, ssize_t *actual) {
    return __cryolite_file_write(descriptor, buffer, bytes, actual);
}

int Sysdeps<Seek>::operator()(int descriptor, off_t offset, int whence, off_t *actual) {
    return __cryolite_file_seek(descriptor, offset, whence, actual);
}

int Sysdeps<Close>::operator()(int descriptor) {
    return __cryolite_file_close(descriptor);
}

int Sysdeps<Isatty>::operator()(int descriptor) {
    return __cryolite_file_isatty(descriptor);
}

#if __MLIBC_POSIX_OPTION
int Sysdeps<Dup2>::operator()(int descriptor, int flags, int destination) {
    return __cryolite_file_dup2(descriptor, flags, destination);
}

int Sysdeps<Fcntl>::operator()(int descriptor, int command, va_list arguments, int *result) {
    int argument = 0;
    switch (command) {
        case F_DUPFD:
        case F_DUPFD_CLOEXEC:
        case F_SETFD:
        case F_SETFL:
        case F_SETOWN:
            argument = va_arg(arguments, int);
            break;
    }
    return __cryolite_file_control(descriptor, command, argument, result);
}
#endif

} // namespace mlibc
