
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
FILE *fopen(const char *path, const char *mode) {
	int flags = mlibc::fd_file::parse_modestring(mode);

	// Allocate before Open so allocation failure never leaves an owned descriptor.
	auto storage = getAllocator().allocate(sizeof(mlibc::fd_file));
	if(!storage) {
		errno = ENOMEM;
		return nullptr;
	}
	int fd;
	if(int e = mlibc::sysdep<Open>(path, flags, 0666, &fd); e) {
		getAllocator().free(storage);
		errno = e;
		return nullptr;
	}

	return new (storage) mlibc::fd_file(fd, mlibc::file_dispose_cb<mlibc::fd_file>);
}

int abstract_file::_ensure_allocation() {
	if(__buffer_ptr)
		return 0;

	if(__buffer_size > SIZE_MAX - ungetBufferSize) {
		__status_bits |= __MLIBC_ERROR_BIT;
		return ENOMEM;
	}
	auto ptr = getAllocator().allocate(__buffer_size + ungetBufferSize);
	if(!ptr) {
		__status_bits |= __MLIBC_ERROR_BIT;
		return ENOMEM;
	}
	__buffer_ptr = reinterpret_cast<char *>(ptr) + ungetBufferSize;
	__unget_ptr = __buffer_ptr;
	return 0;
}

int abstract_file::read(char *buffer, size_t max_size, size_t *actual_size) {
	__ensure(max_size);

	if(int e = _init_bufmode(); e)
		return e;

	if (__unget_ptr != __buffer_ptr) {
		auto chunk = frg::min(max_size, (size_t)(__buffer_ptr - __unget_ptr));
		memcpy(buffer, __unget_ptr, chunk);
		__unget_ptr += chunk;
		// Publish pushback as a short read before any later I/O can fail.
		*actual_size = chunk;
		return 0;
	}

	if(globallyDisableBuffering || _bufmode == buffer_mode::no_buffer) {
		size_t io_size;
		if(int e = io_read(buffer, max_size, &io_size); e) {
			__status_bits |= __MLIBC_ERROR_BIT;
			return e;
		}
		if(!io_size)
			__status_bits |= __MLIBC_EOF_BIT;
		*actual_size = io_size;
		return 0;
	}

	__io_mode = 0;

	// Clear the buffer, then buffer new data.
	if(__offset == __valid_limit) {
		// TODO: We only have to write-back/reset if __valid_limit reaches the buffer end.
		if(int e = _write_back(); e)
			return e;
		if(int e = _reset(); e)
			return e;

		// Perform a read-ahead.
		if(int e = _ensure_allocation(); e)
			return e;
		size_t io_size;
		if(int e = io_read(__buffer_ptr, __buffer_size, &io_size); e) {
			__status_bits |= __MLIBC_ERROR_BIT;
			return e;
		}
		if(!io_size) {
			__status_bits |= __MLIBC_EOF_BIT;
			*actual_size = 0;
			return 0;
		}

		__io_offset = io_size;
		__valid_limit = io_size;
	}

	// Return data from the buffer.
	__ensure(__offset < __valid_limit);

	auto chunk = frg::min(size_t(__valid_limit - __offset), max_size);
	memcpy(buffer, __buffer_ptr + __offset, chunk);
	__offset += chunk;

	*actual_size = chunk;
	return 0;
}

size_t fread_unlocked(void *buffer, size_t size, size_t count, FILE *file_base) {
	auto file = static_cast<mlibc::abstract_file *>(file_base);
	if(!size || !count)
		return 0;

	// Distinguish two cases here: If the object size is one, we perform byte-wise reads.
	// Otherwise, we try to read each object individually.
	if(size == 1) {
		size_t progress = 0;
		while(progress < count) {
			size_t chunk;
			if(int e = file->read((char *)buffer + progress,
					count - progress, &chunk)) {
				file->__status_bits |= __MLIBC_ERROR_BIT;
				errno = e;
				return progress;
			}else if(!chunk) {
				// TODO: Handle eof.
				break;
			}

			progress += chunk;
		}

		return progress;
	}else{
		for(size_t i = 0; i < count; i++) {
			size_t progress = 0;
			while(progress < size) {
				size_t chunk;
				if(int e = file->read((char *)buffer + i * size + progress,
						size - progress, &chunk)) {
					file->__status_bits |= __MLIBC_ERROR_BIT;
					errno = e;
					return i;
				}else if(!chunk) {
					// TODO: Handle eof.
					break;
				}

				progress += chunk;
			}

			if(progress < size)
				return i;
		}

		return count;
	}
}

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
