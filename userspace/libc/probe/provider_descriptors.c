#include <errno.h>
#include <fenv.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <cryolite/file-runtime.h>
#include <cryolite/native.h>

static void require(int condition, kryo_handle debug, unsigned stage) {
    if (!condition) {
        (void)kryo_debug_log(debug, 217, stage);
        kryo_exit(stage);
    }
}

static int control(int descriptor, int command, int argument,
                    kryo_handle debug, unsigned stage) {
    int value = -1;
    require(__cryolite_file_control(descriptor, command, argument, &value) == 0,
            debug, stage);
    return value;
}

static void position(int descriptor, off_t offset, int whence, off_t expected,
                      kryo_handle debug, unsigned stage) {
    off_t actual = -1;
    require(__cryolite_file_seek(descriptor, offset, whence, &actual) == 0
            && actual == expected, debug, stage);
}

static void read_bytes(int descriptor, const char *expected, size_t bytes,
                        kryo_handle debug, unsigned stage) {
    char buffer[32];
    ssize_t actual = -1;
    require(bytes <= sizeof(buffer)
            && __cryolite_file_read(descriptor, buffer, bytes, &actual) == 0
            && actual == (ssize_t)bytes && !memcmp(buffer, expected, bytes), debug, stage);
}

void check_provider_descriptors(kryo_handle debug) {
    int saved_errno = errno;
    fenv_t environment;
    require(fegetenv(&environment) == 0, debug, 800);

    int source;
    require(__cryolite_file_open("/probe", O_RDWR | O_CREAT | O_TRUNC | O_CLOEXEC,
            0666, &source) == 0 && source >= 3, debug, 801);
    require(control(source, F_GETFL, 0, debug, 802) == O_RDWR
            && control(source, F_GETFD, 0, debug, 803) == FD_CLOEXEC, debug, 804);
    static const char data[] = "abcdefghij";
    ssize_t actual = -1;
    require(__cryolite_file_write(source, data, sizeof(data) - 1, &actual) == 0
            && actual == (ssize_t)sizeof(data) - 1, debug, 805);
    position(source, 0, SEEK_SET, 0, debug, 806);

    int first;
    require(__cryolite_file_dup(source, 0, 0, &first) == 0 && first == 0, debug, 807);
    int second = control(source, F_DUPFD_CLOEXEC, 0, debug, 808);
    require(second == 1 && control(first, F_GETFD, 0, debug, 809) == 0
            && control(second, F_GETFD, 0, debug, 810) == FD_CLOEXEC, debug, 811);
    require(control(first, F_SETFD, FD_CLOEXEC, debug, 812) == 0
            && control(source, F_GETFD, 0, debug, 813) == FD_CLOEXEC
            && control(first, F_SETFD, 0, debug, 814) == 0
            && control(second, F_GETFD, 0, debug, 815) == FD_CLOEXEC, debug, 816);
    int ignored = -1;
    require(__cryolite_file_control(first, F_SETFD, 2, &ignored) == EINVAL
            && control(first, F_GETFD, 0, debug, 817) == 0, debug, 818);

    int high = control(source, F_DUPFD, 17, debug, 819);
    int high_cloexec = control(source, F_DUPFD_CLOEXEC, 17, debug, 820);
    require(high == 17 && high_cloexec == 18
            && control(high, F_GETFD, 0, debug, 821) == 0
            && control(high_cloexec, F_GETFD, 0, debug, 822) == FD_CLOEXEC, debug, 823);
    read_bytes(first, "ab", 2, debug, 824);
    read_bytes(second, "cd", 2, debug, 825);
    read_bytes(high, "ef", 2, debug, 826);
    position(high_cloexec, 0, SEEK_CUR, 6, debug, 827);
    require(__cryolite_file_close(source) == 0
            && __cryolite_file_control(source, F_GETFD, 0, &ignored) == EBADF, debug, 828);
    read_bytes(first, "gh", 2, debug, 829);
    require(__cryolite_file_close(first) == 0 && __cryolite_file_close(second) == 0
            && __cryolite_file_close(high) == 0, debug, 830);
    read_bytes(high_cloexec, "ij", 2, debug, 831);

    require(__cryolite_file_dup2(high_cloexec, 0, high_cloexec) == 0
            && control(high_cloexec, F_GETFD, 0, debug, 832) == FD_CLOEXEC
            && __cryolite_file_dup2(high_cloexec, O_CLOEXEC, high_cloexec) == EINVAL
            && control(high_cloexec, F_GETFD, 0, debug, 833) == FD_CLOEXEC, debug, 834);
    require(__cryolite_file_dup(high_cloexec, -1, 0, &ignored) == EINVAL
            && __cryolite_file_dup(high_cloexec, 0, 2, &ignored) == EINVAL
            && __cryolite_file_dup2(high_cloexec, 0, -1) == EBADF
            && __cryolite_file_dup2(high_cloexec, 0x20000, 2) == EINVAL, debug, 835);

    int replaced;
    require(__cryolite_file_open("/readonly", O_RDONLY, 0, &replaced) == 0
            && control(replaced, F_GETFL, 0, debug, 836) == O_RDONLY, debug, 837);
    require(__cryolite_file_dup2(high_cloexec, 0, replaced) == 0
            && control(replaced, F_GETFD, 0, debug, 838) == 0
            && control(replaced, F_GETFL, 0, debug, 839) == O_RDWR, debug, 840);
    position(replaced, 0, SEEK_SET, 0, debug, 841);
    read_bytes(replaced, "ab", 2, debug, 842);
    read_bytes(high_cloexec, "cd", 2, debug, 843);
    require(control(high_cloexec, F_SETFD, 0, debug, 844) == 0
            && __cryolite_file_dup2(replaced, O_CLOEXEC, high_cloexec) == 0
            && control(high_cloexec, F_GETFD, 0, debug, 845) == FD_CLOEXEC
            && control(replaced, F_GETFD, 0, debug, 846) == 0, debug, 847);

    require(__cryolite_file_control(replaced, F_SETFL, O_RDWR | O_APPEND, &ignored) == ENOTSUP
            && __cryolite_file_control(replaced, F_SETFL, O_RDWR | O_NONBLOCK, &ignored) == ENOTSUP
            && __cryolite_file_control(replaced, F_SETFL, O_RDWR | O_SYNC, &ignored) == ENOTSUP
            && control(high_cloexec, F_GETFL, 0, debug, 848) == O_RDWR, debug, 849);
    require(control(replaced, F_SETFL, O_CREAT | O_TRUNC | O_CLOEXEC, debug, 850) == 0
            && control(replaced, F_GETFL, 0, debug, 851) == O_RDWR
            && __cryolite_file_control(replaced, F_SETFL, 0x20000, &ignored) == EINVAL
            && __cryolite_file_control(replaced, F_GETLK, 0, &ignored) == ENOTSUP
            && __cryolite_file_control(replaced, 99, 0, &ignored) == EINVAL, debug, 852);

    // The configured private-VM budget cannot allocate an INT_MAX-sized table.
    require(__cryolite_file_dup(replaced, INT_MAX, 0, &ignored) == ENOMEM
            && __cryolite_file_dup2(replaced, 0, INT_MAX) == ENOMEM
            && control(high_cloexec, F_GETFD, 0, debug, 853) == FD_CLOEXEC, debug, 854);
    position(replaced, 0, SEEK_CUR, 4, debug, 855);

    // This is explicit temporary file redirection, not an inherited terminal.
    require(__cryolite_file_control(1, F_GETFD, 0, &ignored) == EBADF, debug, 856);
    position(replaced, 0, SEEK_SET, 0, debug, 857);
    require(__cryolite_file_dup2(replaced, O_CLOEXEC, 1) == 0, debug, 858);
    static const char redirected[] = "stdio-dup\n";
    clearerr(stdout);
    require(fputs(redirected, stdout) >= 0 && fflush(stdout) == 0 && !ferror(stdout), debug, 859);
    position(replaced, 0, SEEK_CUR, sizeof(redirected) - 1, debug, 860);
    position(replaced, 0, SEEK_SET, 0, debug, 861);
    read_bytes(replaced, redirected, sizeof(redirected) - 1, debug, 862);
    require(__cryolite_file_close(1) == 0
            && __cryolite_file_close(replaced) == 0
            && __cryolite_file_close(high_cloexec) == 0, debug, 863);
    clearerr(stdout);
    require(fesetenv(&environment) == 0
            && kryo_debug_log(debug, 216, 1).status == KRYO_STATUS_OK, debug, 864);
    errno = saved_errno;
}
