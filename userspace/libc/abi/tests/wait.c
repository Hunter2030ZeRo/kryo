#include <cryolite/file.h>

/* Host macro semantics only: this program never calls the native SDK. */
int main(void) {
    for (unsigned int value = 0; value <= 65535; ++value) {
        unsigned int low = value & 127u;
        if (WIFSIGNALED(value) != (low != 0 && low != 127))
            return 1;
        if (WIFEXITED(value) != ((value & 255u) == 0))
            return 2;
        if (WIFSTOPPED(value) != ((value & 255u) == 127))
            return 3;
        if (WIFCONTINUED(value) != (value == 65535))
            return 4;
    }
    int count = 0;
    if (!WIFSIGNALED((++count, 15)) || count != 1)
        return 5;
    count = 0;
    if (!WIFEXITED((++count, 42 << 8)) || count != 1)
        return 6;
    count = 0;
    if (WSTOPSIG((++count, (19 << 8) | 127)) != 19 || count != 1)
        return 7;
    count = 0;
    if (!S_ISREG((++count, S_IFREG | 0644)) || count != 1)
        return 8;
    return 0;
}
