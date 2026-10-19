// C ABI of autonomy_executor (domains/autonomy/executor/src/lib.rs). The structs are
// domains/autonomy/plan/src/lib.rs's #[repr(C)] types: keep them in step, and bump
// AGENT_EXEC_ABI_VERSION on any change. Local NED metres from home, angles in radians unless _cd.
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AGENT_EXEC_ABI_VERSION 2
#define AGENT_MAX_TRACKS 8

struct agent_plan {
    uint16_t id;
    uint8_t primitive;   // 0 none, 1 pursue, 2 evade, 3 goto, 4 orbit, 5 band, 6 loiter
    uint8_t target;      // MAVLink system id of the track
    float point[3];      // north, east, down (m)
    float radius;        // orbit radius (m)
    float band[2];       // height above home, low and high (m)
    float ttl;           // seconds it is good for, from receipt
};

struct agent_state {
    float pos[3];
    float vel[3];
    float roll, pitch, yaw;
    float airspeed;
    float agl;
    float dt;            // seconds since the last step
    float time;          // seconds since AGENT mode was entered
};

struct agent_track {
    uint8_t id;
    uint8_t valid;
    // (2 bytes of padding)
    float pos[3];
    float vel[3];
    float age;
};

struct agent_limits {
    float fence_centre[2];
    float fence_radius;  // 0: no fence
    float ceiling;
    float floor;
    float bank_max;
    float pitch_max;
    float pitch_min;
    float airspeed_min;
    float airspeed_cruise;
    float airspeed_max;
    float separation;
    float grace;
};

struct agent_demand {
    int32_t roll_cd;
    int32_t pitch_cd;
    float throttle;      // 0..1, negative: leave it to the pilot
    uint8_t fallback;    // 0 none, 1 plan expired, 2 bad input, 3 recovery
    uint8_t intervened;  // the envelope changed the demand
    uint8_t pad[2];
};

uint32_t agent_exec_abi_version(void);

// Runs one control step. `plan` may be null (no plan yet). `tracks` is an array of
// AGENT_MAX_TRACKS entries (unused ones have valid = 0). Returns 0, or -1 on a null pointer.
// A plan whose id is new is adopted with its ttl counted from this step; the executor keeps it.
int32_t agent_exec_step(const struct agent_state *state, const struct agent_plan *plan,
                        const struct agent_track *tracks, const struct agent_limits *limits,
                        struct agent_demand *out);

// Forget the plan and all memory (on entering AGENT mode).
void agent_exec_reset(void);

#ifdef __cplusplus
}
#endif
