/*
 * checksum - decode a raw TrueHD file with the SiloObjectAudio C API and print a digest of
 * everything the decoder produced (PCM as 24-bit integers, speakers, metadata updates).
 * Plain C with no dependencies, so the same source runs on every slice (for example in a
 * tvOS simulator via `simctl spawn`) and the digests can be compared across platforms.
 *
 * usage: checksum <file.thd> [presentation]
 */
#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "truehd_atmos.h"

static uint64_t fnv = 0xcbf29ce484222325ull;
static void mix(const void *p, size_t n) {
    const uint8_t *b = p;
    for (size_t i = 0; i < n; i++) fnv = (fnv ^ b[i]) * 0x100000001b3ull;
}

int main(int argc, char **argv) {
    if (argc < 2) return 2;
    FILE *f = fopen(argv[1], "rb");
    if (!f) return 2;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *data = malloc((size_t)size);
    if (fread(data, 1, (size_t)size, f) != (size_t)size) return 2;
    fclose(f);

    int32_t presentation = argc > 2 ? atoi(argv[2]) : TRUEHD_ATMOS_PRESENTATION_HIGHEST;
    TrueHDAtmosDecoder *d = truehd_atmos_decoder_create(presentation);
    uint64_t blocks = 0, frames = 0, updates = 0;
    TrueHDAtmosBlock b;
    for (long off = 0; off < size; off += 4096) {
        size_t n = (size_t)(size - off < 4096 ? size - off : 4096);
        truehd_atmos_decoder_push(d, data + off, n, off);
        while (truehd_atmos_decoder_pull(d, &b) == TRUEHD_ATMOS_OK) {
            for (uint32_t c = 0; c < b.channel_count; c++)
                for (uint32_t i = 0; i < b.frame_count; i++) {
                    int32_t s = (int32_t)lrintf(b.channels[c][i] * 8388608.0f);
                    mix(&s, sizeof s);
                }
            mix(b.speakers, b.channel_count);
            if (b.metadata) {
                for (uint32_t u = 0; u < b.metadata->update_count; u++) {
                    const TrueHDAtmosMetadataUpdate *up = &b.metadata->updates[u];
                    uint64_t t = b.sample_position + up->frame_offset;
                    mix(&t, sizeof t);
                    mix(up->elements, sizeof(TrueHDAtmosElement) * b.metadata->element_count);
                    updates++;
                }
            }
            blocks++;
            frames += b.frame_count;
        }
    }
    printf("%s: %" PRIu64 " blocks, %" PRIu64 " frames, p%u %u ch @ %u Hz, %" PRIu64
           " metadata updates, digest %016" PRIx64 ", selftest %d\n",
           strrchr(argv[1], '/') ? strrchr(argv[1], '/') + 1 : argv[1], blocks, frames,
           b.presentation, b.channel_count, b.sample_rate, updates, fnv, truehd_atmos_selftest());
    truehd_atmos_decoder_destroy(d);
    free(data);
    return blocks ? 0 : 1;
}
