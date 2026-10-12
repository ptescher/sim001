# Domains

The repo is one Cargo workspace, organised as **workspace → domain → module**:

```
Cargo.toml                    the workspace (and, for now, the sim package)
domains/
  <domain>/                   a coherent area of the product, with its own README
    <module>/                 one crate: expectation_<module>, depending only "downwards"
```

A domain is a directory. Each module in it is a crate named `<domain>_<module>`. The
workspace picks them up through the `domains/*/*` glob, so a new module needs no edit to the
root `Cargo.toml`. A module depends on the modules beneath it in its own domain, and on
other domains only through their public crates. Keeping every module a separate crate makes
the compiler enforce that.

| Domain | What it is |
| --- | --- |
| [design](design/README.md) | The look shared by the ground station, the sim's HUD and the docs: palettes, type and corners. |
| [simulation](simulation/README.md) | The flight model and the planes it flies: aircraft descriptions and the physics, headless and independent of any renderer. |
| [expectation](expectation/README.md) | Behaviour written as expectations, checked for conflicts, run as acceptance tests from user stories, and turned into documentation. |

The sim (`src/`), `station/`, `gcs/` and `tools/mapbuild` predate the layout and will move
into domains as the work goes on. `gcs/` and `tools/mapbuild` are excluded from the
workspace for now because they keep their own lockfiles.
