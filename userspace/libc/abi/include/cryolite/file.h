#ifndef CRYOLITE_C_ABI_FILE_H
#define CRYOLITE_C_ABI_FILE_H

#include <cryolite/time.h>

struct stat {
    dev_t st_dev;
    ino_t st_ino;
    nlink_t st_nlink;
    mode_t st_mode;
    uid_t st_uid;
    gid_t st_gid;
    unsigned int __reserved0;
    dev_t st_rdev;
    off_t st_size;
    blksize_t st_blksize;
    blkcnt_t st_blocks;
    struct timespec st_atim;
    struct timespec st_mtim;
    struct timespec st_ctim;
    unsigned long __reserved1;
};

#define st_atime st_atim.tv_sec
#define st_mtime st_mtim.tv_sec
#define st_ctime st_ctim.tv_sec

struct flock {
    short l_type;
    short l_whence;
    unsigned int __reserved0;
    off_t l_start;
    off_t l_len;
    pid_t l_pid;
    unsigned int __reserved1;
};

struct pollfd {
    int fd;
    short events;
    short revents;
};

typedef enum {
    P_ALL = 0,
    P_PID = 1,
    P_PGID = 2
} idtype_t;

#define S_IFMT 0170000
#define S_IFIFO 0010000
#define S_IFCHR 0020000
#define S_IFDIR 0040000
#define S_IFBLK 0060000
#define S_IFREG 0100000
#define S_IFLNK 0120000
#define S_IFSOCK 0140000
#define S_ISUID 04000
#define S_ISGID 02000
#define S_ISVTX 01000
#define S_IRWXU 0700
#define S_IRUSR 0400
#define S_IWUSR 0200
#define S_IXUSR 0100
#define S_IRWXG 0070
#define S_IRGRP 0040
#define S_IWGRP 0020
#define S_IXGRP 0010
#define S_IRWXO 0007
#define S_IROTH 0004
#define S_IWOTH 0002
#define S_IXOTH 0001
#define UTIME_NOW 0x3fffffff
#define UTIME_OMIT 0x3ffffffe

#define S_ISFIFO(mode) (((mode) & S_IFMT) == S_IFIFO)
#define S_ISCHR(mode) (((mode) & S_IFMT) == S_IFCHR)
#define S_ISDIR(mode) (((mode) & S_IFMT) == S_IFDIR)
#define S_ISBLK(mode) (((mode) & S_IFMT) == S_IFBLK)
#define S_ISREG(mode) (((mode) & S_IFMT) == S_IFREG)
#define S_ISLNK(mode) (((mode) & S_IFMT) == S_IFLNK)
#define S_ISSOCK(mode) (((mode) & S_IFMT) == S_IFSOCK)

#define O_RDONLY 0x00000
#define O_WRONLY 0x00001
#define O_RDWR 0x00002
#define O_EXEC 0x00004
#define O_SEARCH 0x00008
#define O_ACCMODE 0x0000f
#define O_CREAT 0x00010
#define O_EXCL 0x00020
#define O_TRUNC 0x00040
#define O_APPEND 0x00080
#define O_NONBLOCK 0x00100
#define O_DIRECTORY 0x00200
#define O_NOFOLLOW 0x00400
#define O_CLOEXEC 0x00800
#define O_NOCTTY 0x01000
#define O_DSYNC 0x02000
#define O_RSYNC 0x04000
#define O_SYNC 0x08000
#define O_TTY_INIT 0x10000
#define FD_CLOEXEC 0x00001

#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_GETLK 5
#define F_SETLK 6
#define F_SETLKW 7
#define F_DUPFD_CLOEXEC 8
#define F_GETOWN 9
#define F_SETOWN 10
#define F_RDLCK 0
#define F_WRLCK 1
#define F_UNLCK 2
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define AT_FDCWD (-100)
#define AT_EACCESS 0x01
#define AT_SYMLINK_NOFOLLOW 0x02
#define AT_SYMLINK_FOLLOW 0x04
#define AT_REMOVEDIR 0x08
#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4
#define POSIX_FADV_NORMAL 0
#define POSIX_FADV_RANDOM 1
#define POSIX_FADV_SEQUENTIAL 2
#define POSIX_FADV_WILLNEED 3
#define POSIX_FADV_DONTNEED 4
#define POSIX_FADV_NOREUSE 5

#define POLLIN 0x001
#define POLLPRI 0x002
#define POLLOUT 0x004
#define POLLERR 0x008
#define POLLHUP 0x010
#define POLLNVAL 0x020
#define POLLRDNORM 0x040
#define POLLRDBAND 0x080
#define POLLWRNORM 0x004
#define POLLWRBAND 0x100

#define WNOHANG 0x01
#define WUNTRACED 0x02
#define WSTOPPED 0x02
#define WCONTINUED 0x04
#define WEXITED 0x08
#define WNOWAIT 0x10
#define WIFEXITED(status) (((unsigned int)(status) & 0xffu) == 0)
#define WEXITSTATUS(status) (((unsigned int)(status) >> 8) & 0xffu)
#define WIFSTOPPED(status) (((unsigned int)(status) & 0xffu) == 0x7fu)
#define WSTOPSIG(status) WEXITSTATUS(status)
#define WIFCONTINUED(status) ((unsigned int)(status) == 0xffffu)
#define WIFSIGNALED(status) ((((unsigned int)(status) & 0x7fu) - 1u) < 0x7eu)
#define WTERMSIG(status) ((unsigned int)(status) & 0x7fu)

#endif
