// The C interface of the Rust station (domains/teleoperation/ios/src/ffi.rs).
#ifndef T1S_H
#define T1S_H
#include <stdint.h>

// An opaque handle to the station.
typedef const void *T1SStation;

// Start the station on a rust-mavlink address. A control_port above 0 starts the control server.
T1SStation t1s_start(const char *address, uint16_t control_port);
void t1s_press(T1SStation station, const char *id);
// The state called key (caller frees with t1s_string_free), or NULL.
char *t1s_observe(T1SStation station, const char *key);
void t1s_string_free(char *s);
#endif
