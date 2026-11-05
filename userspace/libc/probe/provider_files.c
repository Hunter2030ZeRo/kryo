#include <errno.h>
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <cryolite/file.h>
#include <cryolite/file-protocol.h>
#include <cryolite/file-runtime.h>
#include <cryolite/native.h>

static unsigned char content[9000];
static unsigned char observed[9017];

static void require(int condition, kryo_handle debug, unsigned stage) {
    if (!condition) {
        (void)kryo_debug_log(debug, 215, stage);
        kryo_exit(stage);
    }
}

static kryo_message request(kryo_u64 operation, kryo_u64 id,
                            kryo_u64 first, kryo_u64 second) {
    kryo_message message = {{KRYO_FILE_HEADER(operation), id, first, second}};
    return message;
}

static void response(kryo_ipc_result result, kryo_u64 id, kryo_u64 status,
                      kryo_u64 value, kryo_handle debug, unsigned stage) {
    require(result.status == KRYO_STATUS_OK && result.message.words[0] == id
            && result.message.words[1] == status && result.message.words[2] == value
            && result.message.words[3] == 0 && result.count == 0
            && result.handles[0] == 0 && result.handles[1] == 0, debug, stage);
}

static void check_native_description(kryo_handle debug) {
    const kryo_u64 view = UINT64_C(0x30100000);
    kryo_handle namespace = __cryolite_file_namespace();
    require(namespace != 0, debug, 750);
    kryo_result created = kryo_memory_create(1);
    require(created.status == KRYO_STATUS_OK, debug, 751);
    kryo_handle memory = created.value;
    require(kryo_memory_map(memory, view, 0, 1).status == KRYO_STATUS_OK, debug, 752);
    unsigned char *bytes = (unsigned char *)(uintptr_t)view;
    memcpy(bytes, "/probe", 6);
    kryo_grant input = {memory, KRYO_RIGHT_READ | KRYO_RIGHT_MAP};
    kryo_grant output = {memory, KRYO_RIGHT_READ | KRYO_RIGHT_WRITE | KRYO_RIGHT_MAP};
    kryo_ipc_result opened = kryo_ipc_call_exchange(namespace,
            request(KRYO_FILE_OPEN, 50, KRYO_FILE_READ_ONLY, 6), &input, 1, 1);
    require(opened.status == KRYO_STATUS_OK && opened.message.words[0] == 50
            && opened.message.words[1] == KRYO_FILE_OK && opened.message.words[2] == 0
            && opened.message.words[3] == 0 && opened.count == 1
            && opened.handles[0] != 0 && opened.handles[1] == 0, debug, 753);
    kryo_handle file = opened.handles[0];
    kryo_handle_info_result info = kryo_handle_info(file);
    require(info.status == KRYO_STATUS_OK && info.kind == KRYO_OBJECT_ENDPOINT_GATE
            && info.rights == (KRYO_RIGHT_SEND | KRYO_RIGHT_DUP | KRYO_RIGHT_TRANSFER), debug, 754);
    kryo_result copied = kryo_duplicate(file, KRYO_RIGHT_SEND);
    require(copied.status == KRYO_STATUS_OK, debug, 755);
    kryo_handle alias = copied.value;

    response(kryo_ipc_call_exchange(namespace, request(KRYO_FILE_SEEK, 51, 0, SEEK_SET), NULL, 0, 0),
            51, KRYO_FILE_DENIED, 0, debug, 756);
    response(kryo_ipc_call_exchange(file, request(KRYO_FILE_OPEN, 52, 0, 6), &input, 1, 0),
            52, KRYO_FILE_DENIED, 0, debug, 757);
    response(kryo_ipc_call_exchange(file, request(KRYO_FILE_READ, 53, 4095, 2), &output, 1, 0),
            53, KRYO_FILE_RANGE, 0, debug, 758);
    response(kryo_ipc_call_exchange(file, request(KRYO_FILE_READ, 54, 0, 2), &input, 1, 0),
            54, KRYO_FILE_DENIED, 0, debug, 759);
    response(kryo_ipc_call_exchange(file, request(KRYO_FILE_WRITE, 55, 0, 2), &input, 1, 0),
            55, KRYO_FILE_DENIED, 0, debug, 760);

    response(kryo_ipc_call_exchange(file, request(KRYO_FILE_READ, 56, 0, 2), &output, 1, 0),
            56, KRYO_FILE_OK, 2, debug, 761);
    require(!memcmp(bytes, content, 2), debug, 762);
    response(kryo_ipc_call_exchange(alias, request(KRYO_FILE_READ, 57, 0, 2), &output, 1, 0),
            57, KRYO_FILE_OK, 2, debug, 763);
    require(!memcmp(bytes, content + 2, 2), debug, 764);
    require(kryo_close(file).status == KRYO_STATUS_OK, debug, 765);
    response(kryo_ipc_call_exchange(alias, request(KRYO_FILE_READ, 58, 0, 2), &output, 1, 0),
            58, KRYO_FILE_OK, 2, debug, 766);
    require(!memcmp(bytes, content + 4, 2), debug, 767);
    require(kryo_close(alias).status == KRYO_STATUS_OK
            && kryo_memory_unmap(view).status == KRYO_STATUS_OK
            && kryo_close(memory).status == KRYO_STATUS_OK, debug, 768);
}

void check_provider_files(kryo_handle debug) {
    int saved_errno = errno;
    fenv_t environment;
    require(fegetenv(&environment) == 0, debug, 700);
    for (size_t index = 0; index < sizeof(content); ++index)
        content[index] = (unsigned char)(index * 37u + 11u);

    errno = 0;
    require(fopen("/missing", "r") == NULL && errno == ENOENT, debug, 701);
    FILE *file = fopen("/probe", "w+");
    require(file != NULL, debug, 702);
    require(fwrite(content, 1, sizeof(content), file) == sizeof(content)
            && fflush(file) == 0 && ftell(file) == (long)sizeof(content), debug, 703);
    require(fseek(file, 0, SEEK_SET) == 0 && ftell(file) == 0, debug, 704);
    memset(observed, 0, sizeof(observed));
    require(fread(observed, 1, sizeof(observed), file) == sizeof(content)
            && !memcmp(observed, content, sizeof(content)) && feof(file) && !ferror(file), debug, 705);
    require(fread(observed, 1, 1, file) == 0 && feof(file), debug, 706);
    require(fseek(file, -5, SEEK_END) == 0 && !feof(file)
            && fread(observed, 1, 5, file) == 5
            && !memcmp(observed, content + sizeof(content) - 5, 5), debug, 707);
    errno = 0;
    require(fseek(file, -1, SEEK_SET) == -1 && errno == EINVAL, debug, 708);
    clearerr(file);
    require(fclose(file) == 0, debug, 709);

    FILE *first = fopen("/probe", "r");
    FILE *second = fopen("/probe", "r");
    require(first && second, debug, 710);
    require(fgetc(first) == content[0] && fgetc(first) == content[1]
            && fgetc(second) == content[0] && ftell(first) == 2
            && ftell(second) == 1, debug, 711);
    require(fclose(first) == 0 && fclose(second) == 0, debug, 712);

    static const unsigned char tail[] = {0xf1, 0x32, 0x00, 0x7f, 0x80, 0x55, 0xaa};
    file = fopen("/probe", "a+");
    require(file && fseek(file, 0, SEEK_SET) == 0, debug, 713);
    require(fwrite(tail, 1, sizeof(tail), file) == sizeof(tail) && fflush(file) == 0
            && ftell(file) == (long)(sizeof(content) + sizeof(tail)), debug, 714);
    require(fseek(file, 0, SEEK_SET) == 0
            && fread(observed, 1, sizeof(observed), file) == sizeof(content) + sizeof(tail)
            && !memcmp(observed, content, sizeof(content))
            && !memcmp(observed + sizeof(content), tail, sizeof(tail)), debug, 715);
    require(fclose(file) == 0, debug, 716);

    file = fopen("/readonly", "r");
    static const char read_only[] = "read-only fixture\n";
    require(file && fread(observed, 1, sizeof(observed), file) == sizeof(read_only) - 1
            && !memcmp(observed, read_only, sizeof(read_only) - 1)
            && fclose(file) == 0, debug, 717);
    errno = 0;
    require(fopen("/readonly", "r+") == NULL && errno == EACCES, debug, 718);

    int fd;
    ssize_t transferred = -1;
    require(__cryolite_file_open("/readonly", O_RDONLY, 0, &fd) == 0, debug, 719);
    require(__cryolite_file_isatty(fd) == ENOTTY
            && __cryolite_file_write(fd, content, 1, &transferred) == EBADF, debug, 720);
    require(__cryolite_file_close(fd) == 0
            && __cryolite_file_close(fd) == EBADF
            && __cryolite_file_read(fd, observed, 1, &transferred) == EBADF, debug, 721);

    check_native_description(debug);
    off_t position = -1;
    require(__cryolite_file_open("/probe", O_RDWR, 0, &fd) == 0
            && __cryolite_file_seek(fd, 16382, SEEK_SET, &position) == 0
            && position == 16382, debug, 726);
    require(__cryolite_file_write(fd, content, 4, &transferred) == 0
            && transferred == 2, debug, 727);
    require(__cryolite_file_seek(fd, 0, SEEK_CUR, &position) == 0
            && position == 16384, debug, 728);
    require(__cryolite_file_write(fd, content, 1, &transferred) == ENOSPC
            && __cryolite_file_close(fd) == 0, debug, 729);
    file = fopen("/probe", "w");
    require(file && fclose(file) == 0, debug, 722);
    file = fopen("/probe", "r");
    require(file && fgetc(file) == EOF && feof(file) && !ferror(file)
            && fclose(file) == 0, debug, 723);

    // Leave one native descriptor to actual process-handle retirement. The peer
    // cannot finish until it receives this gate's final-authority notification.
    require(__cryolite_file_open("/probe", O_RDONLY, 0, &fd) == 0, debug, 724);
    require(fesetenv(&environment) == 0
            && kryo_debug_log(debug, 214, 1).status == KRYO_STATUS_OK, debug, 725);
    errno = saved_errno;
}
