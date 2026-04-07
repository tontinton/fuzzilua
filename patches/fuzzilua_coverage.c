/* fuzzilua: edge coverage via __sanitizer_cov_trace_pc_guard
 *
 * Compile Redis with:
 *   -fsanitize-coverage=trace-pc-guard
 *
 * Environment variables:
 *   FUZZILUA_SHM_EDGE  - name of shared memory region for edge bitmap
 *
 * The compiler inserts calls to __sanitizer_cov_trace_pc_guard_init and
 * __sanitizer_cov_trace_pc_guard at every edge. We provide our own
 * implementations that write to shared memory instead of the sanitizer
 * runtime (which we don't link).
 */

#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>
#include <stdint.h>

#ifndef FUZZILUA_BITMAP_SIZE
#error "FUZZILUA_BITMAP_SIZE must be defined by the build system"
#endif

#define FUZZILUA_EDGE_BITMAP_SIZE FUZZILUA_BITMAP_SIZE

static uint8_t *fuzzilua_edge_bitmap = NULL;
static uint32_t fuzzilua_edge_bitmap_size = 0;
static uint32_t fuzzilua_next_guard_id = 1;

__attribute__((constructor))
static void fuzzilua_edge_init(void) {
    const char *shm_name = getenv("FUZZILUA_SHM_EDGE");
    if (!shm_name || shm_name[0] == '\0')
        return;

    int fd = shm_open(shm_name, O_RDWR, 0);
    if (fd < 0)
        return;

    size_t total_size = FUZZILUA_EDGE_BITMAP_SIZE * 2;
    uint8_t *base = (uint8_t *)mmap(NULL, total_size, PROT_READ | PROT_WRITE,
                                     MAP_SHARED, fd, 0);
    close(fd);

    if (base == MAP_FAILED)
        return;

    fuzzilua_edge_bitmap = base;
    fuzzilua_edge_bitmap_size = FUZZILUA_EDGE_BITMAP_SIZE;
}

/* Called once per DSO during startup. Assigns guard IDs. */
void __sanitizer_cov_trace_pc_guard_init(uint32_t *start, uint32_t *stop) {
    if (start == stop || *start)
        return;

    for (uint32_t *x = start; x < stop; x++) {
        *x = fuzzilua_next_guard_id++;
    }
}

/* Called at every edge. Increments the corresponding bitmap byte. */
void __sanitizer_cov_trace_pc_guard(uint32_t *guard) {
    uint32_t id = *guard;
    if (id == 0)
        return;

    if (!fuzzilua_edge_bitmap)
        return;

    uint32_t idx = id % fuzzilua_edge_bitmap_size;
    uint8_t old = fuzzilua_edge_bitmap[idx];
    if (old < 255)
        fuzzilua_edge_bitmap[idx] = old + 1;
}
