// The measurement page's calls, beside the generic host (nana_ios_host.h).
#pragma once

#include "../../include/nana_ios_host.h"

#ifdef __cplusplus
extern "C" {
#endif

// Where the preview area is, logical px: the shell lays its camera layer
// out to it.
bool nana_probe_preview_rect(NanaIosHost *host, NanaRect *rect);
// Whether the page asks for the CPU-uploaded preview instead.
bool nana_probe_cpu_preview(NanaIosHost *host);
void nana_probe_push_preview_bgra(NanaIosHost *host, const uint8_t *bytes, uint32_t width,
                                  uint32_t height, uint32_t bytes_per_row);
uint32_t nana_probe_presses(NanaIosHost *host);

#ifdef __cplusplus
}
#endif
