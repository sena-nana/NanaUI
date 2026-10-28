// C ABI of the experimental NanaUI iOS host (platform/ios).
//
// Every call is made on the main thread. Geometry is in physical px unless
// a name says logical; text offsets are UTF-8 bytes. The app's staticlib
// provides nana_ios_host_new (export_ios_app!) and its own calls in its own
// header.
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NanaIosHost NanaIosHost;

typedef struct {
    float top, right, bottom, left;
} NanaSafeArea;

typedef struct {
    float x, y, width, height;
} NanaRect;

// Callbacks must not call back into the host synchronously: they run
// inside input routing. Dispatch onto the main queue instead.
typedef struct {
    // Opaque pointer handed back to every callback.
    void *context;
    // A view task is ready; may run on any thread. Request a frame on main.
    void (*wake)(void *context);
    // The focused editor changed: active, and its caret in logical px.
    void (*text_input)(void *context, bool active, NanaRect caret);
    // Play one NANA_HAPTIC_* feedback.
    void (*haptic)(void *context, int32_t style);
} NanaIosCallbacks;

enum {
    NANA_HAPTIC_LIGHT = 0,
    NANA_HAPTIC_MEDIUM = 1,
    NANA_HAPTIC_SELECTION = 2,
    NANA_HAPTIC_SUCCESS = 3,
    NANA_HAPTIC_WARNING = 4,
};

enum {
    NANA_TOUCH_DOWN = 0,
    NANA_TOUCH_MOVE = 1,
    NANA_TOUCH_UP = 2,
    NANA_TOUCH_CANCEL = 3,
};

enum {
    NANA_FRAME_IDLE = 0,
    NANA_FRAME_NOW = 1,
    NANA_FRAME_AT = 2,
    NANA_FRAME_ERROR = -1,
};

// `layer` is a CAMetalLayer the caller keeps alive until detach or free.
// `transparent` composites with premultiplied alpha over a native layer.
NanaIosHost *nana_ios_host_new(void *layer, uint32_t width, uint32_t height, float scale,
                               NanaSafeArea safe_area, bool dark, float text_scale,
                               bool transparent, NanaIosCallbacks callbacks);
void nana_ios_host_free(NanaIosHost *host);

void nana_ios_host_resize(NanaIosHost *host, uint32_t width, uint32_t height, float scale,
                          NanaSafeArea safe_area);
void nana_ios_host_set_appearance(NanaIosHost *host, bool dark, float text_scale);
// The page background for the current appearance (sRGB RGBA, 0..1). A
// transparent host paints none: the shell paints it under its native layers.
bool nana_ios_host_background(NanaIosHost *host, float rgba[4]);
bool nana_ios_host_attach(NanaIosHost *host, void *layer, uint32_t width, uint32_t height);
void nana_ios_host_detach(NanaIosHost *host);

bool nana_ios_host_touch(NanaIosHost *host, int32_t phase, float x, float y, uint64_t id);

// One display-link tick: lay out and present if anything changed. Returns
// the demand after it; for NANA_FRAME_AT, `*seconds` is the delay.
int32_t nana_ios_host_tick(NanaIosHost *host, double *seconds);
int32_t nana_ios_host_frame_demand(NanaIosHost *host, double *seconds);

// Native text proxy (UITextInput).
void nana_ios_host_set_marked_text(NanaIosHost *host, const char *text, size_t len,
                                   int64_t selection_start, int64_t selection_end);
void nana_ios_host_commit_text(NanaIosHost *host, const char *text, size_t len);
void nana_ios_host_delete_surrounding(NanaIosHost *host, size_t before, size_t after);
void nana_ios_host_set_selection(NanaIosHost *host, size_t anchor, size_t focus);
// Copies the focused editor's text into `buffer` (when it fits) and returns
// its length, or -1 with no editor. Offsets are -1 when absent.
int64_t nana_ios_host_text_snapshot(NanaIosHost *host, char *buffer, size_t capacity,
                                    int64_t *selection_start, int64_t *selection_end,
                                    int64_t *marked_start, int64_t *marked_end);

// Text geometry, logical px: the focused editor's box, the offset a point
// lands on (-1 without an editor), and the caret box before an offset.
bool nana_ios_host_text_bounds(NanaIosHost *host, NanaRect *rect);
int64_t nana_ios_host_text_offset_at(NanaIosHost *host, float x, float y);
bool nana_ios_host_text_caret_rect(NanaIosHost *host, size_t offset, NanaRect *rect);

// Accessibility: a flat snapshot of the nodes VoiceOver stops on, in
// document order. _count takes the snapshot; _node and _text read it by
// index. Frames are logical px of the surface.
enum {
    NANA_AX_ROLE_GENERIC = 0,
    NANA_AX_ROLE_TEXT = 1,
    NANA_AX_ROLE_BUTTON = 2,
    NANA_AX_ROLE_TEXT_INPUT = 3,
    NANA_AX_ROLE_TOGGLE = 4,
    NANA_AX_ROLE_SLIDER = 5,
    NANA_AX_ROLE_IMAGE = 6,
    NANA_AX_ROLE_TAB = 7,
    NANA_AX_ROLE_ROW = 8,
    NANA_AX_ROLE_PROGRESS = 9,
};

enum {
    NANA_AX_DISABLED = 1 << 0,
    NANA_AX_CHECKED = 1 << 1,
    NANA_AX_SELECTED = 1 << 2,
    NANA_AX_FOCUSED = 1 << 3,
    NANA_AX_EDITABLE = 1 << 4,
    NANA_AX_BUSY = 1 << 5,
};

enum {
    NANA_AX_TEXT_LABEL = 0,
    NANA_AX_TEXT_VALUE = 1,
    NANA_AX_TEXT_HINT = 2,
};

enum {
    NANA_AX_ACTION_ACTIVATE = 0,
    NANA_AX_ACTION_INCREMENT = 1,
    NANA_AX_ACTION_DECREMENT = 2,
    NANA_AX_ACTION_FOCUS = 3,
};

typedef struct {
    uint64_t id;
    int32_t role;
    uint32_t flags;
    NanaRect frame;
    // A slider's value; NaN otherwise.
    double numeric_value;
} NanaAccessibilityNode;

size_t nana_ios_host_accessibility_count(NanaIosHost *host);
bool nana_ios_host_accessibility_node(NanaIosHost *host, size_t index,
                                      NanaAccessibilityNode *node);
// Copies one NANA_AX_TEXT_* into `buffer` when it fits; its length, or -1.
int64_t nana_ios_host_accessibility_text(NanaIosHost *host, size_t index, int32_t which,
                                         char *buffer, size_t capacity);
bool nana_ios_host_accessibility_action(NanaIosHost *host, uint64_t id, int32_t action);

#ifdef __cplusplus
}
#endif
