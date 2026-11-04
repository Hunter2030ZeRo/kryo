#include <errno.h>
#include <fenv.h>
#include <stdio.h>
#include <stdlib.h>
#include <cryolite/native.h>

struct block {
    struct block *next;
    unsigned char bytes[4096 - sizeof(struct block *)];
};

static void require(int condition, kryo_handle debug, unsigned stage) {
    if (!condition) {
        (void)kryo_debug_log(debug, 211, stage);
        kryo_exit(stage);
    }
}

static void progress(kryo_handle debug, unsigned step) {
    require(kryo_debug_log(debug, 212, step).status == KRYO_STATUS_OK, debug, 609);
}

static kryo_u64 commit_pressure(kryo_handle debug) {
    const kryo_u64 bytes = 8192 * 4096;
    kryo_result reserved = kryo_vm_reserve(bytes, 4096);
    require(reserved.status == KRYO_STATUS_OK && reserved.value != 0, debug, 610);
    kryo_u64 committed = 0;
    unsigned failures = 0;
    for (unsigned pages = 8192; pages; pages >>= 1) {
        kryo_u64 chunk = (kryo_u64)pages * 4096;
        if (chunk > bytes - committed)
            continue;
        kryo_result result = kryo_vm_commit(reserved.value + committed, chunk);
        if (result.status == KRYO_STATUS_OK)
            committed += chunk;
        else {
            require(result.status == KRYO_STATUS_OUT_OF_RESOURCES, debug, 611);
            ++failures;
        }
        progress(debug, 2000 + pages);
    }
    require(committed > 0 && committed < bytes && failures > 0, debug, 612);
    return reserved.value;
}

void check_provider_stdio(kryo_handle debug) {
    int saved_errno = errno;
    fenv_t environment;
    require(fegetenv(&environment) == 0, debug, 600);
    require(setvbuf(stdout, NULL, _IOFBF, BUFSIZ) == 0
            && setvbuf(stdin, NULL, _IOFBF, BUFSIZ) == 0, debug, 601);
    require(!ferror(stdout) && !ferror(stdin), debug, 602);
    progress(debug, 1);

    struct block *blocks = malloc(sizeof(*blocks));
    require(blocks != NULL, debug, 613);
    blocks->next = NULL;
    progress(debug, 1001);
    // Fill the bounded account in large native commits, then drain only existing
    // allocator slack instead of growing thousands of separate heap arenas.
    kryo_u64 pressure = commit_pressure(debug);
    unsigned count = 1;
    for (; count < 1024; ++count) {
        struct block *block = malloc(sizeof(*block));
        if (!block)
            break;
        block->next = blocks;
        blocks = block;
        unsigned live = count + 1;
        if ((live & (live - 1)) == 0)
            progress(debug, 1000 + live);
    }
    require(count < 1024 && errno == ENOMEM, debug, 603);
    progress(debug, 2);

    // Both streams still need their first 4096-byte buffer plus pushback space.
    errno = 0;
    require(fputc('X', stdout) == EOF && errno == ENOMEM && ferror(stdout), debug, 604);
    progress(debug, 3);
    errno = 0;
    require(ungetc('Q', stdin) == EOF && errno == ENOMEM && ferror(stdin), debug, 605);
    progress(debug, 4);
    while (blocks) {
        struct block *next = blocks->next;
        free(blocks);
        blocks = next;
    }
    require(kryo_vm_release(pressure).status == KRYO_STATUS_OK, debug, 614);
    progress(debug, 5);

    clearerr(stdout);
    clearerr(stdin);
    errno = 0;
    require(ungetc('Q', stdin) == 'Q' && fgetc(stdin) == 'Q'
            && !ferror(stdin) && !feof(stdin), debug, 606);
    progress(debug, 6);
    // Pushback recovery uses no descriptor I/O and leaves stdout without dirty data.
    require(!ferror(stdout) && fesetenv(&environment) == 0, debug, 607);
    require(kryo_debug_log(debug, 210, 1).status == KRYO_STATUS_OK, debug, 608);
    errno = saved_errno;
}
