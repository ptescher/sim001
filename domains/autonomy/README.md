# Autonomy

AI control of planes, from the sim to the real thing: see [plans/sim2real-tag.md](../../plans/sim2real-tag.md).

| Crate | Path | Role |
| --- | --- | --- |
| `autonomy_executor` | [executor/](executor) | What flies a plan on board: `no_std`, with a C ABI, linked into the ArduPlane fork's AGENT mode and used by the training environment, so both run the same code |
