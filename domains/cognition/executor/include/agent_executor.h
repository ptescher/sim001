// C ABI of cognition_executor (domains/cognition/executor/src/lib.rs). The structs are
// domains/cognition/plan/src/lib.rs's #[repr(C)] types: keep them in step, and bump
// AGENT_EXEC_ABI_VERSION on any change. Local NED metres from home, angles in radians unless _cd.
//
// How to drive it:
//   - agent_exec_reset() on entering AGENT mode, then agent_exec_step() once per control loop
//     (it is built for about 50 Hz; any rate with 0 < dt <= 0.5 s works). One thread only: the
//     executor is a single global, not reentrant.
//   - Pass the current plan on every step (or null for none). A plan whose id differs from the
//     one held is adopted, and its ttl counts from that step; the same id again is ignored, so the
//     ground station must give each new plan a new id. When ttl + limits.grace seconds pass with
//     no new id, the plane loiters where it was (fallback 1). A plan with an unknown primitive or
//     a NaN/inf field is refused (the held one stays).
//   - The executor keeps its own time (the sum of dt); state.time is only checked to be finite.
//   - The demand goes to the stock controllers: roll_cd and pitch_cd as nav_roll_cd and
//     nav_pitch_cd, throttle (0..1) as the throttle demand. A negative throttle (only with
//     fallback 2) means "leave it to the pilot / the mode's own throttle".
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AGENT_EXEC_ABI_VERSION 2
#define AGENT_MAX_TRACKS 8

struct agent_plan {
    uint16_t id;         // a new id is a new plan; the same id again is ignored
    uint8_t primitive;   // 0 none, 1 pursue, 2 evade, 3 goto, 4 orbit, 5 band, 6 loiter
    uint8_t target;      // MAVLink system id of the track (pursue, evade)
    float point[3];      // north, east, down (m): goto's point, orbit's and band's centre
    float radius;        // orbit radius (m); widened to what the plane can fly, and circles are
                         // moved (and shrunk) to fit inside the fence
    float band[2];       // height above home, low and high (m)
    float ttl;           // seconds it is good for, from adoption
};

struct agent_state {
    float pos[3];        // EKF position, NED from home (m)
    float vel[3];        // EKF velocity over the ground, NED (m/s)
    float roll, pitch, yaw;  // attitude (rad); yaw is the heading, 0 north, positive clockwise
    float airspeed;      // airspeed (m/s), the AHRS estimate; also used with yaw to estimate the wind
    float agl;           // height above the ground (m); the floor is kept against this
    float dt;            // seconds since the last step: must be in (0, 0.5]
    float time;          // seconds since AGENT mode was entered (not used, but must be finite)
};

struct agent_track {
    uint8_t id;          // MAVLink system id
    uint8_t valid;       // nonzero: this entry is in use
    // (2 bytes of padding)
    float pos[3];        // NED from the same home (m), as last heard
    float vel[3];        // NED (m/s)
    float age;           // seconds since heard: dead-reckoned by it, ignored over 3 s
};

struct agent_limits {
    float fence_centre[2];  // north, east (m)
    float fence_radius;  // m; 0: no fence
    float ceiling;       // height above home (m)
    float floor;         // height above the ground (m), against state.agl
    float bank_max;      // rad
    float pitch_max;     // rad, nose up
    float pitch_min;     // rad, nose down: its size is used, the sign may be either
    float airspeed_min;  // m/s: under it, stall recovery (fallback 3)
    float airspeed_cruise;
    float airspeed_max;
    float separation;    // m, horizontal and vertical, from every valid track; <= 0: off
    float grace;         // seconds a plan stays good past its ttl
};

struct agent_demand {
    int32_t roll_cd;     // positive: right wing down
    int32_t pitch_cd;    // positive: nose up
    float throttle;      // 0..1, negative: leave it to the pilot
    uint8_t fallback;    // 0 none; 1 plan expired, or its target (pursue, evade) lost: loitering;
                         // 2 bad input (a NaN, an inf or an absurd value): wings level, nose level,
                         // throttle negative; 3 recovery (under airspeed_min, or below the floor)
    uint8_t intervened;  // 1: the safety envelope changed the demand on this step
    uint8_t pad[2];
};

uint32_t agent_exec_abi_version(void);

// Runs one control step. `plan` may be null (no plan: keep the held one, or with none ever, fly
// wings level holding height). `tracks` is an array of AGENT_MAX_TRACKS entries (unused ones have
// valid = 0), or null for none. Returns 0, or -1 if `state`, `limits` or `out` is null. The output
// is always finite and within the limits.
int32_t agent_exec_step(const struct agent_state *state, const struct agent_plan *plan,
                        const struct agent_track *tracks, const struct agent_limits *limits,
                        struct agent_demand *out);

// Forget the plan and all memory (on entering AGENT mode).
void agent_exec_reset(void);

#ifdef __cplusplus
}
#endif
