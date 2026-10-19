// C ABI of autonomy_executor (domains/autonomy/executor/src/lib.rs). Keep in step with ABI_VERSION.
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AGENT_EXEC_ABI_VERSION 1

struct agent_demand {
    int32_t roll_cd;   // positive right wing down
    int32_t pitch_cd;  // positive nose up
    float throttle;    // 0..1, negative: leave it to the pilot
};

uint32_t agent_exec_abi_version(void);
int32_t agent_exec_step(struct agent_demand *out);

#ifdef __cplusplus
}
#endif
