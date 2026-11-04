#ifndef CRYOLITE_DRAFT_FILES_H
#define CRYOLITE_DRAFT_FILES_H

/* Compile fixture for a proposed ABI; not an installed libc header. */
#ifdef __cplusplus
#define DRAFT_FILE_ASSERT(condition) static_assert(condition, #condition)
#define DRAFT_FILE_ALIGN(type) alignof(type)
#else
#define DRAFT_FILE_ASSERT(condition) _Static_assert(condition, #condition)
#define DRAFT_FILE_ALIGN(type) _Alignof(type)
#endif
#define DRAFT_FILE_OFFSET(type, member) __builtin_offsetof(type, member)

#ifndef CRYOLITE_DRAFT_TIMESPEC
#define CRYOLITE_DRAFT_TIMESPEC
struct draft_timespec {
    long tv_sec;
    long tv_nsec;
};
#endif

struct draft_stat {
    unsigned long st_dev;
    unsigned long st_ino;
    unsigned long st_nlink;
    unsigned int st_mode;
    unsigned int st_uid;
    unsigned int st_gid;
    unsigned int __reserved0;
    unsigned long st_rdev;
    long st_size;
    long st_blksize;
    long st_blocks;
    struct draft_timespec st_atim;
    struct draft_timespec st_mtim;
    struct draft_timespec st_ctim;
    unsigned long __reserved1;
};

struct draft_flock {
    short l_type;
    short l_whence;
    unsigned int __reserved0;
    long l_start;
    long l_len;
    int l_pid;
    unsigned int __reserved1;
};

typedef unsigned long draft_nfds_t;
struct draft_pollfd {
    int fd;
    short events;
    short revents;
};

enum draft_idtype {
    DRAFT_P_ALL = 0,
    DRAFT_P_PID = 1,
    DRAFT_P_PGID = 2
};

#define DRAFT_S_IFMT 0170000
#define DRAFT_S_IFIFO 0010000
#define DRAFT_S_IFCHR 0020000
#define DRAFT_S_IFDIR 0040000
#define DRAFT_S_IFBLK 0060000
#define DRAFT_S_IFREG 0100000
#define DRAFT_S_IFLNK 0120000
#define DRAFT_S_IFSOCK 0140000
#define DRAFT_S_ISUID 04000
#define DRAFT_S_ISGID 02000
#define DRAFT_S_ISVTX 01000
#define DRAFT_S_IRWXU 0700
#define DRAFT_S_IRUSR 0400
#define DRAFT_S_IWUSR 0200
#define DRAFT_S_IXUSR 0100
#define DRAFT_S_IRWXG 0070
#define DRAFT_S_IRGRP 0040
#define DRAFT_S_IWGRP 0020
#define DRAFT_S_IXGRP 0010
#define DRAFT_S_IRWXO 0007
#define DRAFT_S_IROTH 0004
#define DRAFT_S_IWOTH 0002
#define DRAFT_S_IXOTH 0001
#define DRAFT_UTIME_NOW 0x3fffffff
#define DRAFT_UTIME_OMIT 0x3ffffffe

#define DRAFT_O_RDONLY 0x00000
#define DRAFT_O_WRONLY 0x00001
#define DRAFT_O_RDWR 0x00002
#define DRAFT_O_EXEC 0x00004
#define DRAFT_O_SEARCH 0x00008
#define DRAFT_O_ACCMODE 0x0000f
#define DRAFT_O_CREAT 0x00010
#define DRAFT_O_EXCL 0x00020
#define DRAFT_O_TRUNC 0x00040
#define DRAFT_O_APPEND 0x00080
#define DRAFT_O_NONBLOCK 0x00100
#define DRAFT_O_DIRECTORY 0x00200
#define DRAFT_O_NOFOLLOW 0x00400
#define DRAFT_O_CLOEXEC 0x00800
#define DRAFT_O_NOCTTY 0x01000
#define DRAFT_O_DSYNC 0x02000
#define DRAFT_O_RSYNC 0x04000
#define DRAFT_O_SYNC 0x08000
#define DRAFT_O_TTY_INIT 0x10000
#define DRAFT_FD_CLOEXEC 0x00001

#define DRAFT_F_DUPFD 0
#define DRAFT_F_GETFD 1
#define DRAFT_F_SETFD 2
#define DRAFT_F_GETFL 3
#define DRAFT_F_SETFL 4
#define DRAFT_F_GETLK 5
#define DRAFT_F_SETLK 6
#define DRAFT_F_SETLKW 7
#define DRAFT_F_DUPFD_CLOEXEC 8
#define DRAFT_F_GETOWN 9
#define DRAFT_F_SETOWN 10
#define DRAFT_F_RDLCK 0
#define DRAFT_F_WRLCK 1
#define DRAFT_F_UNLCK 2
#define DRAFT_SEEK_SET 0
#define DRAFT_SEEK_CUR 1
#define DRAFT_SEEK_END 2

#define DRAFT_AT_FDCWD (-100)
#define DRAFT_AT_EACCESS 0x01
#define DRAFT_AT_SYMLINK_NOFOLLOW 0x02
#define DRAFT_AT_SYMLINK_FOLLOW 0x04
#define DRAFT_AT_REMOVEDIR 0x08
#define DRAFT_F_OK 0
#define DRAFT_X_OK 1
#define DRAFT_W_OK 2
#define DRAFT_R_OK 4
#define DRAFT_POSIX_FADV_NORMAL 0
#define DRAFT_POSIX_FADV_RANDOM 1
#define DRAFT_POSIX_FADV_SEQUENTIAL 2
#define DRAFT_POSIX_FADV_WILLNEED 3
#define DRAFT_POSIX_FADV_DONTNEED 4
#define DRAFT_POSIX_FADV_NOREUSE 5

#define DRAFT_POLLIN 0x001
#define DRAFT_POLLPRI 0x002
#define DRAFT_POLLOUT 0x004
#define DRAFT_POLLERR 0x008
#define DRAFT_POLLHUP 0x010
#define DRAFT_POLLNVAL 0x020
#define DRAFT_POLLRDNORM 0x040
#define DRAFT_POLLRDBAND 0x080
#define DRAFT_POLLWRNORM 0x004
#define DRAFT_POLLWRBAND 0x100

#define DRAFT_WNOHANG 0x01
#define DRAFT_WUNTRACED 0x02
#define DRAFT_WSTOPPED 0x02
#define DRAFT_WCONTINUED 0x04
#define DRAFT_WEXITED 0x08
#define DRAFT_WNOWAIT 0x10
#define DRAFT_WIFEXITED(status) (((unsigned int)(status) & 0xffu) == 0)
#define DRAFT_WEXITSTATUS(status) (((unsigned int)(status) >> 8) & 0xffu)
#define DRAFT_WIFSTOPPED(status) (((unsigned int)(status) & 0xffu) == 0x7fu)
#define DRAFT_WSTOPSIG(status) DRAFT_WEXITSTATUS(status)
#define DRAFT_WIFCONTINUED(status) ((unsigned int)(status) == 0xffffu)
#define DRAFT_WIFSIGNALED(status) \
    ((((unsigned int)(status) & 0x7fu) != 0) && \
     (((unsigned int)(status) & 0x7fu) != 0x7fu))
#define DRAFT_WTERMSIG(status) ((unsigned int)(status) & 0x7fu)

DRAFT_FILE_ASSERT(sizeof(long) == 8);
DRAFT_FILE_ASSERT(sizeof(int) == 4);
DRAFT_FILE_ASSERT(sizeof(short) == 2);
DRAFT_FILE_ASSERT(sizeof(struct draft_timespec) == 16);
DRAFT_FILE_ASSERT(DRAFT_FILE_ALIGN(struct draft_timespec) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_timespec, tv_sec) == 0);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_timespec, tv_nsec) == 8);
DRAFT_FILE_ASSERT(sizeof(struct draft_stat) == 128);
DRAFT_FILE_ASSERT(DRAFT_FILE_ALIGN(struct draft_stat) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_dev) == 0);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_ino) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_nlink) == 16);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_mode) == 24);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_uid) == 28);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_gid) == 32);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, __reserved0) == 36);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_rdev) == 40);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_size) == 48);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_blksize) == 56);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_blocks) == 64);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_atim) == 72);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_mtim) == 88);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, st_ctim) == 104);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_stat, __reserved1) == 120);
DRAFT_FILE_ASSERT(sizeof(struct draft_flock) == 32);
DRAFT_FILE_ASSERT(DRAFT_FILE_ALIGN(struct draft_flock) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, l_type) == 0);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, l_whence) == 2);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, __reserved0) == 4);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, l_start) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, l_len) == 16);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, l_pid) == 24);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_flock, __reserved1) == 28);
DRAFT_FILE_ASSERT(sizeof(draft_nfds_t) == 8);
DRAFT_FILE_ASSERT((draft_nfds_t)-1 > 0);
DRAFT_FILE_ASSERT(sizeof(struct draft_pollfd) == 8);
DRAFT_FILE_ASSERT(DRAFT_FILE_ALIGN(struct draft_pollfd) == 4);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_pollfd, fd) == 0);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_pollfd, events) == 4);
DRAFT_FILE_ASSERT(DRAFT_FILE_OFFSET(struct draft_pollfd, revents) == 6);
DRAFT_FILE_ASSERT(sizeof(enum draft_idtype) == 4);
DRAFT_FILE_ASSERT(DRAFT_FILE_ALIGN(enum draft_idtype) == 4);
DRAFT_FILE_ASSERT((DRAFT_S_IRUSR | DRAFT_S_IWUSR | DRAFT_S_IXUSR) == DRAFT_S_IRWXU);
DRAFT_FILE_ASSERT((DRAFT_S_IRGRP | DRAFT_S_IWGRP | DRAFT_S_IXGRP) == DRAFT_S_IRWXG);
DRAFT_FILE_ASSERT((DRAFT_S_IROTH | DRAFT_S_IWOTH | DRAFT_S_IXOTH) == DRAFT_S_IRWXO);
DRAFT_FILE_ASSERT((DRAFT_S_IFREG & DRAFT_S_IFMT) == DRAFT_S_IFREG);
DRAFT_FILE_ASSERT((DRAFT_O_EXEC & DRAFT_O_ACCMODE) == DRAFT_O_EXEC);
DRAFT_FILE_ASSERT((DRAFT_O_SEARCH & DRAFT_O_ACCMODE) == DRAFT_O_SEARCH);
DRAFT_FILE_ASSERT((DRAFT_O_CREAT & DRAFT_O_ACCMODE) == 0);
DRAFT_FILE_ASSERT(DRAFT_POLLWRNORM == DRAFT_POLLOUT);
DRAFT_FILE_ASSERT(DRAFT_WSTOPPED == DRAFT_WUNTRACED);
DRAFT_FILE_ASSERT(DRAFT_WIFEXITED(0));
DRAFT_FILE_ASSERT(DRAFT_WIFEXITED(255 << 8));
DRAFT_FILE_ASSERT(DRAFT_WEXITSTATUS(255 << 8) == 255);
DRAFT_FILE_ASSERT(DRAFT_WIFSIGNALED(15));
DRAFT_FILE_ASSERT(DRAFT_WTERMSIG(15 | 0x80) == 15);
DRAFT_FILE_ASSERT(!DRAFT_WIFEXITED(15));
DRAFT_FILE_ASSERT(DRAFT_WIFSTOPPED((19 << 8) | 0x7f));
DRAFT_FILE_ASSERT(DRAFT_WSTOPSIG((19 << 8) | 0x7f) == 19);
DRAFT_FILE_ASSERT(!DRAFT_WIFSIGNALED((19 << 8) | 0x7f));
DRAFT_FILE_ASSERT(DRAFT_WIFCONTINUED(0xffff));
DRAFT_FILE_ASSERT(!DRAFT_WIFSTOPPED(0xffff));
DRAFT_FILE_ASSERT(!DRAFT_WIFSIGNALED(0xffff));

#undef DRAFT_FILE_OFFSET
#undef DRAFT_FILE_ALIGN
#undef DRAFT_FILE_ASSERT
#endif
